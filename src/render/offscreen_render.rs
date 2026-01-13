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
    math::{Mat4, UVec2, Vec3},
    platform::collections::{HashMap, HashSet, hash_map::Entry},
    prelude::{Deref, DerefMut, ReflectComponent},
    reflect::Reflect,
    render::{
        Extract, ExtractSchedule, Render, RenderApp, RenderSystems,
        extract_component::ExtractComponentPlugin,
        render_asset::RenderAssets,
        render_graph::{InternedRenderSubGraph, RenderSubGraph},
        render_resource::{
            BindGroup, BindGroupEntries, Extent3d, RenderPassColorAttachment, TextureDescriptor,
            TextureDimension, TextureFormat, TextureUsages, TextureView,
        },
        renderer::{RenderDevice, RenderQueue},
        sync_world::{MainEntity, RenderEntity, SyncToRenderWorld},
        texture::{GpuImage, OutputColorAttachment, TextureCache},
        view::{Msaa, PostProcessWrite, ViewTargetAttachments, prepare_windows},
    },
};

use super::filter_render::graph::OffscreenCore2d;
use crate::{
    commands::OffscreenDrawShapes,
    render::{
        filter_render::{
            BevelFilterPipeline, BevelUniform, BlurFilterPipeline, BlurUniform,
            ColorMatrixFilterPipeline, ColorMatrixUniform, FilterUniformBuffers, Filters,
            GlowFilterPipeline, GlowFilterUniform, OffscreenFlashShapeRenderPhases,
            OffscreenShapePartMesh2dPipeline, get_filter_vertex_with_double_blur,
        },
        texture_attachment::ColorAttachment,
    },
    swf_runtime::filter::Filter,
};

#[derive(Component, Default, Clone)]
#[require(OffscreenCameraRenderGraph::new(OffscreenCore2d), SyncToRenderWorld)]
pub struct OffscreenCamera {
    pub is_active: bool,
    pub order: isize,
    pub size: UVec2,
    pub target: RenderTarget,
    pub clear_color: Color,
    pub scale: Vec3,
}

#[derive(Component, Debug, Deref, DerefMut, Reflect, Clone)]
#[reflect(opaque)]
#[reflect(Component, Debug, Clone)]
pub struct OffscreenCameraRenderGraph(InternedRenderSubGraph);

impl OffscreenCameraRenderGraph {
    /// Creates a new [`OffscreenCameraRenderGraph`] from any string-like type.
    #[inline]
    pub fn new<T: RenderSubGraph>(name: T) -> Self {
        Self(name.intern())
    }
}

#[derive(Component)]
pub struct ExtractedOffscreenCamera {
    pub order: isize,
    pub size: UVec2,
    pub target: Option<NormalizedRenderTarget>,
    pub clear_color: Color,
    pub render_graph: InternedRenderSubGraph,
    pub scale: Vec3,
}

pub fn extract_offscreen_cameras(
    mut commands: Commands,
    mut render_phases: ResMut<OffscreenFlashShapeRenderPhases>,
    query: Extract<Query<(RenderEntity, &OffscreenCamera, &OffscreenCameraRenderGraph)>>,
) {
    let mut live_entities = <HashSet<MainEntity>>::new();
    for (render_entity, offscreen_camera, render_graph) in query.iter() {
        if !offscreen_camera.is_active {
            commands
                .entity(render_entity)
                .remove::<ExtractedOffscreenCamera>();
            continue;
        }
        let mut commands = commands.entity(render_entity);
        commands.insert((ExtractedOffscreenCamera {
            order: offscreen_camera.order,
            size: offscreen_camera.size,
            target: offscreen_camera.target.normalize(None),
            clear_color: offscreen_camera.clear_color,
            render_graph: render_graph.0,
            scale: offscreen_camera.scale,
        },));
        render_phases.insert_or_clear(render_entity.into());
        live_entities.insert(render_entity.into());
    }
    render_phases.retain(|k, _| live_entities.contains(k));
}

#[derive(Resource, Default, DerefMut, Deref)]
pub struct SortedOffscreenCameras(pub Vec<SortedOffscreenCamera>);

pub struct SortedOffscreenCamera {
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
    pub fn get_color_attachment(&self) -> RenderPassColorAttachment<'_> {
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
    ) -> RenderPassColorAttachment<'_> {
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

pub struct OffscreenRenderPlugin;

impl Plugin for OffscreenRenderPlugin {
    fn build(&self, app: &mut bevy::app::App) {
        app.add_plugins(ExtractComponentPlugin::<OffscreenDrawShapes>::default());

        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<SortedOffscreenCameras>()
            .init_resource::<OffscreenFlashShapeRenderPhases>()
            .init_resource::<FilterUniformBuffers>()
            .add_systems(ExtractSchedule, extract_offscreen_cameras)
            .add_systems(
                Render,
                (
                    sort_offscreen_cameras.in_set(RenderSystems::ManageViews),
                    prepare_offscreen_view_attachments
                        .in_set(RenderSystems::ManageViews)
                        .before(prepare_offscreen_view_target)
                        .after(prepare_windows),
                    prepare_offscreen_view_target.in_set(RenderSystems::ManageViews),
                    // TODO: 考虑是否移动到 filter_render 中
                    // 准备形状滤镜统一缓冲区
                    prepare_offscreen_shape_filter_uniform.in_set(RenderSystems::PrepareResources),
                    // 准备形状滤镜绑定组
                    prepare_offscreen_shape_bind_group.in_set(RenderSystems::PrepareBindGroups),
                ),
            );
    }
}

fn sort_offscreen_cameras(
    mut sorted_offscreen_cameras: ResMut<SortedOffscreenCameras>,
    mut offscreen_cameras: Query<(Entity, &mut ExtractedOffscreenCamera)>,
) {
    sorted_offscreen_cameras.clear();
    for (entity, offscreen_camera) in offscreen_cameras.iter_mut() {
        sorted_offscreen_cameras.push(SortedOffscreenCamera {
            entity,
            order: offscreen_camera.order,
        });
    }
    sorted_offscreen_cameras.sort_by_key(|k| k.order);
}

fn prepare_offscreen_view_attachments(
    images: Res<RenderAssets<GpuImage>>,
    offscreen_cameras: Query<&ExtractedOffscreenCamera>,
    mut view_target_attachments: ResMut<ViewTargetAttachments>,
) {
    for offscreen_cameras in offscreen_cameras.iter() {
        let Some(target) = &offscreen_cameras.target else {
            continue;
        };
        match view_target_attachments.entry(target.clone()) {
            Entry::Occupied(_) => {}
            Entry::Vacant(entry) => {
                if let NormalizedRenderTarget::Image(image_target) = target {
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
            }
        }
    }
}

fn prepare_offscreen_view_target(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    mut texture_cache: ResMut<TextureCache>,
    offscreen_cameras: Query<(Entity, &ExtractedOffscreenCamera)>,
    view_target_attachments: ResMut<ViewTargetAttachments>,
) {
    let mut textures = HashMap::new();
    for (entity, offscreen_camera) in offscreen_cameras.iter() {
        let (target_size, Some(target)) = (offscreen_camera.size, &offscreen_camera.target) else {
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
        let clear_color = offscreen_camera.clear_color;
        let texture_usage = TextureUsages::RENDER_ATTACHMENT
            | TextureUsages::COPY_SRC
            | TextureUsages::TEXTURE_BINDING;
        let (a, b, sample, main_texture) = textures
            .entry((offscreen_camera.target.clone(), texture_usage, msaa))
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
    query: Query<(Entity, &ExtractedOffscreenCamera, &Filters)>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    mut filter_uniform_buffers: ResMut<FilterUniformBuffers>,
) {
    if query.is_empty() {
        return;
    }

    filter_uniform_buffers.clear();
    for (entity, offscreen_cameras, filters) in query.iter() {
        let size = offscreen_cameras.size.as_vec2();
        let scale = offscreen_cameras.scale;
        let view_matrix = Mat4::from_cols_array_2d(&[
            [2.0 * scale.x / size.x, 0.0, 0.0, 0.0],
            [0.0, -2.0 * scale.y / size.y, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ]);
        let view_offset = filter_uniform_buffers.view_uniform_buffer_push(&view_matrix);
        let mut filter_offsets = FilterOffsets::new(view_offset);

        // 滤镜uniform 数据
        for filter in filters.iter() {
            match filter {
                Filter::BevelFilter(bevel_filter) => {
                    let blur_uniform =
                        BlurUniform::calculate(&bevel_filter.inner_blur_filter(), size.as_uvec2());
                    let offsets = filter_uniform_buffers.blur_uniform_buffer_extend(&blur_uniform);
                    filter_offsets.blur_offset_extend(offsets);

                    let bevel_uniform = BevelUniform::calculate(bevel_filter);
                    let distance = bevel_filter.distance.to_f32();
                    let angle = bevel_filter.angle.to_f32();
                    // TODO:
                    let _filter_vertex_with_double_blur =
                        get_filter_vertex_with_double_blur(distance, angle, size);

                    let offsets = filter_uniform_buffers.bevel_uniform_buffer_push(&bevel_uniform);
                    filter_offsets.bevel_offset_push(offsets);
                }
                Filter::BlurFilter(blur_filter) => {
                    let blur_uniform = BlurUniform::calculate(blur_filter, size.as_uvec2());
                    let offsets = filter_uniform_buffers.blur_uniform_buffer_extend(&blur_uniform);
                    filter_offsets.blur_offset_extend(offsets);
                }
                Filter::ColorMatrixFilter(color_matrix_filter) => {
                    let color_matrix_uniform =
                        ColorMatrixUniform::from_array(color_matrix_filter.matrix);
                    let offsets = filter_uniform_buffers
                        .color_matrix_uniform_buffer_push(&color_matrix_uniform);
                    filter_offsets.color_offset_push(offsets);
                }
                Filter::GlowFilter(glow_filter) => {
                    let blur_uniform =
                        BlurUniform::calculate(&glow_filter.inner_blur_filter(), size.as_uvec2());
                    let offsets = filter_uniform_buffers.blur_uniform_buffer_extend(&blur_uniform);
                    filter_offsets.blur_offset_extend(offsets);

                    let glow_uniform = GlowFilterUniform::calculate(glow_filter);
                    let offset = filter_uniform_buffers.glow_uniform_buffer_push(&glow_uniform);
                    filter_offsets.glow_offset_push(offset);
                }
                _ => {}
            }
        }
        commands.entity(entity).insert(filter_offsets);
    }
    // 写入缓冲区
    filter_uniform_buffers.write_buffer(&render_device, &render_queue);
}

fn prepare_offscreen_shape_bind_group(
    mut commands: Commands,
    query: Query<Entity, With<ExtractedOffscreenCamera>>,
    offscreen_shape_part_mesh2d_pipeline: Res<OffscreenShapePartMesh2dPipeline>,
    color_matrix_pipeline: Res<ColorMatrixFilterPipeline>,
    blur_pipeline: Res<BlurFilterPipeline>,
    glow_pipeline: Res<GlowFilterPipeline>,
    bevel_pipeline: Res<BevelFilterPipeline>,
    render_device: Res<RenderDevice>,
    filter_uniform_buffers: Res<FilterUniformBuffers>,
) {
    if query.is_empty() {
        return;
    }
    let view_buffer = &filter_uniform_buffers.view_uniform_buffer;
    let view_bind_group = render_device.create_bind_group(
        "offscreen_main_transparent_pass_2d_bind_group",
        &offscreen_shape_part_mesh2d_pipeline.view_bind_group_layout,
        &BindGroupEntries::single(view_buffer.binding().unwrap()),
    );

    let transform_buffer = &filter_uniform_buffers.transform_uniform_buffer;

    let transform_bind_group = render_device.create_bind_group(
        "offscreen_main_transparent_pass_2d_transform_bind_group",
        &offscreen_shape_part_mesh2d_pipeline.transform_bind_group_layout,
        &BindGroupEntries::single(transform_buffer.binding().unwrap()),
    );

    let color_matrix_buffer = &filter_uniform_buffers.color_matrix_uniform_buffer;
    let color_matrix_bind_group = if !color_matrix_buffer.is_empty() {
        let color_matrix_bind_group = render_device.create_bind_group(
            "color_matrix_filter_bind_group",
            &color_matrix_pipeline.layout,
            &BindGroupEntries::single(color_matrix_buffer.binding().unwrap()),
        );
        Some(color_matrix_bind_group)
    } else {
        None
    };

    let blur_buffer = &filter_uniform_buffers.blur_uniform_buffer;
    let blur_bind_group = if !blur_buffer.is_empty() {
        let blur_bind_group = render_device.create_bind_group(
            "blur_filter_bind_group",
            &blur_pipeline.layout,
            &BindGroupEntries::single(blur_buffer.binding().unwrap()),
        );
        Some(blur_bind_group)
    } else {
        None
    };

    let glow_buffer = &filter_uniform_buffers.glow_uniform_buffer;
    let glow_bind_group = if !glow_buffer.is_empty() {
        let glow_bind_group = render_device.create_bind_group(
            "glow_filter_bind_group",
            &glow_pipeline.layout,
            &BindGroupEntries::single(glow_buffer.binding().unwrap()),
        );
        Some(glow_bind_group)
    } else {
        None
    };

    let bevel_buffer = &filter_uniform_buffers.bevel_uniform_buffer;
    let bevel_bind_group = if !bevel_buffer.is_empty() {
        let bevel_bind_group = render_device.create_bind_group(
            "bevel_filter_bind_group",
            &bevel_pipeline.layout,
            &BindGroupEntries::single(bevel_buffer.binding().unwrap()),
        );
        Some(bevel_bind_group)
    } else {
        None
    };

    commands.insert_resource(FilterBindGroup {
        view_bind_group,
        transform_bind_group,
        color_matrix_bind_group,
        blur_bind_group,
        glow_bind_group,
        bevel_bind_group,
    });
}

#[derive(Component)]
pub struct FilterOffsets {
    pub view_offset: u32,
    pub color_offsets: Vec<u32>,
    pub blur_offsets: Vec<u32>,
    pub glow_offsets: Vec<u32>,
    pub bevel_offsets: Vec<u32>,
}
impl FilterOffsets {
    fn new(view_offset: u32) -> Self {
        Self {
            view_offset,
            color_offsets: Vec::new(),
            blur_offsets: Vec::new(),
            glow_offsets: Vec::new(),
            bevel_offsets: Vec::new(),
        }
    }

    fn color_offset_push(&mut self, offset: u32) {
        self.color_offsets.push(offset);
    }

    fn blur_offset_extend(&mut self, offsets: Vec<u32>) {
        self.blur_offsets.extend(offsets);
    }

    fn glow_offset_push(&mut self, offset: u32) {
        self.glow_offsets.push(offset);
    }

    fn bevel_offset_push(&mut self, offset: u32) {
        self.bevel_offsets.push(offset);
    }
}

#[derive(Resource)]
pub struct FilterBindGroup {
    pub view_bind_group: BindGroup,
    pub transform_bind_group: BindGroup,

    pub color_matrix_bind_group: Option<BindGroup>,
    pub blur_bind_group: Option<BindGroup>,
    pub glow_bind_group: Option<BindGroup>,
    pub bevel_bind_group: Option<BindGroup>,
}
