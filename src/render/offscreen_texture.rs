use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use bevy::{
    app::Plugin,
    camera::{NormalizedRenderTarget, RenderTarget},
    color::{Color, LinearRgba},
    ecs::{
        component::Component,
        entity::Entity,
        query::With,
        resource::Resource,
        schedule::IntoScheduleConfigs,
        system::{Commands, Query, Res, ResMut},
    },
    log::error,
    math::{Mat4, UVec2, Vec3},
    platform::collections::{HashMap, HashSet, hash_map::Entry},
    prelude::{Deref, DerefMut, ReflectComponent},
    reflect::Reflect,
    render::{
        Extract, ExtractSchedule, Render, RenderApp, RenderStartup, RenderSystems,
        extract_component::ExtractComponentPlugin,
        mesh::RenderMesh,
        render_asset::RenderAssets,
        render_graph::{InternedRenderSubGraph, RenderSubGraph},
        render_resource::{
            BindGroup, BindGroupEntries, Extent3d, PipelineCache, RenderPassColorAttachment,
            SpecializedMeshPipelines, TextureDescriptor, TextureDimension, TextureFormat,
            TextureUsages, TextureView,
        },
        renderer::{RenderDevice, RenderQueue},
        sync_world::{MainEntity, RenderEntity, SyncToRenderWorld},
        texture::{GpuImage, OutputColorAttachment, TextureCache},
        view::{Msaa, PostProcessWrite, ViewTargetAttachments, prepare_windows},
    },
};

use crate::{
    commands::OffscreenDrawCommands,
    render::{
        graph::{DrawPhase, DrawType, OffscreenCore2d, OffscreenFlashShapeRenderPhases},
        pipeline::{
            FilterUniformBuffers, OffscreenMesh2dKey, OffscreenMesh2dPipeline,
            init_offscreen_texture_pipeline,
        },
        texture_attachment::ColorAttachment,
    },
    swf_runtime::filter::Filter,
};

#[derive(Component, Default, Clone)]
#[require(OffscreenTextureRenderGraph::new(OffscreenCore2d))]
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

    pub fn post_process_write(&self) -> PostProcessWrite<'_> {
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
        app.register_required_components::<OffscreenTexture, SyncToRenderWorld>()
            .add_plugins(ExtractComponentPlugin::<OffscreenDrawCommands>::default());

        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<SortedOffscreenTextures>()
            .init_resource::<OffscreenFlashShapeRenderPhases>()
            .init_resource::<SpecializedMeshPipelines<OffscreenMesh2dPipeline>>()
            .init_resource::<FilterUniformBuffers>()
            .add_systems(ExtractSchedule, extract_offscreen_textures)
            .add_systems(
                Render,
                (
                    sort_offscreen_textures.in_set(RenderSystems::ManageViews),
                    prepare_offscreen_view_attachments
                        .in_set(RenderSystems::ManageViews)
                        .before(prepare_offscreen_texture_view_target)
                        .after(prepare_windows),
                    prepare_offscreen_texture_view_target.in_set(RenderSystems::ManageViews),
                    prepare_offscreen_shape_filter_uniform.in_set(RenderSystems::PrepareResources),
                    prepare_offscreen_shape_bind_group.in_set(RenderSystems::PrepareBindGroups),
                    special_and_queue_shape_draw.in_set(RenderSystems::Queue),
                ),
            )
            .add_systems(RenderStartup, init_offscreen_texture_pipeline);
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
    images: Res<RenderAssets<GpuImage>>,
    offscreen_textures: Query<&ExtractedOffscreenTexture>,
    mut view_target_attachments: ResMut<ViewTargetAttachments>,
) {
    for offscreen_texture in offscreen_textures.iter() {
        let Some(target) = &offscreen_texture.target else {
            continue;
        };
        match view_target_attachments.entry(target.clone()) {
            Entry::Occupied(_) => {}
            Entry::Vacant(entry) => match target {
                NormalizedRenderTarget::Image(image_target) => {
                    let view = images
                        .get(&image_target.handle)
                        .map(|image| &image.texture_view);
                    let format = images
                        .get(&image_target.handle)
                        .map(|image| image.texture_format);

                    if let Some(attachment) = view
                        .zip(format)
                        .map(|(view, format)| OutputColorAttachment::new(view.clone(), format))
                    {
                        entry.insert(attachment);
                    }
                }
                _ => {}
            },
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

        let main_texture_format = TextureFormat::Rgba8Unorm;
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

fn prepare_offscreen_shape_filter_uniform(
    mut commands: Commands,
    query: Query<(Entity, &ExtractedOffscreenTexture)>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    mut filter_uniform_buffers: ResMut<FilterUniformBuffers>,
) {
    if query.is_empty() {
        return;
    }

    filter_uniform_buffers.clear();
    for (entity, offscreen_texture) in query.iter() {
        let size = offscreen_texture.size.as_vec2();
        let scale = offscreen_texture.scale;
        let view_matrix = Mat4::from_cols_array_2d(&[
            [2.0 * scale.x / size.x, 0.0, 0.0, 0.0],
            [0.0, -2.0 * scale.y / size.y, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ]);
        let view_offset = filter_uniform_buffers
            .view_uniform_buffer
            .push(&view_matrix);
        let filter_offsets = FilterOffsets::new(view_offset);

        commands.entity(entity).insert(filter_offsets);
    }
    // 写入缓冲区
    filter_uniform_buffers.write_buffer(&render_device, &render_queue);
}

fn prepare_offscreen_shape_bind_group(
    mut commands: Commands,
    query: Query<Entity, With<ExtractedOffscreenTexture>>,
    offscreen_mesh2d_pipeline: Res<OffscreenMesh2dPipeline>,
    render_device: Res<RenderDevice>,
    filter_uniform_buffers: Res<FilterUniformBuffers>,
) {
    if query.is_empty() {
        return;
    }
    let view_buffer = &filter_uniform_buffers.view_uniform_buffer;

    let view_bind_group = render_device.create_bind_group(
        "offscreen_main_transparent_pass_2d_bind_group",
        &offscreen_mesh2d_pipeline.view_bind_group_layout,
        &BindGroupEntries::single(view_buffer.binding().unwrap()),
    );

    commands.insert_resource(FilterBindGroup { view_bind_group });
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

#[derive(Component)]
pub struct FilterOffsets {
    pub view_offset: u32,
}
impl FilterOffsets {
    pub fn new(view_offset: u32) -> Self {
        Self { view_offset }
    }
}

#[derive(Resource)]
pub struct FilterBindGroup {
    pub view_bind_group: BindGroup,
}
