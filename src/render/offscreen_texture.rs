use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use bevy::app::{App, Plugin};
use bevy::asset::load_internal_asset;
use bevy::color::{Color, LinearRgba};
use bevy::ecs::entity::Entity;
use bevy::ecs::query::With;
use bevy::ecs::resource::Resource;
use bevy::ecs::schedule::IntoScheduleConfigs;
use bevy::ecs::system::{Commands, Query, Res, ResMut};
use bevy::image::BevyDefault;
use bevy::log::error;
use bevy::math::{UVec2, Vec3};
use bevy::platform::collections::hash_map::Entry;
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::{Deref, DerefMut, ReflectComponent};
use bevy::render::camera::{ManualTextureViews, NormalizedRenderTarget, RenderTarget};
use bevy::render::extract_component::ExtractComponentPlugin;
use bevy::render::mesh::RenderMesh;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_graph::{InternedRenderSubGraph, RenderSubGraph};
use bevy::render::render_resource::{
    Extent3d, PipelineCache, RenderPassColorAttachment, Shader, SpecializedMeshPipelines,
    TextureDescriptor, TextureDimension, TextureFormat, TextureUsages, TextureView,
};
use bevy::render::renderer::RenderDevice;
use bevy::render::sync_world::{MainEntity, RenderEntity, SyncToRenderWorld};
use bevy::render::texture::{GpuImage, OutputColorAttachment, TextureCache};
use bevy::render::view::{ExtractedWindows, Msaa, PostProcessWrite, ViewTargetAttachments};
use bevy::render::{Extract, ExtractSchedule, Render, RenderApp, RenderSet};
use bevy::{ecs::component::Component, reflect::Reflect};

use crate::commands::OffscreenDrawCommands;
use crate::render::graph::{DrawPhase, DrawType, OffscreenCore2d, OffscreenFlashShapeRenderPhases};
use crate::render::pipeline::{
    BEVEL_FILTER_SHADER_HANDLE, BLUR_FILTER_SHADER_HANDLE, COLOR_MATRIX_FILTER_SHADER_HANDLE,
    GLOW_FILTER_SHADER_HANDLE, OFFSCREEN_MESH2D_BITMAP_SHADER_HANDLE,
    OFFSCREEN_MESH2D_GRADIENT_SHADER_HANDLE, OFFSCREEN_MESH2D_SHADER_HANDLE, OffscreenMesh2dKey,
    OffscreenMesh2dPipeline,
};
use crate::render::texture_attachment::ColorAttachment;
use crate::swf_runtime::filter::Filter;
#[derive(Component, Default, Clone)]
#[require(
    OffscreenTextureRenderGraph::new(OffscreenCore2d),
    Msaa,
    SyncToRenderWorld
)]
pub struct OffscreenTexture {
    pub is_active: bool,
    pub order: isize,
    pub size: UVec2,
    pub target: RenderTarget,
    pub clear_color: Color,
    pub filters: Vec<Filter>,
    pub scale: Vec3,
}

#[derive(Component, Debug, Deref, DerefMut, Reflect, Clone)]
#[reflect(opaque)]
#[reflect(Component, Debug, Clone)]
pub struct OffscreenTextureRenderGraph(InternedRenderSubGraph);

impl OffscreenTextureRenderGraph {
    /// Creates a new [`OffscreenTextureRenderGraph`] from any string-like type.
    #[inline]
    pub fn new<T: RenderSubGraph>(name: T) -> Self {
        Self(name.intern())
    }
}

#[derive(Component)]
pub struct ExtractedOffscreenTexture {
    pub order: isize,
    pub size: UVec2,
    pub target: Option<NormalizedRenderTarget>,
    pub clear_color: Color,
    pub render_graph: InternedRenderSubGraph,
    pub filters: Vec<Filter>,
    pub scale: Vec3,
}

pub fn extract_offscreen_textures(
    mut commands: Commands,
    mut render_phases: ResMut<OffscreenFlashShapeRenderPhases>,
    query: Extract<
        Query<(
            RenderEntity,
            &OffscreenTexture,
            &OffscreenTextureRenderGraph,
        )>,
    >,
) {
    let mut live_entities = <HashSet<MainEntity>>::new();
    for (render_entity, offscreen_texture, render_graph) in query.iter() {
        if !offscreen_texture.is_active {
            commands
                .entity(render_entity)
                .remove::<ExtractedOffscreenTexture>();
            continue;
        }
        let mut commands = commands.entity(render_entity);
        commands.insert(ExtractedOffscreenTexture {
            order: offscreen_texture.order,
            size: offscreen_texture.size,
            target: offscreen_texture.target.normalize(None),
            clear_color: offscreen_texture.clear_color,
            render_graph: render_graph.0,
            filters: offscreen_texture.filters.clone(),
            scale: offscreen_texture.scale,
        });
        render_phases.insert_or_clear(render_entity.into());
        live_entities.insert(render_entity.into());
    }
    render_phases.retain(|k, _| live_entities.contains(k));
}

#[derive(Resource, Default, DerefMut, Deref)]
pub struct SortedOffscreenTextures(pub Vec<SortedOffscreenTexture>);

pub struct SortedOffscreenTexture {
    pub entity: Entity,
    pub order: isize,
}

#[derive(Component)]
pub struct ViewTarget {
    main_textures: MainTargetTextures,
    main_texture_format: TextureFormat,

    main_texture: Arc<AtomicUsize>,
    out_texture: OutputColorAttachment,
}

impl ViewTarget {
    pub fn new(
        main_texture_format: TextureFormat,
        main_texture: Arc<AtomicUsize>,
        main_textures: MainTargetTextures,
        out_texture: OutputColorAttachment,
    ) -> Self {
        Self {
            main_textures,
            main_texture_format,
            main_texture,
            out_texture,
        }
    }

    /// 获取此目标的主纹理的颜色附件。
    pub fn get_color_attachment(&self) -> RenderPassColorAttachment {
        if self.main_texture.load(Ordering::SeqCst) == 0 {
            self.main_textures.a.get_attachment()
        } else {
            self.main_textures.b.get_attachment()
        }
    }

    /// “主” 未采样纹理视图
    pub fn main_texture_view(&self) -> &TextureView {
        if self.main_texture.load(Ordering::SeqCst) == 0 {
            &self.main_textures.a.texture.default_view
        } else {
            &self.main_textures.b.texture.default_view
        }
    }

    #[inline]
    pub fn main_texture_format(&self) -> TextureFormat {
        self.main_texture_format
    }

    /// 此视图将要渲染到的最终纹理。
    #[inline]
    pub fn out_texture(&self) -> &TextureView {
        &self.out_texture.view
    }

    pub fn out_texture_color_attachment(
        &self,
        clear_color: Option<LinearRgba>,
    ) -> RenderPassColorAttachment {
        self.out_texture.get_attachment(clear_color)
    }

    #[inline]
    pub fn out_texture_format(&self) -> TextureFormat {
        self.out_texture.format
    }

    pub fn post_process_write(&self) -> PostProcessWrite {
        let old_is_a_main_texture = self.main_texture.fetch_xor(1, Ordering::SeqCst);
        // if the old main texture is a, then the post processing must write from a to b
        if old_is_a_main_texture == 0 {
            self.main_textures.b.mark_as_cleared();
            PostProcessWrite {
                source: &self.main_textures.a.texture.default_view,
                source_texture: &self.main_textures.a.texture.texture,
                destination: &self.main_textures.b.texture.default_view,
                destination_texture: &self.main_textures.b.texture.texture,
            }
        } else {
            self.main_textures.a.mark_as_cleared();
            PostProcessWrite {
                source: &self.main_textures.b.texture.default_view,
                source_texture: &self.main_textures.b.texture.texture,
                destination: &self.main_textures.a.texture.default_view,
                destination_texture: &self.main_textures.a.texture.texture,
            }
        }
    }
}

#[derive(Clone)]
pub struct MainTargetTextures {
    a: ColorAttachment,
    b: ColorAttachment,

    main_texture: Arc<AtomicUsize>,
}

impl MainTargetTextures {
    pub fn new(a: ColorAttachment, b: ColorAttachment, main_texture: Arc<AtomicUsize>) -> Self {
        Self { a, b, main_texture }
    }

    pub fn main_texture(&self) -> Arc<AtomicUsize> {
        self.main_texture.clone()
    }
}

pub struct OffscreenTexturePlugin;

impl Plugin for OffscreenTexturePlugin {
    fn build(&self, app: &mut bevy::app::App) {
        load_internal_asset!(
            app,
            OFFSCREEN_MESH2D_SHADER_HANDLE,
            "shaders/offscreen_mesh2d/color.wgsl",
            Shader::from_wgsl
        );
        load_internal_asset!(
            app,
            OFFSCREEN_MESH2D_GRADIENT_SHADER_HANDLE,
            "shaders/offscreen_mesh2d/gradient.wgsl",
            Shader::from_wgsl
        );
        load_internal_asset!(
            app,
            OFFSCREEN_MESH2D_BITMAP_SHADER_HANDLE,
            "shaders/offscreen_mesh2d/bitmap.wgsl",
            Shader::from_wgsl
        );
        load_internal_asset!(
            app,
            BLUR_FILTER_SHADER_HANDLE,
            "shaders/filters/blur.wgsl",
            Shader::from_wgsl
        );
        load_internal_asset!(
            app,
            COLOR_MATRIX_FILTER_SHADER_HANDLE,
            "shaders/filters/color_matrix.wgsl",
            Shader::from_wgsl
        );
        load_internal_asset!(
            app,
            GLOW_FILTER_SHADER_HANDLE,
            "shaders/filters/glow.wgsl",
            Shader::from_wgsl
        );
        load_internal_asset!(
            app,
            BEVEL_FILTER_SHADER_HANDLE,
            "shaders/filters/bevel.wgsl",
            Shader::from_wgsl
        );

        app.add_plugins(ExtractComponentPlugin::<OffscreenDrawCommands>::default());

        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<SortedOffscreenTextures>()
            .init_resource::<OffscreenFlashShapeRenderPhases>()
            .init_resource::<SpecializedMeshPipelines<OffscreenMesh2dPipeline>>()
            .add_systems(ExtractSchedule, extract_offscreen_textures)
            .add_systems(
                Render,
                (
                    sort_offscreen_textures.in_set(RenderSet::ManageViews),
                    prepare_offscreen_view_attachments.in_set(RenderSet::ManageViews),
                    prepare_offscreen_texture_view_target
                        .in_set(RenderSet::ManageViews)
                        .after(prepare_offscreen_view_attachments),
                    special_and_queue_shape_draw.in_set(RenderSet::Queue),
                ),
            );
    }

    fn finish(&self, app: &mut App) {
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app.init_resource::<OffscreenMesh2dPipeline>();
        }
    }
}

fn sort_offscreen_textures(
    mut sorted_offscreen_textures: ResMut<SortedOffscreenTextures>,
    mut offscreen_textures: Query<(Entity, &mut ExtractedOffscreenTexture)>,
) {
    sorted_offscreen_textures.clear();
    for (entity, offscreen_texture) in offscreen_textures.iter_mut() {
        sorted_offscreen_textures.push(SortedOffscreenTexture {
            entity,
            order: offscreen_texture.order,
        });
    }
    sorted_offscreen_textures.sort_by_key(|k| k.order);
}

fn prepare_offscreen_view_attachments(
    windows: Res<ExtractedWindows>,
    images: Res<RenderAssets<GpuImage>>,
    manual_texture_views: Res<ManualTextureViews>,
    offscreen_textures: Query<&ExtractedOffscreenTexture>,
    mut view_target_attachments: ResMut<ViewTargetAttachments>,
) {
    for offscreen_texture in offscreen_textures.iter() {
        let Some(target) = &offscreen_texture.target else {
            continue;
        };
        match view_target_attachments.entry(target.clone()) {
            Entry::Occupied(_) => {}
            Entry::Vacant(entry) => {
                let Some(attachment) = target
                    .get_texture_view(&windows, &images, &manual_texture_views)
                    .cloned()
                    .zip(target.get_texture_format(&windows, &images, &manual_texture_views))
                    .map(|(view, format)| OutputColorAttachment::new(view, format))
                else {
                    continue;
                };
                entry.insert(attachment);
            }
        }
    }
}

fn prepare_offscreen_texture_view_target(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    mut texture_cache: ResMut<TextureCache>,
    offscreen_textures: Query<(Entity, &ExtractedOffscreenTexture)>,
    view_target_attachments: ResMut<ViewTargetAttachments>,
) {
    let mut textures = HashMap::new();
    for (entity, offscreen_texture) in offscreen_textures.iter() {
        let (target_size, Some(target)) = (offscreen_texture.size, &offscreen_texture.target)
        else {
            continue;
        };

        let Some(out_attachment) = view_target_attachments.get(target) else {
            continue;
        };

        let size = Extent3d {
            width: target_size.x,
            height: target_size.y,
            depth_or_array_layers: 1,
        };

        let main_texture_format = TextureFormat::bevy_default();
        let msaa = Msaa::default();
        let clear_color = offscreen_texture.clear_color;
        let texture_usage = TextureUsages::RENDER_ATTACHMENT
            | TextureUsages::COPY_SRC
            | TextureUsages::TEXTURE_BINDING;
        let (a, b, sample, main_texture) = textures
            .entry((offscreen_texture.target.clone(), texture_usage, msaa))
            .or_insert_with(|| {
                let descriptor = TextureDescriptor {
                    label: None,
                    size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: TextureDimension::D2,
                    format: main_texture_format,
                    usage: texture_usage,
                    view_formats: match main_texture_format {
                        TextureFormat::Bgra8Unorm => &[TextureFormat::Bgra8UnormSrgb],
                        TextureFormat::Rgba8Unorm => &[TextureFormat::Rgba8UnormSrgb],
                        _ => &[],
                    },
                };
                let a = texture_cache.get(
                    &render_device,
                    TextureDescriptor {
                        label: Some("offscreen_texture_a"),
                        ..descriptor
                    },
                );
                let b = texture_cache.get(
                    &render_device,
                    TextureDescriptor {
                        label: Some("offscreen_texture_b"),
                        ..descriptor
                    },
                );
                let sampled = if msaa.samples() > 1 {
                    let sampled = texture_cache.get(
                        &render_device,
                        TextureDescriptor {
                            label: Some("main_texture_sampled"),
                            size,
                            mip_level_count: 1,
                            sample_count: msaa.samples(),
                            dimension: TextureDimension::D2,
                            format: main_texture_format,
                            usage: TextureUsages::RENDER_ATTACHMENT,
                            view_formats: descriptor.view_formats,
                        },
                    );
                    Some(sampled)
                } else {
                    None
                };
                let main_texture = Arc::new(AtomicUsize::new(0));
                (a, b, sampled, main_texture)
            });

        let main_textures = MainTargetTextures::new(
            ColorAttachment::new(a.clone(), sample.clone(), Some(clear_color.into())),
            ColorAttachment::new(b.clone(), sample.clone(), Some(clear_color.into())),
            main_texture.clone(),
        );

        commands.entity(entity).insert(ViewTarget::new(
            main_texture_format,
            main_textures.main_texture(),
            main_textures,
            out_attachment.clone(),
        ));
    }
}

pub fn special_and_queue_shape_draw(
    offscreen_mesh2d_pipeline: Res<OffscreenMesh2dPipeline>,
    mut pipelines: ResMut<SpecializedMeshPipelines<OffscreenMesh2dPipeline>>,
    pipeline_cache: Res<PipelineCache>,
    query: Query<(Entity, &OffscreenDrawCommands), With<ExtractedOffscreenTexture>>,
    render_meshes: Res<RenderAssets<RenderMesh>>,
    mut render_phases: ResMut<OffscreenFlashShapeRenderPhases>,
) {
    for (main_entity, offscreen_draw_commands) in query.iter() {
        let main_entity = MainEntity::from(main_entity);
        let Some(render_phase) = render_phases.get_mut(&main_entity) else {
            continue;
        };
        for draw_command in offscreen_draw_commands.iter() {
            let Some(mesh) = render_meshes.get(&draw_command.mesh) else {
                continue;
            };
            let Some(mesh_key) = OffscreenMesh2dKey::from_bits(draw_command.blend.bits() as u16)
            else {
                continue;
            };
            let mesh_key = mesh_key
                | match &draw_command.material_type {
                    crate::commands::MaterialType::Color(_) => OffscreenMesh2dKey::COLOR,
                    crate::commands::MaterialType::Gradient(_) => OffscreenMesh2dKey::GRADIENT,
                    crate::commands::MaterialType::Bitmap(_) => OffscreenMesh2dKey::BITMAP,
                };
            let pipeline_id = pipelines.specialize(
                &pipeline_cache,
                &offscreen_mesh2d_pipeline,
                mesh_key,
                &mesh.layout,
            );
            let pipeline_id = match pipeline_id {
                Ok(id) => id,
                Err(err) => {
                    error!("{}", err);
                    continue;
                }
            };
            render_phase.push(DrawPhase {
                draw_type: DrawType::from(&draw_command.material_type),
                mesh_asset_id: draw_command.mesh.id(),
                pipeline_id,
            });
        }
    }
}
