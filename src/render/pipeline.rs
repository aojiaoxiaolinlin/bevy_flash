use std::borrow::Cow;

use bevy::{
    asset::{Handle, uuid_handle},
    core_pipeline::FullscreenShader,
    ecs::{
        resource::Resource,
        system::{Commands, Res},
    },
    math::{Mat4, Vec2},
    mesh::{Mesh, PrimitiveTopology, VertexBufferLayout},
    render::{
        render_resource::{
            AsBindGroup, BindGroupLayout, BindGroupLayoutEntries, BlendComponent, BlendFactor,
            BlendOperation, BlendState, BufferUsages, BufferVec, CachedRenderPipelineId,
            ColorTargetState, ColorWrites, DynamicUniformBuffer, FragmentState, FrontFace,
            MultisampleState, PipelineCache, PolygonMode, PrimitiveState, RenderPipelineDescriptor,
            Sampler, SamplerBindingType, SamplerDescriptor, ShaderStages, ShaderType,
            SpecializedMeshPipeline, TextureFormat, TextureSampleType, VertexFormat, VertexState,
            VertexStepMode,
            binding_types::{sampler, texture_2d, uniform_buffer},
        },
        renderer::{RenderDevice, RenderQueue},
        view::Msaa,
    },
    shader::Shader,
};
use bytemuck::{Pod, Zeroable};

use crate::render::material::{BitmapMaterial, ColorMaterial, GradientMaterial, TransformUniform};

pub const OFFSCREEN_COMMON_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("a1b2c3d4-e5f6-4729-8a9b-0c1d2e3f4a5b");

pub const OFFSCREEN_MESH2D_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("f1e2d3c4-b5a6-4978-8c9d-0e1f2a3b4c5d");

pub const OFFSCREEN_MESH2D_GRADIENT_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("a1b2c3d4-e5f6-4789-8a9b-0c1d2e3f4a5b");

pub const OFFSCREEN_MESH2D_BITMAP_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("e3f4a5b6-c7d8-4e9f-0a1b-2c3d4e5f6a7b");

pub const BLUR_FILTER_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("f59e3d1c-7a24-4b8c-82a3-1d94e6f2c705");

pub const COLOR_MATRIX_FILTER_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("1a2b3c4d-5e6f-4789-0123-456789abcdef");

pub const GLOW_FILTER_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("c1d2e3f4-a5b6-4789-0123-456789abcdef");

pub const BEVEL_FILTER_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("e2f8a9d6-3c7b-42f1-8e9d-5a6b4c3d2e1f");

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

#[derive(Resource, Clone)]
pub struct OffscreenMesh2dPipeline {
    pub view_bind_group_layout: BindGroupLayout,
    pub transform_bind_group_layout: BindGroupLayout,

    pub color_bind_group_layout: BindGroupLayout,
    pub gradient_bind_group_layout: BindGroupLayout,
    pub bitmap_bind_group_layout: BindGroupLayout,
    /// 某些特殊的位图填充好像需要特殊处理，这个暂时保留
    #[allow(unused)]
    pub sampler: Sampler,
}

pub fn init_offscreen_texture_pipeline(mut commands: Commands, render_device: Res<RenderDevice>) {
    let view_bind_group_layout = render_device.create_bind_group_layout(
        "纹理变换矩阵布局",
        &BindGroupLayoutEntries::single(ShaderStages::VERTEX, uniform_buffer::<Mat4>(true)),
    );

    let transform_bind_group_layout = render_device.create_bind_group_layout(
        "变换矩阵布局",
        &BindGroupLayoutEntries::single(
            ShaderStages::VERTEX_FRAGMENT,
            uniform_buffer::<TransformUniform>(true),
        ),
    );

    let color_bind_group_layout = ColorMaterial::bind_group_layout(&render_device);
    let gradient_bind_group_layout = GradientMaterial::bind_group_layout(&render_device);
    let bitmap_bind_group_layout = BitmapMaterial::bind_group_layout(&render_device);

    let sampler = render_device.create_sampler(&SamplerDescriptor::default());

    commands.insert_resource(OffscreenMesh2dPipeline {
        view_bind_group_layout,
        transform_bind_group_layout,
        color_bind_group_layout,
        gradient_bind_group_layout,
        bitmap_bind_group_layout,
        sampler,
    });
}

impl SpecializedMeshPipeline for OffscreenMesh2dPipeline {
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

/// 模糊滤镜
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, ShaderType, Pod, Zeroable, PartialEq)]
pub struct BlurUniform {
    pub direction: [f32; 2],
    pub full_size: f32,
    pub m: f32,
    pub m2: f32,
    pub first_weight: f32,
    pub last_offset: f32,
    pub last_weight: f32,
}

/// 颜色矩阵滤镜
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, ShaderType, Pod, Zeroable, PartialEq)]
pub struct ColorMatrixUniform {
    pub matrix: [f32; 20],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, ShaderType, Pod, Zeroable, PartialEq)]
pub struct GlowFilterUniform {
    pub color: [f32; 4],
    pub strength: f32,
    pub inner: u32,            // a wasteful bool, but we need to be aligned anyway
    pub knockout: u32,         // a wasteful bool, but we need to be aligned anyway
    pub composite_source: u32, // undocumented flash feature, another bool
}

#[repr(C)]
#[derive(Copy, Clone, Debug, ShaderType, Pod, Zeroable, PartialEq)]
pub struct BevelUniform {
    pub highlight_color: [f32; 4],
    pub shadow_color: [f32; 4],
    pub strength: f32,
    pub bevel_type: u32,       // 0 outer, 1 inner, 2 full
    pub knockout: u32,         // a wasteful bool, but we need to be aligned anyway
    pub composite_source: u32, // undocumented flash feature, another bool
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
    pub layout: BindGroupLayout,
    pub sampler: Sampler,
    pub pipeline_id: CachedRenderPipelineId,
}

pub(crate) fn init_blur_filter_pipeline(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    pipeline_cache: Res<PipelineCache>,
    fullscreen_shader: Res<FullscreenShader>,
) {
    let layout = render_device.create_bind_group_layout(
        "blur_filter_bind_group_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                uniform_buffer::<BlurUniform>(false),
            ),
        ),
    );

    let sampler = render_device.create_sampler(&SamplerDescriptor::default());

    let descriptor = RenderPipelineDescriptor {
        label: Some(Cow::from("blur_filter_render_pipeline")),
        layout: vec![layout.clone()],
        push_constant_ranges: vec![],
        vertex: fullscreen_shader.to_vertex_state(),
        primitive: PrimitiveState::default(),
        depth_stencil: None,
        multisample: MultisampleState::default(),
        fragment: Some(FragmentState {
            shader: BLUR_FILTER_SHADER_HANDLE,
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
    pub layout: BindGroupLayout,
    pub sampler: Sampler,
    pub pipeline_id: CachedRenderPipelineId,
}

pub(crate) fn init_color_matrix_filter_pipeline(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    pipeline_cache: Res<PipelineCache>,
    fullscreen_shader: Res<FullscreenShader>,
) {
    let layout = render_device.create_bind_group_layout(
        "color_matrix_bind_group_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                uniform_buffer::<ColorMatrixUniform>(false),
            ),
        ),
    );
    let sampler = render_device.create_sampler(&SamplerDescriptor::default());

    let descriptor = RenderPipelineDescriptor {
        label: Some(Cow::from("color_matrix_filter_render_pipeline")),
        layout: vec![layout.clone()],
        push_constant_ranges: vec![],
        vertex: fullscreen_shader.to_vertex_state(),
        primitive: PrimitiveState::default(),
        depth_stencil: None,
        multisample: MultisampleState::default(),
        fragment: Some(FragmentState {
            shader: COLOR_MATRIX_FILTER_SHADER_HANDLE,
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
    pub layout: BindGroupLayout,
    pub sampler: Sampler,
    pub pipeline_id: CachedRenderPipelineId,
}

pub(crate) fn init_glow_filter_pipeline(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    pipeline_cache: Res<PipelineCache>,
    fullscreen_shader: Res<FullscreenShader>,
) {
    let layout = render_device.create_bind_group_layout(
        "glow_filter_bind_group_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                uniform_buffer::<GlowFilterUniform>(false),
                texture_2d(TextureSampleType::Float { filterable: true }),
            ),
        ),
    );
    let sampler = render_device.create_sampler(&SamplerDescriptor::default());

    let descriptor = RenderPipelineDescriptor {
        label: Some(Cow::from("glow_filter_filter_render_pipeline")),
        layout: vec![layout.clone()],
        push_constant_ranges: vec![],
        vertex: fullscreen_shader.to_vertex_state(),
        primitive: PrimitiveState::default(),
        depth_stencil: None,
        multisample: MultisampleState::default(),
        fragment: Some(FragmentState {
            shader: GLOW_FILTER_SHADER_HANDLE,
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
    pub layout: BindGroupLayout,
    pub sampler: Sampler,
    pub pipeline_id: CachedRenderPipelineId,
}

pub(crate) fn init_bevel_filter_pipeline(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    pipeline_cache: Res<PipelineCache>,
) {
    let layout = render_device.create_bind_group_layout(
        "glow_filter_bind_group_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                uniform_buffer::<BevelUniform>(false),
                texture_2d(TextureSampleType::Float { filterable: true }),
            ),
        ),
    );
    let sampler = render_device.create_sampler(&SamplerDescriptor::default());

    let descriptor = RenderPipelineDescriptor {
        label: Some(Cow::from("bevel_filter_render_pipeline")),
        layout: vec![layout.clone()],
        push_constant_ranges: vec![],
        vertex: VertexState {
            shader: BEVEL_FILTER_SHADER_HANDLE,
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
            shader: BEVEL_FILTER_SHADER_HANDLE,
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

    pub fn write_view_buffer(&mut self, device: &RenderDevice, queue: &RenderQueue) {
        self.view_uniform_buffer.write_buffer(device, queue);
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
