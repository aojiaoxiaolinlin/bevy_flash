pub mod graph;

use std::borrow::Cow;

use bevy::{
    app::Plugin,
    asset::{
        AssetId, AssetServer, Handle, embedded_asset, load_embedded_asset, load_internal_asset,
        uuid_handle,
    },
    core_pipeline::{
        FullscreenShader,
        blit::{BlitPipeline, BlitPipelineKey},
    },
    ecs::{
        component::Component,
        entity::Entity,
        query::With,
        resource::Resource,
        schedule::IntoScheduleConfigs,
        system::{Commands, Query, Res, ResMut},
    },
    log::error,
    math::{Mat4, UVec2, Vec2, Vec4},
    mesh::{Mesh, MeshVertexBufferLayoutRef, PrimitiveTopology, VertexBufferLayout, VertexFormat},
    platform::collections::{HashSet, hash_map::Entry},
    prelude::{Deref, DerefMut},
    render::{
        Render, RenderApp, RenderStartup, RenderSystems,
        extract_component::{ExtractComponent, ExtractComponentPlugin},
        mesh::RenderMesh,
        render_asset::RenderAssets,
        render_resource::{
            AsBindGroup, BindGroupLayoutDescriptor, BindGroupLayoutEntries, BlendComponent,
            BlendFactor, BlendOperation, BlendState, BufferUsages, BufferVec,
            CachedRenderPipelineId, ColorTargetState, ColorWrites, DynamicUniformBuffer,
            FragmentState, FrontFace, MultisampleState, PipelineCache, PolygonMode, PrimitiveState,
            RenderPipelineDescriptor, Sampler, SamplerBindingType, SamplerDescriptor, ShaderStages,
            ShaderType, SpecializedMeshPipeline, SpecializedMeshPipelines,
            SpecializedRenderPipelines, TextureFormat, TextureSampleType, VertexState,
            VertexStepMode,
            binding_types::{sampler, texture_2d, uniform_buffer},
        },
        renderer::{RenderDevice, RenderQueue},
        sync_world::{MainEntity, MainEntityHashMap},
        view::Msaa,
    },
    shader::{Shader, load_shader_library},
    utils::default,
};
use bytemuck::{Pod, Zeroable};

use crate::{
    commands::{OffscreenDrawShapes, ShapeCommand},
    render::{
        blend_pipeline::{BlendMode, TrivialBlend},
        material::{BitmapMaterial, GradientMaterial, SwfMaterial, TransformUniform},
        offscreen_render::{ExtractedOffscreenCamera, ViewTarget},
    },
    swf_runtime::filter::Filter,
};

use self::graph::SwfFilterRenderGraphPlugin;

pub const OFFSCREEN_MESH2D_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("f1e2d3c4-b5a6-4978-8c9d-0e1f2a3b4c5d");

pub const OFFSCREEN_MESH2D_GRADIENT_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("a1b2c3d4-e5f6-4789-8a9b-0c1d2e3f4a5b");

pub const OFFSCREEN_MESH2D_BITMAP_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("e3f4a5b6-c7d8-4e9f-0a1b-2c3d4e5f6a7b");

pub struct SwfFilterRenderPlugin;

impl Plugin for SwfFilterRenderPlugin {
    fn build(&self, app: &mut bevy::app::App) {
        load_shader_library!(app, "shaders/offscreen_mesh2d/offscreen_common.wgsl");
        embedded_asset!(app, "shaders/filters/blur.wgsl");
        embedded_asset!(app, "shaders/filters/color_matrix.wgsl");
        embedded_asset!(app, "shaders/filters/glow.wgsl");
        embedded_asset!(app, "shaders/filters/bevel.wgsl");
        embedded_asset!(app, "shaders/filters/displacement_map.wgsl");

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

        app.add_plugins(SwfFilterRenderGraphPlugin)
            .add_plugins(ExtractComponentPlugin::<Filters>::default());

        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };

        render_app
            .init_resource::<SpecializedMeshPipelines<OffscreenShapePartMesh2dPipeline>>()
            .add_systems(
                RenderStartup,
                (
                    init_offscreen_texture_pipeline,
                    init_source_texture_layout,
                    init_blur_filter_pipeline,
                    init_color_matrix_filter_pipeline,
                    init_glow_filter_pipeline,
                    init_bevel_filter_pipeline,
                )
                    .chain(),
            )
            .add_systems(
                Render,
                (
                    special_and_queue_shape_draw.in_set(RenderSystems::Queue),
                    prepare_offscreen_view_upscaling_pipelines
                        .in_set(RenderSystems::Prepare)
                        .ambiguous_with_all(),
                ),
            );
    }
}

#[derive(Component, Deref, DerefMut, Default, Clone, ExtractComponent)]
pub struct Filters(pub Vec<Filter>);

#[derive(Component)]
pub struct ViewUpscalingPipeline(CachedRenderPipelineId);

pub fn prepare_offscreen_view_upscaling_pipelines(
    mut commands: Commands,
    mut pipeline_cache: ResMut<PipelineCache>,
    mut pipelines: ResMut<SpecializedRenderPipelines<BlitPipeline>>,
    blit_pipeline: Res<BlitPipeline>,
    view_targets: Query<(Entity, &ViewTarget)>,
) {
    let mut output_textures = <HashSet<_>>::default();
    for (entity, view_target) in view_targets.iter() {
        let out_texture_id = view_target.out_texture().id();
        let already_seen = output_textures.contains(&out_texture_id);
        output_textures.insert(out_texture_id);
        let blend_state = if already_seen {
            Some(BlendState::ALPHA_BLENDING)
        } else {
            output_textures.insert(out_texture_id);
            None
        };

        let key = BlitPipelineKey {
            texture_format: view_target.out_texture_view_format(),
            blend_state,
            samples: 1,
        };
        let pipeline = pipelines.specialize(&pipeline_cache, &blit_pipeline, key);

        // Ensure the pipeline is loaded before continuing the frame to prevent frames without any GPU work submitted
        pipeline_cache.block_on_render_pipeline(pipeline);

        commands
            .entity(entity)
            .insert(ViewUpscalingPipeline(pipeline));
    }
}

#[derive(Clone, Debug)]
pub enum DrawType {
    Color,
    Gradient(AssetId<GradientMaterial>),
    Bitmap(AssetId<BitmapMaterial>),
}

#[derive(Clone, Debug)]
pub struct PartMesh {
    pub draw_type: DrawType,
    pub mesh_asset_id: AssetId<Mesh>,
    pub pipeline_id: CachedRenderPipelineId,
    pub transform_offset: u32,
}

impl From<&SwfMaterial> for DrawType {
    fn from(value: &SwfMaterial) -> Self {
        match value {
            SwfMaterial::Color(_) => DrawType::Color,
            SwfMaterial::Gradient(gradient) => DrawType::Gradient(gradient.id()),
            SwfMaterial::Bitmap(bitmap) => DrawType::Bitmap(bitmap.id()),
        }
    }
}

#[derive(Resource, Deref, DerefMut, Default)]
pub struct OffscreenFlashShapeRenderPhases(pub MainEntityHashMap<Vec<PartMesh>>);

impl OffscreenFlashShapeRenderPhases {
    pub fn insert_or_clear(&mut self, entity: MainEntity) {
        match self.entry(entity) {
            Entry::Occupied(mut entry) => entry.get_mut().clear(),
            Entry::Vacant(entry) => {
                entry.insert(default());
            }
        }
    }
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    #[repr(transparent)]
    pub struct OffscreenMesh2dKey:u16 {
        const NONE     = 0;

        const BLEND_ADD                         = 1 << 0;  // Additive blending
        const BLEND_SUBTRACT                    = 1 << 1;  // Subtractive blending
        const BLEND_SCREEN                      = 1 << 2;  // Screen blending
        const BLEND_LIGHTEN                     = 1 << 3;  // Lighten blending
        const BLEND_DARKEN                      = 1 << 4;  // Darken blending
        const BLEND_MULTIPLY                    = 1 << 5;  // Multiply blending
        const BLEND_ALPHA                       = 1 << 6;  // Alpha blending


        const COLOR    = 1 << 7;
        const GRADIENT = 1 << 8;
        const BITMAP   = 1 << 9;
        const MSAA     = 1 << 10;
    }
}

impl From<&BlendMode> for OffscreenMesh2dKey {
    fn from(value: &BlendMode) -> Self {
        match value {
            BlendMode::Trivial(trivial_blend) => match trivial_blend {
                TrivialBlend::Add => OffscreenMesh2dKey::BLEND_ADD,
                TrivialBlend::Subtract => OffscreenMesh2dKey::BLEND_SUBTRACT,
                TrivialBlend::Screen => OffscreenMesh2dKey::BLEND_SCREEN,
                TrivialBlend::Lighten => OffscreenMesh2dKey::BLEND_LIGHTEN,
                TrivialBlend::Darken => OffscreenMesh2dKey::BLEND_DARKEN,
                TrivialBlend::Multiply => OffscreenMesh2dKey::BLEND_MULTIPLY,
                TrivialBlend::Normal => OffscreenMesh2dKey::BLEND_ALPHA,
            },
            _ => OffscreenMesh2dKey::BLEND_ALPHA,
        }
    }
}

#[derive(Resource, Clone)]
pub struct OffscreenShapePartMesh2dPipeline {
    pub view_bind_group_layout: BindGroupLayoutDescriptor,
    pub transform_bind_group_layout: BindGroupLayoutDescriptor,

    pub gradient_bind_group_layout: BindGroupLayoutDescriptor,
    pub bitmap_bind_group_layout: BindGroupLayoutDescriptor,
    /// 某些特殊的位图填充好像需要特殊处理，这个暂时保留
    #[expect(unused)]
    pub sampler: Sampler,
}

pub fn init_offscreen_texture_pipeline(mut commands: Commands, render_device: Res<RenderDevice>) {
    let view_bind_group_layout = BindGroupLayoutDescriptor::new(
        "纹理变换矩阵布局",
        &BindGroupLayoutEntries::single(ShaderStages::VERTEX, uniform_buffer::<Mat4>(true)),
    );

    let transform_bind_group_layout = BindGroupLayoutDescriptor::new(
        "变换矩阵布局",
        &BindGroupLayoutEntries::single(
            ShaderStages::VERTEX_FRAGMENT,
            uniform_buffer::<TransformUniform>(true),
        ),
    );

    let gradient_bind_group_layout = GradientMaterial::bind_group_layout_descriptor(&render_device);
    let bitmap_bind_group_layout = BitmapMaterial::bind_group_layout_descriptor(&render_device);

    let sampler = render_device.create_sampler(&SamplerDescriptor::default());

    commands.insert_resource(OffscreenShapePartMesh2dPipeline {
        view_bind_group_layout,
        transform_bind_group_layout,
        gradient_bind_group_layout,
        bitmap_bind_group_layout,
        sampler,
    });
}

impl SpecializedMeshPipeline for OffscreenShapePartMesh2dPipeline {
    type Key = OffscreenMesh2dKey;

    fn specialize(
        &self,
        key: Self::Key,
        layout: &bevy::mesh::MeshVertexBufferLayoutRef,
    ) -> Result<RenderPipelineDescriptor, bevy::render::render_resource::SpecializedMeshPipelineError>
    {
        let mut vertex_attributes = Vec::new();

        if layout.0.contains(Mesh::ATTRIBUTE_POSITION) {
            vertex_attributes.push(Mesh::ATTRIBUTE_POSITION.at_shader_location(0));
        }

        if layout.0.contains(Mesh::ATTRIBUTE_UV_0) {
            vertex_attributes.push(Mesh::ATTRIBUTE_UV_0.at_shader_location(1));
        }

        if layout.0.contains(Mesh::ATTRIBUTE_COLOR) {
            vertex_attributes.push(Mesh::ATTRIBUTE_COLOR.at_shader_location(2));
        }

        let vertex_buffer_layout = layout.0.get_layout(&vertex_attributes)?;
        let format = TextureFormat::Rgba8Unorm;

        let label;
        let bind_group_layout = if key.contains(OffscreenMesh2dKey::COLOR) {
            label = "color_offscreen_mesh2d";
            vec![
                self.view_bind_group_layout.clone(),
                self.transform_bind_group_layout.clone(),
            ]
        } else if key.contains(OffscreenMesh2dKey::GRADIENT) {
            label = "gradient_offscreen_mesh2d";
            vec![
                self.view_bind_group_layout.clone(),
                self.transform_bind_group_layout.clone(),
                self.gradient_bind_group_layout.clone(),
            ]
        } else {
            label = "bitmap_offscreen_mesh2d";
            vec![
                self.view_bind_group_layout.clone(),
                self.transform_bind_group_layout.clone(),
                self.bitmap_bind_group_layout.clone(),
            ]
        };

        let shader = if key.contains(OffscreenMesh2dKey::COLOR) {
            OFFSCREEN_MESH2D_SHADER_HANDLE
        } else if key.contains(OffscreenMesh2dKey::GRADIENT) {
            OFFSCREEN_MESH2D_GRADIENT_SHADER_HANDLE
        } else if key.contains(OffscreenMesh2dKey::BITMAP) {
            OFFSCREEN_MESH2D_BITMAP_SHADER_HANDLE
        } else {
            OFFSCREEN_MESH2D_SHADER_HANDLE
        };

        let blend = if key.contains(OffscreenMesh2dKey::BLEND_ADD) {
            Some(BlendState {
                color: BlendComponent {
                    src_factor: BlendFactor::One,
                    dst_factor: BlendFactor::One,
                    operation: BlendOperation::Add,
                },
                alpha: BlendComponent::OVER,
            })
        } else if key.contains(OffscreenMesh2dKey::BLEND_MULTIPLY) {
            Some(BlendState {
                color: BlendComponent {
                    src_factor: BlendFactor::Dst,
                    dst_factor: BlendFactor::OneMinusSrcAlpha,
                    operation: BlendOperation::Add,
                },
                alpha: BlendComponent::OVER,
            })
        } else if key.contains(OffscreenMesh2dKey::BLEND_SUBTRACT) {
            Some(BlendState {
                color: BlendComponent {
                    src_factor: BlendFactor::One,
                    dst_factor: BlendFactor::One,
                    operation: BlendOperation::ReverseSubtract,
                },
                alpha: BlendComponent::OVER,
            })
        } else if key.contains(OffscreenMesh2dKey::BLEND_SCREEN) {
            Some(BlendState {
                color: BlendComponent {
                    src_factor: BlendFactor::One,
                    dst_factor: BlendFactor::OneMinusSrc,
                    operation: BlendOperation::Add,
                },
                alpha: BlendComponent::OVER,
            })
        } else if key.contains(OffscreenMesh2dKey::BLEND_LIGHTEN) {
            Some(BlendState {
                color: BlendComponent {
                    src_factor: BlendFactor::One,
                    dst_factor: BlendFactor::One,
                    operation: BlendOperation::Max,
                },
                alpha: BlendComponent::OVER,
            })
        } else if key.contains(OffscreenMesh2dKey::BLEND_DARKEN) {
            Some(BlendState {
                color: BlendComponent {
                    src_factor: BlendFactor::One,
                    dst_factor: BlendFactor::One,
                    operation: BlendOperation::Min,
                },
                alpha: BlendComponent::OVER,
            })
        } else {
            Some(BlendState::PREMULTIPLIED_ALPHA_BLENDING)
        };

        Ok(RenderPipelineDescriptor {
            label: Some(label.into()),
            layout: bind_group_layout,
            push_constant_ranges: vec![],
            vertex: VertexState {
                shader: shader.clone(),
                shader_defs: vec![],
                entry_point: Some("vertex".into()),
                buffers: vec![vertex_buffer_layout],
            },
            primitive: PrimitiveState {
                topology: PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: MultisampleState {
                count: Msaa::default().samples(),
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            fragment: Some(FragmentState {
                shader,
                shader_defs: vec![],
                entry_point: Some("fragment".into()),
                targets: vec![Some(ColorTargetState {
                    format,
                    blend,
                    write_mask: ColorWrites::ALL,
                })],
            }),
            zero_initialize_workgroup_memory: false,
        })
    }
}

#[derive(Resource)]
pub struct SourceTextureLayout {
    source_layout: BindGroupLayoutDescriptor,

    blur_texture_layout: BindGroupLayoutDescriptor,
}

fn init_source_texture_layout(mut commands: Commands) {
    let source_layout = BindGroupLayoutDescriptor::new(
        "source_texture_bind_group_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
            ),
        ),
    );

    let blur_texture_layout = BindGroupLayoutDescriptor::new(
        "blur_texture_bind_group_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (texture_2d(TextureSampleType::Float { filterable: true }),),
        ),
    );

    commands.insert_resource(SourceTextureLayout {
        source_layout,
        blur_texture_layout,
    });
}

/// 模糊滤镜
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, ShaderType, Pod, Zeroable, PartialEq)]
pub struct BlurUniform {
    pub direction: Vec2,
    pub full_size: f32,
    pub m: f32,
    pub m2: f32,
    pub first_weight: f32,
    pub last_offset: f32,
    pub last_weight: f32,
}
impl BlurUniform {
    pub(crate) fn calculate(blur_filter: &swf::BlurFilter, size: UVec2) -> Vec<BlurUniform> {
        let mut uniforms = Vec::new();

        let width = size.x as f32;
        let height = size.y as f32;
        for _ in 0..(blur_filter.num_passes() as usize) {
            for i in 0..2 {
                let horizontal = i % 2 == 0;
                let strength = if horizontal {
                    blur_filter.blur_x.to_f32()
                } else {
                    blur_filter.blur_y.to_f32()
                };
                let full_size = strength.min(255.0);
                if full_size <= 1.0 {
                    continue;
                }
                let radius = (full_size - 1.0) / 2.0;
                let m = radius.ceil() - 1.0;
                let alpha = ((radius - m) * 255.0).floor() / 255.0;
                let last_offset = 1.0 / ((1.0 / alpha) + 1.0);
                let last_weight = alpha + 1.0;

                let uniform = BlurUniform {
                    direction: if horizontal {
                        Vec2::from_array([1.0 / width, 0.0])
                    } else {
                        Vec2::from_array([0.0, 1.0 / height])
                    },
                    full_size,
                    m,
                    m2: m * 2.0,
                    first_weight: alpha,
                    last_offset,
                    last_weight,
                };
                uniforms.push(uniform);
            }
        }

        uniforms
    }
}

/// 颜色矩阵滤镜
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, ShaderType, Pod, Zeroable, PartialEq)]
pub struct ColorMatrixUniform {
    pub matrix: [Vec4; 5],
}
impl ColorMatrixUniform {
    pub(crate) fn from_array(matrix: [f32; 20]) -> ColorMatrixUniform {
        Self {
            matrix: [
                Vec4::from_slice(&matrix[0..4]),
                Vec4::from_slice(&matrix[4..8]),
                Vec4::from_slice(&matrix[8..12]),
                Vec4::from_slice(&matrix[12..16]),
                Vec4::from_slice(&matrix[16..20]),
            ],
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, ShaderType, Pod, Zeroable, PartialEq)]
pub struct GlowFilterUniform {
    pub color: Vec4,
    pub strength: f32,
    pub inner: u32,            // a wasteful bool, but we need to be aligned anyway
    pub knockout: u32,         // a wasteful bool, but we need to be aligned anyway
    pub composite_source: u32, // undocumented flash feature, another bool
}
impl GlowFilterUniform {
    pub(crate) fn calculate(glow_filter: &swf::GlowFilter) -> Self {
        Self {
            color: Vec4::from_slice(&[
                f32::from(glow_filter.color.r) / 255.0,
                f32::from(glow_filter.color.g) / 255.0,
                f32::from(glow_filter.color.b) / 255.0,
                f32::from(glow_filter.color.a) / 255.0,
            ]),
            strength: glow_filter.strength.to_f32(),
            inner: if glow_filter.is_inner() { 1 } else { 0 },
            knockout: if glow_filter.is_knockout() { 1 } else { 0 },
            composite_source: if glow_filter.composite_source() { 1 } else { 0 },
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, ShaderType, Pod, Zeroable, PartialEq)]
pub struct BevelUniform {
    pub highlight_color: Vec4,
    pub shadow_color: Vec4,
    pub strength: f32,
    pub bevel_type: u32,       // 0 outer, 1 inner, 2 full
    pub knockout: u32,         // a wasteful bool, but we need to be aligned anyway
    pub composite_source: u32, // undocumented flash feature, another bool
}

impl BevelUniform {
    pub(crate) fn calculate(bevel_filter: &swf::BevelFilter) -> Self {
        let mut highlight_color = Vec4::from_array([
            f32::from(bevel_filter.highlight_color.r) / 255.0,
            f32::from(bevel_filter.highlight_color.g) / 255.0,
            f32::from(bevel_filter.highlight_color.b) / 255.0,
            f32::from(bevel_filter.highlight_color.a) / 255.0,
        ]);
        highlight_color[0] *= highlight_color[3];
        highlight_color[1] *= highlight_color[3];
        highlight_color[2] *= highlight_color[3];
        let mut shadow_color = Vec4::from_array([
            f32::from(bevel_filter.shadow_color.r) / 255.0,
            f32::from(bevel_filter.shadow_color.g) / 255.0,
            f32::from(bevel_filter.shadow_color.b) / 255.0,
            f32::from(bevel_filter.shadow_color.a) / 255.0,
        ]);
        shadow_color[0] *= shadow_color[3];
        shadow_color[1] *= shadow_color[3];
        shadow_color[2] *= shadow_color[3];
        Self {
            highlight_color,
            shadow_color,
            strength: bevel_filter.strength.to_f32(),
            bevel_type: if bevel_filter.is_on_top() {
                2
            } else if bevel_filter.is_inner() {
                1
            } else {
                0
            },
            knockout: if bevel_filter.is_knockout() { 1 } else { 0 },
            composite_source: 1,
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, ShaderType, Pod, Zeroable, PartialEq)]
pub struct FilterVertexWithDoubleBlur {
    pub position: [f32; 2],
    pub source_uv: [f32; 2],
    pub blur_uv_left: [f32; 2],
    pub blur_uv_right: [f32; 2],
}

#[derive(Resource)]
pub struct BlurFilterPipeline {
    pub layout: BindGroupLayoutDescriptor,
    pub sampler: Sampler,
    pub pipeline_id: CachedRenderPipelineId,
}

pub(crate) fn init_blur_filter_pipeline(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    pipeline_cache: Res<PipelineCache>,
    fullscreen_shader: Res<FullscreenShader>,
    assert_server: Res<AssetServer>,
    source_texture_layout: Res<SourceTextureLayout>,
) {
    let layout = BindGroupLayoutDescriptor::new(
        "blur_filter_bind_group_layout",
        &BindGroupLayoutEntries::single(
            ShaderStages::FRAGMENT,
            uniform_buffer::<BlurUniform>(true),
        ),
    );

    let sampler = render_device.create_sampler(&SamplerDescriptor::default());

    let descriptor = RenderPipelineDescriptor {
        label: Some(Cow::from("blur_filter_render_pipeline")),
        layout: vec![source_texture_layout.source_layout.clone(), layout.clone()],
        push_constant_ranges: vec![],
        vertex: fullscreen_shader.to_vertex_state(),
        primitive: PrimitiveState::default(),
        depth_stencil: None,
        multisample: MultisampleState::default(),
        fragment: Some(FragmentState {
            shader: load_embedded_asset!(assert_server.as_ref(), "shaders/filters/blur.wgsl"),
            shader_defs: vec![],
            entry_point: Some("fragment".into()),
            targets: vec![Some(ColorTargetState {
                format: TextureFormat::Rgba8Unorm,
                blend: None,
                write_mask: ColorWrites::ALL,
            })],
        }),
        zero_initialize_workgroup_memory: false,
    };

    let pipeline_id = pipeline_cache.queue_render_pipeline(descriptor);

    commands.insert_resource(BlurFilterPipeline {
        layout,
        sampler,
        pipeline_id,
    });
}

#[derive(Resource)]
pub struct ColorMatrixFilterPipeline {
    pub layout: BindGroupLayoutDescriptor,
    pub sampler: Sampler,
    pub pipeline_id: CachedRenderPipelineId,
}

pub(crate) fn init_color_matrix_filter_pipeline(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    pipeline_cache: Res<PipelineCache>,
    fullscreen_shader: Res<FullscreenShader>,
    assert_server: Res<AssetServer>,
    source_texture_layout: Res<SourceTextureLayout>,
) {
    let layout = BindGroupLayoutDescriptor::new(
        "color_matrix_bind_group_layout",
        &BindGroupLayoutEntries::single(
            ShaderStages::FRAGMENT,
            uniform_buffer::<ColorMatrixUniform>(true),
        ),
    );
    let sampler = render_device.create_sampler(&SamplerDescriptor::default());

    let descriptor = RenderPipelineDescriptor {
        label: Some(Cow::from("color_matrix_filter_render_pipeline")),
        layout: vec![source_texture_layout.source_layout.clone(), layout.clone()],
        push_constant_ranges: vec![],
        vertex: fullscreen_shader.to_vertex_state(),
        primitive: PrimitiveState::default(),
        depth_stencil: None,
        multisample: MultisampleState::default(),
        fragment: Some(FragmentState {
            shader: load_embedded_asset!(
                assert_server.as_ref(),
                "shaders/filters/color_matrix.wgsl"
            ),
            shader_defs: vec![],
            entry_point: Some("fragment".into()),
            targets: vec![Some(ColorTargetState {
                format: TextureFormat::Rgba8Unorm,
                blend: None,
                write_mask: ColorWrites::ALL,
            })],
        }),
        zero_initialize_workgroup_memory: false,
    };

    let pipeline_id = pipeline_cache.queue_render_pipeline(descriptor);

    commands.insert_resource(ColorMatrixFilterPipeline {
        layout,
        sampler,
        pipeline_id,
    });
}

#[derive(Resource)]
pub struct GlowFilterPipeline {
    pub layout: BindGroupLayoutDescriptor,
    pub sampler: Sampler,
    pub pipeline_id: CachedRenderPipelineId,
}

pub(crate) fn init_glow_filter_pipeline(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    pipeline_cache: Res<PipelineCache>,
    fullscreen_shader: Res<FullscreenShader>,
    assert_server: Res<AssetServer>,
    source_texture_layout: Res<SourceTextureLayout>,
) {
    let layout = BindGroupLayoutDescriptor::new(
        "glow_filter_bind_group_layout",
        &BindGroupLayoutEntries::single(
            ShaderStages::FRAGMENT,
            uniform_buffer::<GlowFilterUniform>(true),
        ),
    );
    let sampler = render_device.create_sampler(&SamplerDescriptor::default());

    let descriptor = RenderPipelineDescriptor {
        label: Some(Cow::from("glow_filter_filter_render_pipeline")),
        layout: vec![
            source_texture_layout.source_layout.clone(),
            source_texture_layout.blur_texture_layout.clone(),
            layout.clone(),
        ],
        push_constant_ranges: vec![],
        vertex: fullscreen_shader.to_vertex_state(),
        primitive: PrimitiveState::default(),
        depth_stencil: None,
        multisample: MultisampleState::default(),
        fragment: Some(FragmentState {
            shader: load_embedded_asset!(assert_server.as_ref(), "shaders/filters/glow.wgsl"),
            shader_defs: vec![],
            entry_point: Some("fragment".into()),
            targets: vec![Some(ColorTargetState {
                format: TextureFormat::Rgba8Unorm,
                blend: None,
                write_mask: ColorWrites::ALL,
            })],
        }),
        zero_initialize_workgroup_memory: false,
    };

    let pipeline_id = pipeline_cache.queue_render_pipeline(descriptor);

    commands.insert_resource(GlowFilterPipeline {
        layout,
        sampler,
        pipeline_id,
    });
}

#[derive(Resource)]
pub struct BevelFilterPipeline {
    pub layout: BindGroupLayoutDescriptor,
    pub sampler: Sampler,
    pub pipeline_id: CachedRenderPipelineId,
}

pub(crate) fn init_bevel_filter_pipeline(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    pipeline_cache: Res<PipelineCache>,
    assert_server: Res<AssetServer>,
    source_texture_layout: Res<SourceTextureLayout>,
) {
    let layout = BindGroupLayoutDescriptor::new(
        "glow_filter_bind_group_layout",
        &BindGroupLayoutEntries::single(
            ShaderStages::FRAGMENT,
            uniform_buffer::<BevelUniform>(true),
        ),
    );
    let sampler = render_device.create_sampler(&SamplerDescriptor::default());
    let shader = load_embedded_asset!(assert_server.as_ref(), "shaders/filters/bevel.wgsl");

    let descriptor = RenderPipelineDescriptor {
        label: Some(Cow::from("bevel_filter_render_pipeline")),
        layout: vec![
            source_texture_layout.source_layout.clone(),
            source_texture_layout.blur_texture_layout.clone(),
            layout.clone(),
        ],
        push_constant_ranges: vec![],
        vertex: VertexState {
            shader: shader.clone(),
            shader_defs: vec![],
            entry_point: Some("vertex".into()),
            buffers: vec![VertexBufferLayout::from_vertex_formats(
                VertexStepMode::Vertex,
                vec![
                    VertexFormat::Float32x2,
                    VertexFormat::Float32x2,
                    VertexFormat::Float32x2,
                    VertexFormat::Float32x2,
                ],
            )],
        },
        primitive: PrimitiveState::default(),
        depth_stencil: None,
        multisample: MultisampleState::default(),
        fragment: Some(FragmentState {
            shader,
            shader_defs: vec![],
            entry_point: Some("fragment".into()),
            targets: vec![Some(ColorTargetState {
                format: TextureFormat::Rgba8Unorm,
                blend: None,
                write_mask: ColorWrites::ALL,
            })],
        }),
        zero_initialize_workgroup_memory: false,
    };

    let pipeline_id = pipeline_cache.queue_render_pipeline(descriptor);

    commands.insert_resource(BevelFilterPipeline {
        layout,
        sampler,
        pipeline_id,
    });
}

pub fn special_and_queue_shape_draw(
    offscreen_shape_part_mesh2d_pipeline: Res<OffscreenShapePartMesh2dPipeline>,
    mut pipelines: ResMut<SpecializedMeshPipelines<OffscreenShapePartMesh2dPipeline>>,
    pipeline_cache: Res<PipelineCache>,
    query: Query<(Entity, &OffscreenDrawShapes), With<ExtractedOffscreenCamera>>,
    render_meshes: Res<RenderAssets<RenderMesh>>,
    mut render_phases: ResMut<OffscreenFlashShapeRenderPhases>,
    mut filter_uniform_buffers: ResMut<FilterUniformBuffers>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
) {
    let mut get_pipeline_id = |mesh_layout: &MeshVertexBufferLayoutRef,
                               mesh_key: OffscreenMesh2dKey| {
        let pipeline_id = pipelines.specialize(
            &pipeline_cache,
            &offscreen_shape_part_mesh2d_pipeline,
            mesh_key,
            mesh_layout,
        );
        let pipeline_id = match pipeline_id {
            Ok(id) => id,
            Err(err) => {
                error!("{}", err);
                return None;
            }
        };
        Some(pipeline_id)
    };

    let size = query.iter().map(|(_, commands)| commands.len()).sum();

    let Some(mut transform_uniform_buffer_writer) = filter_uniform_buffers
        .transform_uniform_buffer
        .get_writer(size, &render_device, &render_queue)
    else {
        return;
    };

    for (main_entity, offscreen_draw_commands) in query.iter() {
        let main_entity = MainEntity::from(main_entity);
        let Some(render_phase) = render_phases.get_mut(&main_entity) else {
            continue;
        };
        for draw_command in offscreen_draw_commands.iter() {
            match draw_command {
                ShapeCommand::RenderShape {
                    draw_shape,
                    transform,
                    blend_mode,
                } => {
                    let transform_offset =
                        transform_uniform_buffer_writer.write(&TransformUniform::from(*transform));
                    for mesh_draw in draw_shape.iter() {
                        let Some(mesh) = render_meshes.get(mesh_draw.mesh.id()) else {
                            continue;
                        };
                        let mesh_key = OffscreenMesh2dKey::from(blend_mode);
                        let mesh_key = mesh_key
                            | match &mesh_draw.material {
                                SwfMaterial::Color(_) => OffscreenMesh2dKey::COLOR,
                                SwfMaterial::Gradient(_) => OffscreenMesh2dKey::GRADIENT,
                                SwfMaterial::Bitmap(_) => OffscreenMesh2dKey::BITMAP,
                            };

                        let Some(pipeline_id) = get_pipeline_id(&mesh.layout, mesh_key) else {
                            continue;
                        };

                        render_phase.push(PartMesh {
                            draw_type: DrawType::from(&mesh_draw.material),
                            mesh_asset_id: mesh_draw.mesh.id(),
                            pipeline_id,
                            transform_offset,
                        });
                    }
                }
                ShapeCommand::RenderBitmap {
                    mesh,
                    material,
                    transform,
                    blend_mode,
                } => {
                    let transform_offset =
                        transform_uniform_buffer_writer.write(&TransformUniform::from(*transform));

                    let mesh_asset_id = mesh.id();
                    let Some(mesh) = render_meshes.get(mesh_asset_id) else {
                        continue;
                    };
                    let mut mesh_key = OffscreenMesh2dKey::from(blend_mode);
                    mesh_key |= OffscreenMesh2dKey::BITMAP;

                    let Some(pipeline_id) = get_pipeline_id(&mesh.layout, mesh_key) else {
                        continue;
                    };
                    render_phase.push(PartMesh {
                        draw_type: DrawType::Bitmap(material.id()),
                        mesh_asset_id,
                        pipeline_id,
                        transform_offset,
                    });
                }
                _ => {}
            }
        }
    }
}

#[derive(Resource)]
pub struct FilterUniformBuffers {
    pub view_uniform_buffer: DynamicUniformBuffer<Mat4>,
    pub transform_uniform_buffer: DynamicUniformBuffer<TransformUniform>,

    pub color_matrix_uniform_buffer: DynamicUniformBuffer<ColorMatrixUniform>,
    pub blur_uniform_buffer: DynamicUniformBuffer<BlurUniform>,
    pub glow_uniform_buffer: DynamicUniformBuffer<GlowFilterUniform>,
    pub bevel_uniform_buffer: DynamicUniformBuffer<BevelUniform>,
    pub filter_vertex_with_double_blur_buffer: BufferVec<FilterVertexWithDoubleBlur>,
}

impl Default for FilterUniformBuffers {
    fn default() -> Self {
        Self {
            view_uniform_buffer: DynamicUniformBuffer::default(),
            transform_uniform_buffer: DynamicUniformBuffer::default(),
            color_matrix_uniform_buffer: DynamicUniformBuffer::default(),
            blur_uniform_buffer: DynamicUniformBuffer::default(),
            glow_uniform_buffer: DynamicUniformBuffer::default(),
            bevel_uniform_buffer: DynamicUniformBuffer::default(),
            filter_vertex_with_double_blur_buffer: BufferVec::new(BufferUsages::VERTEX),
        }
    }
}

impl FilterUniformBuffers {
    pub fn clear(&mut self) {
        self.view_uniform_buffer.clear();
        self.transform_uniform_buffer.clear();
        self.color_matrix_uniform_buffer.clear();
        self.blur_uniform_buffer.clear();
        self.glow_uniform_buffer.clear();
        self.bevel_uniform_buffer.clear();
        self.filter_vertex_with_double_blur_buffer.clear();
    }

    pub fn view_uniform_buffer_push(&mut self, uniform: &Mat4) -> u32 {
        self.view_uniform_buffer.push(uniform)
    }

    pub fn blur_uniform_buffer_extend(&mut self, uniforms: &[BlurUniform]) -> Vec<u32> {
        uniforms
            .iter()
            .map(|uniform| self.blur_uniform_buffer.push(uniform))
            .collect()
    }

    pub fn glow_uniform_buffer_push(&mut self, uniform: &GlowFilterUniform) -> u32 {
        self.glow_uniform_buffer.push(uniform)
    }

    pub fn bevel_uniform_buffer_push(&mut self, uniform: &BevelUniform) -> u32 {
        self.bevel_uniform_buffer.push(uniform)
    }

    pub fn color_matrix_uniform_buffer_push(&mut self, uniform: &ColorMatrixUniform) -> u32 {
        self.color_matrix_uniform_buffer.push(uniform)
    }

    pub fn write_buffer(&mut self, device: &RenderDevice, queue: &RenderQueue) {
        self.view_uniform_buffer.write_buffer(device, queue);
        self.color_matrix_uniform_buffer.write_buffer(device, queue);
        self.blur_uniform_buffer.write_buffer(device, queue);
        self.glow_uniform_buffer.write_buffer(device, queue);
        self.bevel_uniform_buffer.write_buffer(device, queue);
    }
}

pub fn get_filter_vertex_with_double_blur(
    distance: f32,
    angle: f32,
    size: Vec2,
) -> Vec<FilterVertexWithDoubleBlur> {
    let blur_offset_x = angle.cos() * distance;
    let blur_offset_y = angle.sin() * distance;
    let width = size.x;
    let height = size.y;
    vec![
        FilterVertexWithDoubleBlur {
            position: [0.0, 0.0],
            source_uv: [0.0, 0.0],
            blur_uv_left: [blur_offset_x / width, blur_offset_y / height],
            blur_uv_right: [
                (0.0 - blur_offset_x) / width,
                (0.0 - blur_offset_y) / height,
            ],
        },
        FilterVertexWithDoubleBlur {
            position: [1.0, 0.0],
            source_uv: [1.0, 0.0],
            blur_uv_left: [(width + blur_offset_x) / width, blur_offset_y / height],
            blur_uv_right: [
                (width - blur_offset_x) / width,
                (0.0 - blur_offset_y) / height,
            ],
        },
        FilterVertexWithDoubleBlur {
            position: [1.0, 1.0],
            source_uv: [1.0, 1.0],
            blur_uv_left: [
                (width + blur_offset_x) / width,
                (height + blur_offset_y) / height,
            ],
            blur_uv_right: [
                (width - blur_offset_x) / width,
                (height - blur_offset_y) / height,
            ],
        },
        FilterVertexWithDoubleBlur {
            position: [0.0, 1.0],
            source_uv: [0.0, 1.0],
            blur_uv_left: [blur_offset_x / width, (blur_offset_y + height) / height],
            blur_uv_right: [
                (0.0 - blur_offset_x) / width,
                (height - blur_offset_y) / height,
            ],
        },
    ]
}
