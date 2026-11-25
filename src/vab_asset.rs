//! Main VAB asset, render commands, decoding and loader entry point.
mod ui;

use anyhow::{Context, Result, bail};
use bevy::camera::visibility::{VisibilityClass, add_visibility_class};
use bevy::{
    asset::{Asset, AssetLoader, Handle, LoadContext, RenderAssetUsages, io::Reader},
    ecs::component::Component,
    image::{Image, ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor},
    math::{Mat3, Mat4, Vec3},
    mesh::{Indices, Mesh, MeshVertexAttribute, PrimitiveTopology, VertexAttributeValues},
    platform::collections::HashMap,
    prelude::{Transform, Visibility},
    reflect::TypePath,
    render::{
        extract_component::ExtractComponent,
        render_resource::{Extent3d, TextureDimension, TextureFormat, VertexFormat},
    },
};
use vatf::{
    animation::{AnimFilter, AnimMatrix, AnimTransform, filter_dest_rect},
    baked::BakedNode,
    reader::VabReader,
};

use crate::material::{BitmapMaterial, GradientMaterial, GradientUniformsVab};

/// Root sprite id — always 0 in VAB.
pub const ROOT_SPRITE_ID: u16 = 0;

/// Frame rate assumed when a VAB carries no usable frame rate.
pub const DEFAULT_FRAME_RATE: f32 = 30.0;

/// Compact 2D local position attribute (`Float32x2`) emitted by [`VabLoader`].
pub const ATTRIBUTE_VAB_POSITION: MeshVertexAttribute =
    MeshVertexAttribute::new("Vab_Position", 988_540_917, VertexFormat::Float32x2);

/// Compact encoded-sRGB vertex color attribute (`Unorm8x4`) emitted by [`VabLoader`].
pub const ATTRIBUTE_VAB_COLOR_SRGB: MeshVertexAttribute =
    MeshVertexAttribute::new("Vab_ColorSrgb", 988_540_918, VertexFormat::Unorm8x4);

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

    /// Conservative union bounds `[x, y, width, height]` in Flash local pixels
    /// across all frames (and all skin variants) of `clip`, including filter expansion.
    pub fn clip_bounds(&self, clip: usize) -> Option<[f32; 4]> {
        let clip = self.baked.clips.get(clip)?;
        let mut bounds = [
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        ];
        for frame in &clip.frames {
            accumulate_baked_bounds(self, frame, AnimMatrix::IDENTITY, &mut bounds, 0);
        }
        bounds_to_rect(bounds)
    }

    /// Conservative union bounds `[x, y, width, height]` in Flash local pixels
    /// across all clips and skin variants in this asset, including filter expansion.
    pub fn content_bounds(&self) -> Option<[f32; 4]> {
        let mut bounds = [
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        ];
        for clip in &self.baked.clips {
            for frame in &clip.frames {
                accumulate_baked_bounds(self, frame, AnimMatrix::IDENTITY, &mut bounds, 0);
            }
        }
        bounds_to_rect(bounds)
    }
}

fn bounds_to_rect(bounds: [f32; 4]) -> Option<[f32; 4]> {
    if !bounds.iter().all(|v| v.is_finite()) {
        return None;
    }
    Some([
        bounds[0],
        bounds[1],
        (bounds[2] - bounds[0]).max(0.0),
        (bounds[3] - bounds[1]).max(0.0),
    ])
}

fn union_rect(bounds: &mut [f32; 4], rect: [f32; 4]) {
    if rect.iter().all(|v| v.is_finite()) {
        bounds[0] = bounds[0].min(rect[0]);
        bounds[1] = bounds[1].min(rect[1]);
        bounds[2] = bounds[2].max(rect[0] + rect[2]);
        bounds[3] = bounds[3].max(rect[1] + rect[3]);
    }
}

fn accumulate_baked_bounds(
    asset: &VabAsset,
    nodes: &[BakedNode],
    parent: AnimMatrix,
    bounds: &mut [f32; 4],
    depth: usize,
) {
    if depth >= 128 {
        return;
    }
    for node in nodes {
        match node {
            BakedNode::Shape {
                id,
                ratio,
                transform,
            } => {
                let Some(handles) = asset
                    .shape_map
                    .get(id)
                    .or_else(|| asset.morph_map.get(&(*id, *ratio)))
                else {
                    continue;
                };
                let matrix = parent * transform.matrix;
                for &handle in handles {
                    let Some(render_mesh) = asset.render_meshes.get(handle) else {
                        continue;
                    };
                    let local_bounds = &render_mesh.local_bounds;
                    for [local_x, local_y] in [
                        [local_bounds[0], local_bounds[1]],
                        [local_bounds[0], local_bounds[3]],
                        [local_bounds[2], local_bounds[1]],
                        [local_bounds[2], local_bounds[3]],
                    ] {
                        let world_x = matrix.a * local_x + matrix.c * local_y + matrix.tx;
                        let world_y = matrix.b * local_x + matrix.d * local_y + matrix.ty;
                        bounds[0] = bounds[0].min(world_x);
                        bounds[1] = bounds[1].min(world_y);
                        bounds[2] = bounds[2].max(world_x);
                        bounds[3] = bounds[3].max(world_y);
                    }
                }
            }
            BakedNode::Skin {
                symbol, transform, ..
            } => {
                let Some(skin) = asset.baked.skins.iter().find(|s| s.symbol == *symbol) else {
                    continue;
                };
                let matrix = parent * transform.matrix;
                for variant in &skin.variants {
                    accumulate_baked_bounds(asset, &variant.nodes, matrix, bounds, depth + 1);
                }
            }
            BakedNode::Group {
                children, filters, ..
            } => {
                let active: Vec<_> = filters.iter().filter(|f| !f.impotent()).cloned().collect();
                if active.is_empty() {
                    accumulate_baked_bounds(asset, children, parent, bounds, depth + 1);
                } else {
                    let mut child_bounds = [
                        f32::INFINITY,
                        f32::INFINITY,
                        f32::NEG_INFINITY,
                        f32::NEG_INFINITY,
                    ];
                    accumulate_baked_bounds(asset, children, parent, &mut child_bounds, depth + 1);
                    if let Some([x, y, w, h]) = bounds_to_rect(child_bounds) {
                        let (fx, fy, fw, fh) = filter_dest_rect(x, y, w.ceil(), h.ceil(), &active);
                        union_rect(bounds, [fx, fy, fw, fh]);
                    }
                }
            }
            BakedNode::Mask { mask, children } => {
                let mut mask_bounds = [
                    f32::INFINITY,
                    f32::INFINITY,
                    f32::NEG_INFINITY,
                    f32::NEG_INFINITY,
                ];
                let mut child_bounds = [
                    f32::INFINITY,
                    f32::INFINITY,
                    f32::NEG_INFINITY,
                    f32::NEG_INFINITY,
                ];
                accumulate_baked_bounds(asset, mask, parent, &mut mask_bounds, depth + 1);
                accumulate_baked_bounds(asset, children, parent, &mut child_bounds, depth + 1);
                if let (Some(a), Some(b)) =
                    (bounds_to_rect(mask_bounds), bounds_to_rect(child_bounds))
                {
                    let x = a[0].max(b[0]);
                    let y = a[1].max(b[1]);
                    let right = (a[0] + a[2]).min(b[0] + b[2]);
                    let bottom = (a[1] + a[3]).min(b[1] + b[3]);
                    if right > x && bottom > y {
                        union_rect(bounds, [x, y, right - x, bottom - y]);
                    }
                }
            }
        }
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
#[extract_app(bevy::render::RenderApp)]
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
        let indices_u32 = vab.indices().context("VAB is missing the INDX chunk")?;
        let indices_u16 = vab.indices_u16().context("VAB is missing the IND2 chunk")?;
        let shape_meshes = vab
            .shape_meshes()
            .context("VAB is missing the SHME chunk")?;
        let gradient_uniforms = vab
            .gradient_uniforms()
            .context("VAB is missing the GRAD chunk")?;
        let bitmap_uniforms = vab
            .bitmap_uniforms()
            .context("VAB is missing the BMAP chunk")?;
        let texture_records = vab
            .texture_records()
            .context("VAB is missing the TEXD chunk")?;
        let texture_bytes = vab
            .texture_data()
            .context("VAB is missing the TEXT chunk")?;
        let shape_records = vab
            .shape_records()
            .context("VAB is missing the SHAP chunk")?;
        let morph_entries = vab
            .morph_entries()
            .context("VAB is missing the MORP chunk")?;

        // Cache decoded RGBA payloads per TEXD index so each embedded image is
        // WebP-decoded at most once even if referenced with multiple sampler modes.
        let mut decoded_textures: HashMap<u32, (u32, u32, Vec<u8>)> = HashMap::new();

        // Pack all 256×1 gradient ramps into shared 256×N atlas pages to avoid
        // creating hundreds of separate 256×1 GPU textures and bind groups.
        let gradient_atlas = build_gradient_atlas(
            load_context,
            shape_meshes,
            texture_records,
            texture_bytes,
            &mut decoded_textures,
        )?;

        // One `RenderMeshGroup` per VAB ShapeMesh, in file order — `ShapeHandle`
        // is a positional index into `render_meshes`, so a mesh may never be
        // skipped silently.
        let mut render_meshes = Vec::with_capacity(shape_meshes.len());
        let mut textures = HashMap::new();
        for (mesh_index, shape_mesh) in shape_meshes.iter().enumerate() {
            let mesh = build_mesh(vertices, indices_u32, indices_u16, shape_mesh, mesh_index)?;

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
                    let (texture_handle, ramp_v) =
                        if let Some((handle, v)) = gradient_atlas.get(&shape_mesh.texture_index) {
                            (handle.clone(), *v)
                        } else {
                            let handle = load_shared_texture(
                                load_context,
                                &mut textures,
                                &mut decoded_textures,
                                texture_records,
                                texture_bytes,
                                shape_mesh,
                                mesh_index,
                                1,
                            )?;
                            (handle, 0.5)
                        };

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
                        ramp_v,
                    };
                    MeshMaterial::Gradient(
                        load_context.add_labeled_asset(
                            format!("__vab/gmat_{mesh_index}"),
                            gradient_material,
                        ),
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
                        &mut decoded_textures,
                        texture_records,
                        texture_bytes,
                        shape_mesh,
                        mesh_index,
                        shape_mesh.sampler_flags,
                    )?;

                    MeshMaterial::Bitmap(load_context.add_labeled_asset(
                        format!("__vab/bmat_{mesh_index}"),
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
                mesh: load_context.add_labeled_asset(format!("__vab/mesh_{mesh_index}"), mesh),
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
        let graphics = vab.graphics()?;
        let buttons = vab.buttons()?;
        let baked = vab.into_baked();

        let asset = VabAsset {
            baked,
            shape_map,
            morph_map,
            render_meshes,
        };
        asset.validate_baked_references()?;
        ui::load_ui_assets(&asset, graphics, buttons, load_context)?;
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
    indices_u32: &[u32],
    indices_u16: &[u16],
    shape_mesh: &vatf::ShapeMesh,
    mesh_index: usize,
) -> Result<Mesh> {
    let vertex_start = shape_mesh.vertex_offset as usize;
    let vertex_end = vertex_start
        .checked_add(shape_mesh.vertex_count as usize)
        .with_context(|| format!("mesh {mesh_index}: vertex range overflow"))?;
    let index_start = shape_mesh.index_offset as usize;
    let index_end = index_start
        .checked_add(shape_mesh.index_count as usize)
        .with_context(|| format!("mesh {mesh_index}: index range overflow"))?;

    let vertex_slice = vertices.get(vertex_start..vertex_end).with_context(|| {
        format!(
            "mesh {mesh_index}: vertex range {vertex_start}..{vertex_end} out of bounds ({} vertices)",
            vertices.len(),
        )
    })?;

    let mesh_indices = match shape_mesh.index_format {
        vatf::index_format::U16 => {
            let index_slice = indices_u16.get(index_start..index_end).with_context(|| {
                format!(
                    "mesh {mesh_index}: u16 index range {index_start}..{index_end} out of bounds ({} indices)",
                    indices_u16.len(),
                )
            })?;
            if index_slice.len() % 3 != 0
                || index_slice
                    .iter()
                    .any(|i| *i as usize >= vertex_slice.len())
            {
                bail!("mesh {mesh_index}: invalid triangle indices");
            }
            Indices::U16(index_slice.to_vec())
        }
        vatf::index_format::U32 => {
            let index_slice = indices_u32.get(index_start..index_end).with_context(|| {
                format!(
                    "mesh {mesh_index}: u32 index range {index_start}..{index_end} out of bounds ({} indices)",
                    indices_u32.len(),
                )
            })?;
            if index_slice.len() % 3 != 0
                || index_slice
                    .iter()
                    .any(|i| *i as usize >= vertex_slice.len())
            {
                bail!("mesh {mesh_index}: invalid triangle indices");
            }
            if vertex_slice.len() <= u16::MAX as usize {
                Indices::U16(index_slice.iter().map(|&i| i as u16).collect())
            } else {
                Indices::U32(index_slice.to_vec())
            }
        }
        other => bail!("mesh {mesh_index}: unknown index format {other}"),
    };

    let mut positions: Vec<[f32; 2]> = Vec::with_capacity(vertex_slice.len());
    let mut colors: Vec<[u8; 4]> = Vec::with_capacity(vertex_slice.len());
    for vertex in vertex_slice {
        let local_x =
            vertex.x as f32 / 32767.0 * shape_mesh.bounds_half_x + shape_mesh.bounds_center_x;
        let local_y =
            vertex.y as f32 / 32767.0 * shape_mesh.bounds_half_y + shape_mesh.bounds_center_y;
        positions.push([local_x, local_y]);
        // Keep encoded sRGB bytes directly in Unorm8x4; VAB shaders work in encoded
        // sRGB space and avoid per-fragment linear→sRGB conversion.
        colors.push([
            vertex.color.r,
            vertex.color.g,
            vertex.color.b,
            vertex.color.a,
        ]);
    }

    Ok(Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(ATTRIBUTE_VAB_POSITION, positions)
    .with_inserted_attribute(
        ATTRIBUTE_VAB_COLOR_SRGB,
        VertexAttributeValues::Unorm8x4(colors),
    )
    .with_inserted_indices(mesh_indices))
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

/// Decodes a texture record from `TEXD`/`TEXT` into raw RGBA8 bytes.
fn decode_texture_record(
    texture_records: &[vatf::TextureRecord],
    texture_bytes: &[u8],
    texture_index: u32,
    mesh_index: usize,
) -> Result<(u32, u32, Vec<u8>)> {
    let record = texture_records
        .get(texture_index as usize)
        .with_context(|| {
            format!(
                "mesh {mesh_index}: texture index {texture_index} out of bounds ({} records)",
                texture_records.len(),
            )
        })?;
    let start = record.offset as usize;
    let end = start
        .checked_add(record.length as usize)
        .with_context(|| format!("mesh {mesh_index}: texture range overflow"))?;
    let data = texture_bytes.get(start..end).with_context(|| {
        format!(
            "mesh {mesh_index}: texture range {start}..{end} out of bounds ({} bytes)",
            texture_bytes.len(),
        )
    })?;

    match record.encoding {
        vatf::texture_encoding::WEBP => {
            let decoded = image::load_from_memory(data)
                .with_context(|| format!("mesh {mesh_index}: failed to decode texture"))?;
            let rgba = decoded.to_rgba8();
            let (width, height) = rgba.dimensions();
            Ok((width, height, rgba.into_raw()))
        }
        vatf::texture_encoding::RAW_RGBA8 => {
            let width = u32::from(record.width);
            let height = u32::from(record.height);
            let expected = (width as usize)
                .checked_mul(height as usize)
                .and_then(|n| n.checked_mul(4))
                .with_context(|| format!("mesh {mesh_index}: raw texture size overflow"))?;
            if data.len() != expected {
                bail!(
                    "mesh {mesh_index}: raw RGBA8 texture length {} != expected {expected}",
                    data.len()
                );
            }
            Ok((width, height, data.to_vec()))
        }
        other => bail!("mesh {mesh_index}: unknown texture encoding {other}"),
    }
}

const GRADIENT_RAMP_WIDTH: u32 = 256;
const MAX_GRADIENT_ATLAS_ROWS: usize = 2048;

/// Decodes unique 256×1 gradient ramps and packs them into shared 256×N textures.
fn build_gradient_atlas(
    context: &mut LoadContext,
    shape_meshes: &[vatf::ShapeMesh],
    texture_records: &[vatf::TextureRecord],
    texture_bytes: &[u8],
    decoded_textures: &mut HashMap<u32, (u32, u32, Vec<u8>)>,
) -> Result<HashMap<u32, (Handle<Image>, f32)>> {
    let mut seen = HashMap::<u32, usize>::new();
    let mut rows: Vec<(u32, Vec<u8>)> = Vec::new();
    for (mesh_index, shape_mesh) in shape_meshes.iter().enumerate() {
        if shape_mesh.material_type != vatf::material::GRADIENT {
            continue;
        }
        let tex_idx = shape_mesh.texture_index;
        if seen.contains_key(&tex_idx) {
            continue;
        }
        let (width, height, rgba) =
            decode_texture_record(texture_records, texture_bytes, tex_idx, mesh_index)?;
        if (width, height) == (GRADIENT_RAMP_WIDTH, 1) {
            seen.insert(tex_idx, rows.len());
            rows.push((tex_idx, rgba));
        } else {
            decoded_textures
                .entry(tex_idx)
                .or_insert((width, height, rgba));
        }
    }
    let mut result = HashMap::with_capacity(rows.len());
    for (page_index, chunk) in rows.chunks(MAX_GRADIENT_ATLAS_ROWS).enumerate() {
        let height = chunk.len() as u32;
        let mut pixels = Vec::with_capacity(GRADIENT_RAMP_WIDTH as usize * chunk.len() * 4);
        for (_, row_bytes) in chunk {
            pixels.extend_from_slice(row_bytes);
        }
        let mut image = Image::new(
            Extent3d {
                width: GRADIENT_RAMP_WIDTH,
                height,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            pixels,
            TextureFormat::Rgba8Unorm,
            RenderAssetUsages::RENDER_WORLD,
        );
        image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::ClampToEdge,
            address_mode_v: ImageAddressMode::ClampToEdge,
            mag_filter: ImageFilterMode::Linear,
            min_filter: ImageFilterMode::Linear,
            ..Default::default()
        });
        let handle = context.add_labeled_asset(format!("__vab/gradient_atlas_{page_index}"), image);
        for (row_index, (tex_idx, _)) in chunk.iter().enumerate() {
            let v = (row_index as f32 + 0.5) / height as f32;
            result.insert(*tex_idx, (handle.clone(), v));
        }
    }
    Ok(result)
}

#[allow(clippy::too_many_arguments)]
fn load_shared_texture(
    context: &mut LoadContext,
    cache: &mut HashMap<(u32, u16), Handle<Image>>,
    decoded_textures: &mut HashMap<u32, (u32, u32, Vec<u8>)>,
    texture_records: &[vatf::TextureRecord],
    bytes: &[u8],
    mesh: &vatf::ShapeMesh,
    index: usize,
    flags: u16,
) -> Result<Handle<Image>> {
    if flags > 3 {
        bail!("mesh {index}: unknown sampler flags {flags}");
    }
    let key = (mesh.texture_index, flags);
    if let Some(handle) = cache.get(&key) {
        return Ok(handle.clone());
    }
    let (width, height, rgba) = if let Some(cached) = decoded_textures.get(&mesh.texture_index) {
        cached
    } else {
        let decoded = decode_texture_record(texture_records, bytes, mesh.texture_index, index)?;
        decoded_textures.insert(mesh.texture_index, decoded);
        &decoded_textures[&mesh.texture_index]
    };
    let mut image = Image::new(
        Extent3d {
            width: *width,
            height: *height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        rgba.clone(),
        // VATF bitmap payloads are premultiplied in encoded sRGB. The custom
        // shader unpremultiplies and applies Flash color transforms before linearizing.
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
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
    let handle = context.add_labeled_asset(format!("__vab/texture_{}_{}", key.0, key.1), image);
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
        let mesh = build_mesh(&vertices, &indices, &[], &triangle_shape_mesh(), 0).unwrap();

        let Some(VertexAttributeValues::Float32x2(positions)) =
            mesh.attribute(ATTRIBUTE_VAB_POSITION)
        else {
            panic!("expected Float32x2 compact positions");
        };
        assert_eq!(positions[0], [0.0, 0.0]);
        assert_eq!(positions[1], [50.0, 0.0]);
        assert_eq!(positions[2], [0.0, 50.0]);
        assert!(
            matches!(mesh.indices(), Some(Indices::U16(idx)) if idx == &[0, 1, 2]),
            "meshes under 65536 vertices must use U16 indices"
        );
    }

    #[test]
    fn vertex_colors_stay_encoded_srgb_unorm8() {
        let (vertices, _) = triangle();
        let mut shape_mesh = triangle_shape_mesh();
        shape_mesh.index_format = vatf::index_format::U16;
        let mesh = build_mesh(&vertices, &[], &[0u16, 1, 2], &shape_mesh, 0).unwrap();

        let Some(VertexAttributeValues::Unorm8x4(colors)) =
            mesh.attribute(ATTRIBUTE_VAB_COLOR_SRGB)
        else {
            panic!("expected Unorm8x4 encoded sRGB colors");
        };
        assert_eq!(colors[0], [255, 255, 255, 255]);
        assert_eq!(colors[1], [128, 128, 128, 255]);
        assert_eq!(colors[2], [0, 0, 0, 0]);
        assert!(matches!(mesh.indices(), Some(Indices::U16(idx)) if idx == &[0, 1, 2]));
    }

    #[test]
    fn clip_and_content_bounds_include_skins_and_filters() {
        use vatf::animation::AnimBlurFilter;
        use vatf::baked::{BakedClip, BakedMovie, BakedSkin, BakedSkinVariant};

        let asset = VabAsset {
            baked: BakedMovie {
                frame_rate: 30.0,
                clips: vec![BakedClip {
                    name: "idle".into(),
                    start_frame: 0,
                    events: vec![],
                    frames: vec![vec![
                        BakedNode::Group {
                            children: vec![BakedNode::Shape {
                                id: 1,
                                ratio: 0,
                                transform: AnimTransform::default(),
                            }],
                            filters: vec![AnimFilter::BlurFilter(AnimBlurFilter {
                                blur_x: 9 * 65536,
                                blur_y: 9 * 65536,
                                num_passes: 1,
                            })],
                            blend_mode: 0,
                        },
                        BakedNode::Skin {
                            slot: "weapon".into(),
                            symbol: 10,
                            transform: AnimTransform::default(),
                        },
                    ]],
                }],
                skins: vec![BakedSkin {
                    symbol: 10,
                    variants: vec![
                        BakedSkinVariant {
                            name: "short".into(),
                            nodes: vec![BakedNode::Shape {
                                id: 1,
                                ratio: 0,
                                transform: AnimTransform::default(),
                            }],
                        },
                        BakedSkinVariant {
                            name: "long".into(),
                            nodes: vec![BakedNode::Shape {
                                id: 2,
                                ratio: 0,
                                transform: AnimTransform::default(),
                            }],
                        },
                    ],
                }],
            },
            shape_map: [(1, vec![0]), (2, vec![1])].into(),
            morph_map: HashMap::default(),
            render_meshes: vec![
                RenderMeshGroup {
                    mesh: Handle::default(),
                    material: MeshMaterial::Color,
                    local_bounds: [0.0, 0.0, 10.0, 10.0],
                },
                RenderMeshGroup {
                    mesh: Handle::default(),
                    material: MeshMaterial::Color,
                    local_bounds: [0.0, 0.0, 100.0, 20.0],
                },
            ],
        };
        let bounds = asset.clip_bounds(0).expect("clip bounds");
        assert!(
            bounds[0] < 0.0,
            "blur filter should expand min_x: {bounds:?}"
        );
        assert!(
            bounds[0] + bounds[2] >= 100.0,
            "skin variant 'long' should be included: {bounds:?}"
        );
        assert_eq!(asset.content_bounds(), Some(bounds));
    }

    #[test]
    fn decode_rejects_out_of_range_vertex_range() {
        let (vertices, indices) = triangle();
        let mut shape_mesh = triangle_shape_mesh();
        shape_mesh.vertex_count = 9;
        shape_mesh.vertex_offset = 2;

        let error = build_mesh(&vertices, &indices, &[], &shape_mesh, 7).unwrap_err();
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
        let records = [vatf::TextureRecord {
            offset: 2,
            length: 100,
            width: 1,
            height: 1,
            encoding: vatf::texture_encoding::WEBP,
            _pad: [0; 3],
        }];
        let error = decode_texture_record(&records, &[0u8; 4], 0, 3).unwrap_err();
        assert!(
            error.to_string().contains("mesh 3"),
            "error must name the mesh: {error}",
        );
    }
}
