use anyhow::{Context, Result, bail};
use bevy::camera::visibility::{VisibilityClass, add_visibility_class};
use bevy::{
    asset::{Asset, AssetLoader, Handle, LoadContext, RenderAssetUsages, io::Reader},
    color::Color,
    ecs::component::Component,
    image::{Image, ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor},
    math::{Mat3, Mat4, Vec3},
    mesh::{Indices, Mesh, PrimitiveTopology},
    platform::collections::HashMap,
    prelude::{Transform, Visibility},
    reflect::TypePath,
    render::{
        extract_component::ExtractComponent,
        render_resource::{Extent3d, TextureDimension, TextureFormat},
    },
};
use vatf::{
    animation::{AnimFilter, AnimTransform},
    reader::VabReader,
};

use crate::material::{BitmapMaterial, GradientMaterial, GradientUniformsVab};

/// Root sprite id — always 0 in VAB.
pub const ROOT_SPRITE_ID: u16 = 0;

/// Frame rate assumed when a VAB carries no usable frame rate.
pub const DEFAULT_FRAME_RATE: f32 = 30.0;

// ===========================================================================
// Render command types — mirroring swf_player's CommandList architecture
// ===========================================================================

/// ShapeHandle in VAB = index into `VabAsset::render_meshes`.
/// Unlike swf_player's `Arc<dyn ShapeHandleImpl>`, this is a concrete index
/// resolving to pre-built Bevy meshes + materials.
pub type ShapeHandle = usize;

/// Blend modes — subset matching SWF's common modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VabBlendMode {
    Normal,
    Layer,
    Multiply,
    Screen,
    Lighten,
    Darken,
    Difference,
    Add,
    Subtract,
    Invert,
    Alpha,
    Erase,
    Overlay,
    HardLight,
}

impl From<u8> for VabBlendMode {
    fn from(value: u8) -> Self {
        match value {
            2 => Self::Layer,
            3 => Self::Multiply,
            4 => Self::Screen,
            5 => Self::Lighten,
            6 => Self::Darken,
            7 => Self::Difference,
            8 => Self::Add,
            9 => Self::Subtract,
            10 => Self::Invert,
            11 => Self::Alpha,
            12 => Self::Erase,
            13 => Self::Overlay,
            14 => Self::HardLight,
            _ => Self::Normal,
        }
    }
}

/// Render command — same semantic structure as swf_player's `Command`.
#[derive(Debug, Clone)]
pub enum VabCommand {
    RenderShape {
        handle: ShapeHandle,
        transform: AnimTransform,
    },
    /// Render `commands` into an offscreen texture, apply `filters` to it, then
    /// composite the result at `bounds` (`[offset_x, offset_y, width, height]`
    /// in the parent's local pixel space).
    ///
    /// The offscreen target itself is the renderer's responsibility — it should
    /// be pooled (see `render::texture_cache::FrameInternalTextureCache`)
    /// rather than allocated per command.
    ApplyFilter {
        commands: CommandList,
        filters: Vec<AnimFilter>,
        bounds: [f32; 4],
    },
    /// Render sub-commands to an offscreen texture, then composite with blend mode.
    Blend(CommandList, VabBlendMode),
    PushMask,
    ActivateMask,
    DeactivateMask,
    PopMask,
}

#[derive(Debug, Default, Clone)]
pub struct CommandList {
    pub commands: Vec<VabCommand>,
}

impl CommandList {
    pub fn render_shape(&mut self, handle: ShapeHandle, transform: AnimTransform) {
        self.commands
            .push(VabCommand::RenderShape { handle, transform });
    }

    pub fn apply_filter(&mut self, other: CommandList, filters: Vec<AnimFilter>, bounds: [f32; 4]) {
        self.commands.push(VabCommand::ApplyFilter {
            commands: other,
            filters,
            bounds,
        });
    }

    pub fn blend(&mut self, other: CommandList, mode: VabBlendMode) {
        self.commands.push(VabCommand::Blend(other, mode));
    }

    pub fn push_mask(&mut self) {
        self.commands.push(VabCommand::PushMask);
    }

    pub fn activate_mask(&mut self) {
        self.commands.push(VabCommand::ActivateMask);
    }

    pub fn deactivate_mask(&mut self) {
        self.commands.push(VabCommand::DeactivateMask);
    }

    pub fn pop_mask(&mut self) {
        self.commands.push(VabCommand::PopMask);
    }

    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }
}

// ===========================================================================
// VAB asset
// ===========================================================================

/// A fully-loaded VAB asset with pre-built Bevy meshes and materials.
#[derive(Asset, TypePath)]
pub struct VabAsset {
    /// Offline-evaluated frames. This is the only animation data a `.vab`
    /// carries; frame rate lives in [`vatf::baked::BakedMovie::frame_rate`].
    pub baked: vatf::baked::BakedMovie,
    /// Shape id → ShapeHandle(s).
    pub shape_map: HashMap<u16, Vec<ShapeHandle>>,
    /// (morph_id, ratio) → ShapeHandle(s).
    pub morph_map: HashMap<(u16, u16), Vec<ShapeHandle>>,
    /// Pre-built renderable meshes (one per VAB ShapeMesh).
    pub render_meshes: Vec<RenderMeshGroup>,
}

/// A pre-built mesh + material pair for one draw call.
#[derive(Debug, Clone)]
pub struct RenderMeshGroup {
    pub mesh: Handle<Mesh>,
    pub material: MeshMaterial,
    /// Local-space AABB `[x_min, y_min, x_max, y_max]`.
    /// Used to compute offscreen texture sizes for filter rendering.
    pub local_bounds: [f32; 4],
}

#[derive(Debug, Clone)]
pub enum MeshMaterial {
    Color,
    Gradient(Handle<GradientMaterial>),
    Bitmap(Handle<BitmapMaterial>),
}

impl VabAsset {
    /// Root timeline frame count — the furthest frame any clip reaches.
    pub fn root_frame_count(&self) -> usize {
        self.baked
            .clips
            .iter()
            .map(|clip| clip.start_frame as usize + clip.frames.len())
            .max()
            .unwrap_or(0)
    }

    /// Frame rate to play at, falling back to [`DEFAULT_FRAME_RATE`].
    pub fn playback_frame_rate(&self) -> f32 {
        let frame_rate = self.baked.frame_rate;
        if frame_rate.is_finite() && frame_rate > 0.0 {
            frame_rate
        } else {
            DEFAULT_FRAME_RATE
        }
    }

    /// Build the frame's command list by sampling the baked clip covering the
    /// given root-timeline frame.
    ///
    /// Ordinary sprite timelines were already expanded at bake time, so there is
    /// no per-instance timeline state to maintain here.
    pub fn build_frame_commands(&self, scale: Vec3, frame: usize) -> CommandList {
        for (index, clip) in self.baked.clips.iter().enumerate() {
            if let Some(local) = frame.checked_sub(clip.start_frame as usize)
                && local < clip.frames.len()
            {
                return self
                    .sample(index, local, &crate::sampling::VabSkin::default(), scale)
                    .unwrap_or_default();
            }
        }
        CommandList::default()
    }
}

/// Compute the transformed AABB of all meshes reachable from `commands`,
/// returning `[offset_x, offset_y, pixel_width, pixel_height]` for offscreen
/// texture sizing. Nested `Blend` / `ApplyFilter` sub-lists are included.
///
/// Bounds are in the parent's local space; the renderer must translate by
/// `-offset` when replaying `commands` into the offscreen texture.
pub(crate) fn compute_command_bounds(asset: &VabAsset, commands: &CommandList) -> [f32; 4] {
    let mut accumulated = [
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    ];
    accumulate_command_bounds(asset, commands, &mut accumulated);

    if !accumulated[0].is_finite() {
        return [0.0, 0.0, 0.0, 0.0];
    }

    // Convert [x_min, y_min, x_max, y_max] → [offset_x, offset_y, width, height]
    [
        accumulated[0],
        accumulated[1],
        (accumulated[2] - accumulated[0]).ceil().max(0.0),
        (accumulated[3] - accumulated[1]).ceil().max(0.0),
    ]
}

fn accumulate_command_bounds(asset: &VabAsset, commands: &CommandList, bounds: &mut [f32; 4]) {
    for command in &commands.commands {
        match command {
            VabCommand::RenderShape { handle, transform } => {
                let Some(render_mesh) = asset.render_meshes.get(*handle) else {
                    continue;
                };
                let local_bounds = &render_mesh.local_bounds;
                let matrix = &transform.matrix;
                let corners = [
                    [local_bounds[0], local_bounds[1]],
                    [local_bounds[0], local_bounds[3]],
                    [local_bounds[2], local_bounds[1]],
                    [local_bounds[2], local_bounds[3]],
                ];
                for [local_x, local_y] in &corners {
                    let world_x = matrix.a * local_x + matrix.c * local_y + matrix.tx;
                    let world_y = matrix.b * local_x + matrix.d * local_y + matrix.ty;
                    if world_x < bounds[0] {
                        bounds[0] = world_x;
                    }
                    if world_y < bounds[1] {
                        bounds[1] = world_y;
                    }
                    if world_x > bounds[2] {
                        bounds[2] = world_x;
                    }
                    if world_y > bounds[3] {
                        bounds[3] = world_y;
                    }
                }
            }
            VabCommand::ApplyFilter { bounds: output, .. } => {
                bounds[0] = bounds[0].min(output[0]);
                bounds[1] = bounds[1].min(output[1]);
                bounds[2] = bounds[2].max(output[0] + output[2]);
                bounds[3] = bounds[3].max(output[1] + output[3]);
            }
            VabCommand::Blend(commands, _) => {
                accumulate_command_bounds(asset, commands, bounds);
            }
            VabCommand::PushMask
            | VabCommand::ActivateMask
            | VabCommand::DeactivateMask
            | VabCommand::PopMask => {}
        }
    }
}

/// Wrapper component so a loaded `VabAsset` can be attached to a Bevy entity.
#[derive(Component, Clone, ExtractComponent)]
#[component(on_add = add_visibility_class::<VabAssetHandle>)]
#[require(Transform, Visibility, VisibilityClass)]
pub struct VabAssetHandle(pub Handle<VabAsset>);

// ===========================================================================
// Asset loader
// ===========================================================================

#[derive(Default, TypePath)]
pub struct VabLoader;

impl AssetLoader for VabLoader {
    type Asset = VabAsset;
    type Settings = ();
    type Error = anyhow::Error;

    fn extensions(&self) -> &[&str] {
        &["vab"]
    }

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut data = Vec::new();
        reader.read_to_end(&mut data).await?;
        let vab = VabReader::from_bytes(&data)?;

        let vertices = vab.vertices().context("VAB is missing the VERT chunk")?;
        let indices = vab.indices().context("VAB is missing the INDX chunk")?;
        let shape_meshes = vab
            .shape_meshes()
            .context("VAB is missing the SHME chunk")?;
        let gradient_uniforms = vab
            .gradient_uniforms()
            .context("VAB is missing the GRAD chunk")?;
        let bitmap_uniforms = vab
            .bitmap_uniforms()
            .context("VAB is missing the BMAP chunk")?;
        let texture_bytes = vab
            .texture_data()
            .context("VAB is missing the TEXT chunk")?;
        let shape_records = vab
            .shape_records()
            .context("VAB is missing the SHAP chunk")?;
        let morph_entries = vab
            .morph_entries()
            .context("VAB is missing the MORP chunk")?;

        // One `RenderMeshGroup` per VAB ShapeMesh, in file order — `ShapeHandle`
        // is a positional index into `render_meshes`, so a mesh may never be
        // skipped silently.
        let mut render_meshes = Vec::with_capacity(shape_meshes.len());
        let mut textures = HashMap::new();
        for (mesh_index, shape_mesh) in shape_meshes.iter().enumerate() {
            let mesh = build_mesh(vertices, indices, shape_mesh, mesh_index)?;

            let material = match shape_mesh.material_type {
                vatf::material::COLOR => MeshMaterial::Color,
                vatf::material::GRADIENT => {
                    let gradient_uniforms = gradient_uniforms
                        .get(shape_mesh.material_offset as usize)
                        .with_context(|| {
                            format!(
                                "mesh {mesh_index}: GRAD index {} out of range ({} entries)",
                                shape_mesh.material_offset,
                                gradient_uniforms.len(),
                            )
                        })?;
                    let texture_handle = load_shared_texture(
                        load_context,
                        &mut textures,
                        texture_bytes,
                        shape_mesh,
                        mesh_index,
                        1,
                    )?;

                    let gradient_material = GradientMaterial {
                        texture: texture_handle,
                        gradient: GradientUniformsVab::from_vab(gradient_uniforms),
                        texture_transform: Mat4::from_mat3(Mat3::from_cols_array_2d(&[
                            [
                                gradient_uniforms.texture_transform[0],
                                gradient_uniforms.texture_transform[1],
                                0.0,
                            ],
                            [
                                gradient_uniforms.texture_transform[2],
                                gradient_uniforms.texture_transform[3],
                                0.0,
                            ],
                            [
                                gradient_uniforms.texture_transform[4],
                                gradient_uniforms.texture_transform[5],
                                1.0,
                            ],
                        ])),
                    };
                    MeshMaterial::Gradient(
                        load_context
                            .add_labeled_asset(format!("gmat_{mesh_index}"), gradient_material),
                    )
                }
                vatf::material::BITMAP => {
                    let bitmap_uniforms = bitmap_uniforms
                        .get(shape_mesh.material_offset as usize)
                        .with_context(|| {
                            format!(
                                "mesh {mesh_index}: BMAP index {} out of range ({} entries)",
                                shape_mesh.material_offset,
                                bitmap_uniforms.len(),
                            )
                        })?;
                    let texture_handle = load_shared_texture(
                        load_context,
                        &mut textures,
                        texture_bytes,
                        shape_mesh,
                        mesh_index,
                        shape_mesh.sampler_flags,
                    )?;

                    MeshMaterial::Bitmap(load_context.add_labeled_asset(
                        format!("bmat_{mesh_index}"),
                        BitmapMaterial {
                            texture: texture_handle,
                            texture_transform: Mat4::from_mat3(Mat3::from_cols_array_2d(&[
                                [bitmap_uniforms[0], bitmap_uniforms[1], 0.0],
                                [bitmap_uniforms[2], bitmap_uniforms[3], 0.0],
                                [bitmap_uniforms[4], bitmap_uniforms[5], 1.0],
                            ])),
                        },
                    ))
                }
                other => bail!("mesh {mesh_index}: unknown material type {other}"),
            };

            render_meshes.push(RenderMeshGroup {
                mesh: load_context.add_labeled_asset(format!("mesh_{mesh_index}"), mesh),
                material,
                local_bounds: local_bounds(vertices, shape_mesh, mesh_index)?,
            });
        }

        // Shape id → render_meshes range, validated against the meshes we built.
        let mut shape_map: HashMap<u16, Vec<ShapeHandle>> =
            HashMap::with_capacity(shape_records.len());
        for shape_record in shape_records {
            let start = shape_record.sub_shape_offset as usize;
            let count = shape_record.sub_shape_count as usize;
            let end = start
                .checked_add(count)
                .with_context(|| format!("shape {}: mesh range overflow", shape_record.id))?;
            if end > render_meshes.len() {
                bail!(
                    "shape {}: mesh range {start}..{end} out of bounds ({} meshes)",
                    shape_record.id,
                    render_meshes.len(),
                );
            }
            shape_map.insert(shape_record.id, (start..end).collect());
        }

        // (morph_id, ratio) → render_meshes range, resolved through each
        // MorphEntry's own vertex/index range rather than a positional guess.
        let mesh_lookup = build_mesh_lookup(shape_meshes, render_meshes.len());
        let mut morph_map: HashMap<(u16, u16), Vec<ShapeHandle>> = HashMap::new();
        for morph_entry in morph_entries {
            let key = (
                morph_entry.vertex_offset,
                morph_entry.vertex_count,
                morph_entry.index_offset,
                morph_entry.index_count,
            );
            let mesh_index = mesh_lookup.get(&key).with_context(|| {
                format!(
                    "morph {} ratio {}: no mesh matches vertex {}..{} index {}..{}",
                    morph_entry.morph_id,
                    morph_entry.ratio,
                    morph_entry.vertex_offset,
                    morph_entry.vertex_offset + morph_entry.vertex_count,
                    morph_entry.index_offset,
                    morph_entry.index_offset + morph_entry.index_count,
                )
            })?;
            morph_map
                .entry((morph_entry.morph_id, morph_entry.ratio))
                .or_default()
                .push(*mesh_index);
        }

        // ── Animation lookup ──────────────────────────────────────────────
        // BAKD carries everything the renderer needs, frame rate included.
        let baked = vab.into_baked();

        let asset = VabAsset {
            baked,
            shape_map,
            morph_map,
            render_meshes,
        };
        asset.validate_baked_references()?;
        Ok(asset)
    }
}

// ── Decode helpers ─────────────────────────────────────────────────────────

/// Maps a mesh's `(vertex_offset, vertex_count, index_offset, index_count)`
/// tuple to its index in `render_meshes`.
type MeshRangeKey = (u32, u32, u32, u32);

fn build_mesh_lookup(
    shape_meshes: &[vatf::ShapeMesh],
    render_mesh_count: usize,
) -> HashMap<MeshRangeKey, usize> {
    shape_meshes
        .iter()
        .take(render_mesh_count)
        .enumerate()
        .map(|(mesh_index, shape_mesh)| {
            (
                (
                    shape_mesh.vertex_offset,
                    shape_mesh.vertex_count,
                    shape_mesh.index_offset,
                    shape_mesh.index_count,
                ),
                mesh_index,
            )
        })
        .collect()
}

/// Dequantises one VAB ShapeMesh into a Bevy `Mesh`.
fn build_mesh(
    vertices: &[vatf::Vertex],
    indices: &[u32],
    shape_mesh: &vatf::ShapeMesh,
    mesh_index: usize,
) -> Result<Mesh> {
    let vertex_start = shape_mesh.vertex_offset as usize;
    let vertex_end = vertex_start + shape_mesh.vertex_count as usize;
    let index_start = shape_mesh.index_offset as usize;
    let index_end = index_start + shape_mesh.index_count as usize;

    let vertex_slice = vertices.get(vertex_start..vertex_end).with_context(|| {
        format!(
            "mesh {mesh_index}: vertex range {vertex_start}..{vertex_end} out of bounds ({} vertices)",
            vertices.len(),
        )
    })?;
    let index_slice = indices.get(index_start..index_end).with_context(|| {
        format!(
            "mesh {mesh_index}: index range {index_start}..{index_end} out of bounds ({} indices)",
            indices.len(),
        )
    })?;
    if index_slice.len() % 3 != 0
        || index_slice
            .iter()
            .any(|i| *i as usize >= vertex_slice.len())
    {
        bail!("mesh {mesh_index}: invalid triangle indices");
    }

    let mut positions: Vec<[f32; 3]> = Vec::with_capacity(vertex_slice.len());
    let mut colors: Vec<[f32; 4]> = Vec::with_capacity(vertex_slice.len());
    for vertex in vertex_slice {
        let local_x =
            vertex.x as f32 / 32767.0 * shape_mesh.bounds_half_x + shape_mesh.bounds_center_x;
        let local_y =
            vertex.y as f32 / 32767.0 * shape_mesh.bounds_half_y + shape_mesh.bounds_center_y;
        positions.push([local_x, local_y, 0.0]);
        // Vertex colours are stored as sRGB bytes; Bevy expects linear.
        let linear_color = Color::srgba_u8(
            vertex.color.r,
            vertex.color.g,
            vertex.color.b,
            vertex.color.a,
        )
        .to_linear();
        colors.push([
            linear_color.red,
            linear_color.green,
            linear_color.blue,
            linear_color.alpha,
        ]);
    }

    Ok(Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
    .with_inserted_indices(Indices::U32(index_slice.to_vec())))
}

/// Computes a mesh's local-space AABB `[x_min, y_min, x_max, y_max]`.
fn local_bounds(
    vertices: &[vatf::Vertex],
    shape_mesh: &vatf::ShapeMesh,
    mesh_index: usize,
) -> Result<[f32; 4]> {
    let vertex_start = shape_mesh.vertex_offset as usize;
    let vertex_end = vertex_start + shape_mesh.vertex_count as usize;
    let vertex_slice = vertices.get(vertex_start..vertex_end).with_context(|| {
        format!(
            "mesh {mesh_index}: vertex range {vertex_start}..{vertex_end} out of bounds ({} vertices)",
            vertices.len(),
        )
    })?;

    let mut bounds = [
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    ];
    for vertex in vertex_slice {
        let local_x =
            vertex.x as f32 / 32767.0 * shape_mesh.bounds_half_x + shape_mesh.bounds_center_x;
        let local_y =
            vertex.y as f32 / 32767.0 * shape_mesh.bounds_half_y + shape_mesh.bounds_center_y;
        bounds[0] = bounds[0].min(local_x);
        bounds[1] = bounds[1].min(local_y);
        bounds[2] = bounds[2].max(local_x);
        bounds[3] = bounds[3].max(local_y);
    }

    if !bounds[0].is_finite() {
        return Ok([0.0, 0.0, 0.0, 0.0]);
    }
    Ok(bounds)
}

/// Decodes the WebP texture slice referenced by a ShapeMesh.
fn decode_texture(
    texture_bytes: &[u8],
    texture_offset: u32,
    texture_length: u32,
    mesh_index: usize,
) -> Result<Image> {
    let start = texture_offset as usize;
    let end = start + texture_length as usize;
    let data = texture_bytes.get(start..end).with_context(|| {
        format!(
            "mesh {mesh_index}: texture range {start}..{end} out of bounds ({} bytes)",
            texture_bytes.len(),
        )
    })?;

    let decoded = image::load_from_memory(data)
        .with_context(|| format!("mesh {mesh_index}: failed to decode texture"))?;
    let rgba = decoded.to_rgba8();
    let (width, height) = rgba.dimensions();

    Ok(Image::new(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        rgba.into_raw(),
        // VATF bitmap payloads are premultiplied in encoded sRGB. The custom
        // shader unpremultiplies and applies Flash color transforms before linearizing.
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    ))
}

// ── Tests ──────────────────────────────────────────────────────────────────

fn load_shared_texture(
    context: &mut LoadContext,
    cache: &mut HashMap<(u32, u32, u16), Handle<Image>>,
    bytes: &[u8],
    mesh: &vatf::ShapeMesh,
    index: usize,
    flags: u16,
) -> Result<Handle<Image>> {
    if flags > 3 {
        bail!("mesh {index}: unknown sampler flags {flags}");
    }
    let key = (mesh.texture_offset, mesh.texture_length, flags);
    if let Some(handle) = cache.get(&key) {
        return Ok(handle.clone());
    }
    let mut image = decode_texture(bytes, key.0, key.1, index)?;
    let address = if flags & 2 != 0 {
        ImageAddressMode::Repeat
    } else {
        ImageAddressMode::ClampToEdge
    };
    let filter = if flags & 1 != 0 {
        ImageFilterMode::Linear
    } else {
        ImageFilterMode::Nearest
    };
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: address,
        address_mode_v: address,
        mag_filter: filter,
        min_filter: filter,
        ..Default::default()
    });
    let handle = context.add_labeled_asset(format!("texture_{}_{}_{}", key.0, key.1, key.2), image);
    cache.insert(key, handle.clone());
    Ok(handle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::mesh::VertexAttributeValues;
    use vatf::{ShapeMesh, Vertex};

    fn vertex(x: i16, y: i16, red: u8, green: u8, blue: u8, alpha: u8) -> Vertex {
        Vertex {
            x,
            y,
            color: vatf::Color {
                r: red,
                g: green,
                b: blue,
                a: alpha,
            },
        }
    }

    fn triangle_shape_mesh() -> ShapeMesh {
        ShapeMesh {
            vertex_count: 3,
            index_count: 3,
            material_type: vatf::material::COLOR,
            bounds_half_x: 50.0,
            bounds_half_y: 50.0,
            ..Default::default()
        }
    }

    fn triangle() -> (Vec<Vertex>, Vec<u32>) {
        (
            vec![
                vertex(0, 0, 255, 255, 255, 255),
                vertex(32767, 0, 128, 128, 128, 255),
                vertex(0, 32767, 0, 0, 0, 0),
            ],
            vec![0, 1, 2],
        )
    }

    #[test]
    fn vertex_positions_dequantize_against_bounds() {
        let (vertices, indices) = triangle();
        let mesh = build_mesh(&vertices, &indices, &triangle_shape_mesh(), 0).unwrap();

        let Some(VertexAttributeValues::Float32x3(positions)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("expected Float32x3 positions");
        };
        assert_eq!(positions[0], [0.0, 0.0, 0.0]);
        assert_eq!(positions[1], [50.0, 0.0, 0.0]);
        assert_eq!(positions[2], [0.0, 50.0, 0.0]);
    }

    #[test]
    fn vertex_colors_are_linearized() {
        let (vertices, indices) = triangle();
        let mesh = build_mesh(&vertices, &indices, &triangle_shape_mesh(), 0).unwrap();

        let Some(VertexAttributeValues::Float32x4(colors)) = mesh.attribute(Mesh::ATTRIBUTE_COLOR)
        else {
            panic!("expected Float32x4 colors");
        };
        assert_eq!(colors[0], [1.0, 1.0, 1.0, 1.0]);
        assert!(
            (colors[1][0] - 0.215_860_8).abs() < 1e-5,
            "mid grey must be linearized, got {}",
            colors[1][0],
        );
        assert_eq!(colors[2], [0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn decode_rejects_out_of_range_vertex_range() {
        let (vertices, indices) = triangle();
        let mut shape_mesh = triangle_shape_mesh();
        shape_mesh.vertex_count = 9;
        shape_mesh.vertex_offset = 2;

        let error = build_mesh(&vertices, &indices, &shape_mesh, 7).unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains("mesh 7"),
            "error must name the mesh: {message}"
        );
        assert!(
            message.contains("vertex range"),
            "error must name the range: {message}",
        );
    }

    #[test]
    fn decode_texture_rejects_out_of_range_slice() {
        let error = decode_texture(&[0u8; 4], 2, 100, 3).unwrap_err();
        assert!(
            error.to_string().contains("mesh 3"),
            "error must name the mesh: {error}",
        );
    }
}
