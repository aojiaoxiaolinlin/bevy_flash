use std::borrow::Cow;

use bevy::{
    asset::{Handle, weak_handle},
    core_pipeline::fullscreen_vertex_shader::fullscreen_shader_vertex_state,
    ecs::{
        resource::Resource,
        system::{Res, SystemState},
        world::{FromWorld, World},
    },
    image::BevyDefault,
    math::{Mat4, UVec2},
    prelude::Deref,
    render::{
        mesh::{Mesh, PrimitiveTopology, VertexBufferLayout},
        render_resource::{
            AsBindGroup, BindGroupLayout, BindGroupLayoutEntries, BlendComponent, BlendFactor,
            BlendOperation, BlendState, Buffer, BufferInitDescriptor, BufferUsages,
            CachedRenderPipelineId, ColorTargetState, ColorWrites, FragmentState, FrontFace,
            MultisampleState, PipelineCache, PolygonMode, PrimitiveState, RawBufferVec,
            RenderPipelineDescriptor, Sampler, SamplerBindingType, SamplerDescriptor, Shader,
            ShaderStages, ShaderType, SpecializedMeshPipeline, SpecializedRenderPipeline,
            TextureFormat, TextureSampleType, VertexFormat, VertexState, VertexStepMode,
            binding_types::{sampler, texture_2d, uniform_buffer},
        },
        renderer::{RenderDevice, RenderQueue},
        view::Msaa,
    },
};
use bytemuck::{Pod, Zeroable};

use crate::render::material::{BitmapMaterial, ColorMaterial, GradientMaterial};

pub const OFFSCREEN_MESH2D_SHADER_HANDLE: Handle<Shader> =
    weak_handle!("f1e2d3c4-b5a6-4978-8c9d-0e1f2a3b4c5d");

pub const OFFSCREEN_MESH2D_GRADIENT_SHADER_HANDLE: Handle<Shader> =
    weak_handle!("a1b2c3d4-e5f6-4789-8a9b-0c1d2e3f4a5b");

pub const OFFSCREEN_MESH2D_BITMAP_SHADER_HANDLE: Handle<Shader> =
    weak_handle!("e3f4a5b6-c7d8-4e9f-0a1b-2c3d4e5f6a7b");

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
    pub color_bind_group_layout: BindGroupLayout,
    pub gradient_bind_group_layout: BindGroupLayout,
    pub bitmap_bind_group_layout: BindGroupLayout,
    /// 某些特殊的位图填充好像需要特殊处理，这个暂时保留
    #[allow(unused)]
    pub sampler: Sampler,
}

impl FromWorld for OffscreenMesh2dPipeline {
    fn from_world(world: &mut bevy::ecs::world::World) -> Self {
        let mut system_state: SystemState<Res<RenderDevice>> = SystemState::new(world);
        let render_device = system_state.get_mut(world);
        let render_device = render_device.into_inner();

        let view_bind_group_layout = render_device.create_bind_group_layout(
            "纹理变换矩阵布局",
            &BindGroupLayoutEntries::single(ShaderStages::VERTEX, uniform_buffer::<Mat4>(false)),
        );

        let color_bind_group_layout = ColorMaterial::bind_group_layout(render_device);
        let gradient_bind_group_layout = GradientMaterial::bind_group_layout(render_device);
        let bitmap_bind_group_layout = BitmapMaterial::bind_group_layout(render_device);

        let sampler = render_device.create_sampler(&SamplerDescriptor::default());

        OffscreenMesh2dPipeline {
            view_bind_group_layout,
            color_bind_group_layout,
            gradient_bind_group_layout,
            bitmap_bind_group_layout,
            sampler,
        }
    }
}

impl SpecializedMeshPipeline for OffscreenMesh2dPipeline {
    type Key = OffscreenMesh2dKey;

    fn specialize(
        &self,
        key: Self::Key,
        layout: &bevy::render::mesh::MeshVertexBufferLayoutRef,
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
        let format = TextureFormat::bevy_default();

        let label;
        let bind_group_layout = if key.contains(OffscreenMesh2dKey::COLOR) {
            label = "color_offscreen_mesh2d";
            vec![
                self.view_bind_group_layout.clone(),
                self.color_bind_group_layout.clone(),
            ]
        } else if key.contains(OffscreenMesh2dKey::GRADIENT) {
            label = "gradient_offscreen_mesh2d";
            vec![
                self.view_bind_group_layout.clone(),
                self.gradient_bind_group_layout.clone(),
            ]
        } else {
            label = "bitmap_offscreen_mesh2d";
            vec![
                self.view_bind_group_layout.clone(),
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
                entry_point: "vertex".into(),
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
                entry_point: "fragment".into(),
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

impl SpecializedRenderPipeline for OffscreenMesh2dPipeline {
    type Key = OffscreenMesh2dKey;

    fn specialize(&self, key: Self::Key) -> RenderPipelineDescriptor {
        let vertex_buffer_layout = if key.contains(OffscreenMesh2dKey::COLOR) {
            VertexBufferLayout::from_vertex_formats(
                VertexStepMode::Vertex,
                vec![VertexFormat::Float32x3, VertexFormat::Float32x4],
            )
        } else {
            VertexBufferLayout::from_vertex_formats(
                VertexStepMode::Vertex,
                vec![VertexFormat::Float32x3],
            )
        };

        let bind_group_layout = if key.contains(OffscreenMesh2dKey::GRADIENT) {
            vec![
                self.view_bind_group_layout.clone(),
                self.gradient_bind_group_layout.clone(),
            ]
        } else if key.contains(OffscreenMesh2dKey::BITMAP) {
            vec![
                self.view_bind_group_layout.clone(),
                self.bitmap_bind_group_layout.clone(),
            ]
        } else {
            vec![self.view_bind_group_layout.clone()]
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

        RenderPipelineDescriptor {
            label: Some(Cow::from("intermediate_render_pipeline")),
            layout: bind_group_layout,
            push_constant_ranges: vec![],
            vertex: VertexState {
                shader: shader.clone(),
                shader_defs: vec![],
                entry_point: "vertex".into(),
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
                entry_point: "fragment".into(),
                targets: vec![Some(ColorTargetState {
                    format: TextureFormat::bevy_default(),
                    blend: Some(BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: ColorWrites::ALL,
                })],
            }),
            zero_initialize_workgroup_memory: false,
        }
    }
}

pub const BLUR_FILTER_SHADER_HANDLE: Handle<Shader> =
    weak_handle!("f59e3d1c-7a24-4b8c-82a3-1d94e6f2c705");

pub const COLOR_MATRIX_FILTER_SHADER_HANDLE: Handle<Shader> =
    weak_handle!("1a2b3c4d-5e6f-4789-0123-456789abcdef");

pub const GLOW_FILTER_SHADER_HANDLE: Handle<Shader> =
    weak_handle!("c1d2e3f4-a5b6-4789-0123-456789abcdef");

pub const BEVEL_FILTER_SHADER_HANDLE: Handle<Shader> =
    weak_handle!("e2f8a9d6-3c7b-42f1-8e9d-5a6b4c3d2e1f");

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

impl FromWorld for BlurFilterPipeline {
    fn from_world(world: &mut bevy::ecs::world::World) -> Self {
        let render_device = world.resource::<RenderDevice>();
        let pipeline_cache = world.resource::<PipelineCache>();
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
            vertex: fullscreen_shader_vertex_state(),
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: MultisampleState::default(),
            fragment: Some(FragmentState {
                shader: BLUR_FILTER_SHADER_HANDLE,
                shader_defs: vec![],
                entry_point: "fragment".into(),
                targets: vec![Some(ColorTargetState {
                    format: TextureFormat::bevy_default(),
                    blend: None,
                    write_mask: ColorWrites::ALL,
                })],
            }),
            zero_initialize_workgroup_memory: false,
        };

        let pipeline_id = pipeline_cache.queue_render_pipeline(descriptor);

        Self {
            layout,
            sampler,
            pipeline_id,
        }
    }
}

#[derive(Resource)]
pub struct ColorMatrixFilterPipeline {
    pub layout: BindGroupLayout,
    pub sampler: Sampler,
    pub pipeline_id: CachedRenderPipelineId,
}

impl FromWorld for ColorMatrixFilterPipeline {
    fn from_world(world: &mut bevy::ecs::world::World) -> Self {
        let render_device = world.resource::<RenderDevice>();
        let pipeline_cache = world.resource::<PipelineCache>();
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
            vertex: fullscreen_shader_vertex_state(),
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: MultisampleState::default(),
            fragment: Some(FragmentState {
                shader: COLOR_MATRIX_FILTER_SHADER_HANDLE,
                shader_defs: vec![],
                entry_point: "fragment".into(),
                targets: vec![Some(ColorTargetState {
                    format: TextureFormat::bevy_default(),
                    blend: None,
                    write_mask: ColorWrites::ALL,
                })],
            }),
            zero_initialize_workgroup_memory: false,
        };

        let pipeline_id = pipeline_cache.queue_render_pipeline(descriptor);

        Self {
            layout,
            sampler,
            pipeline_id,
        }
    }
}

#[derive(Resource)]
pub struct GlowFilterPipeline {
    pub layout: BindGroupLayout,
    pub sampler: Sampler,
    pub pipeline_id: CachedRenderPipelineId,
}

impl FromWorld for GlowFilterPipeline {
    fn from_world(world: &mut bevy::ecs::world::World) -> Self {
        let render_device = world.resource::<RenderDevice>();
        let pipeline_cache = world.resource::<PipelineCache>();
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
            vertex: VertexState {
                shader: GLOW_FILTER_SHADER_HANDLE,
                shader_defs: vec![],
                entry_point: "vertex".into(),
                buffers: vec![VertexBufferLayout::from_vertex_formats(
                    VertexStepMode::Vertex,
                    vec![
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
                shader: GLOW_FILTER_SHADER_HANDLE,
                shader_defs: vec![],
                entry_point: "fragment".into(),
                targets: vec![Some(ColorTargetState {
                    format: TextureFormat::bevy_default(),
                    blend: None,
                    write_mask: ColorWrites::ALL,
                })],
            }),
            zero_initialize_workgroup_memory: false,
        };

        let pipeline_id = pipeline_cache.queue_render_pipeline(descriptor);

        Self {
            layout,
            sampler,
            pipeline_id,
        }
    }
}

#[derive(Resource)]
pub struct BevelFilterPipeline {
    pub layout: BindGroupLayout,
    pub sampler: Sampler,
    pub pipeline_id: CachedRenderPipelineId,
}

impl FromWorld for BevelFilterPipeline {
    fn from_world(world: &mut bevy::ecs::world::World) -> Self {
        let render_device = world.resource::<RenderDevice>();
        let pipeline_cache = world.resource::<PipelineCache>();
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
                entry_point: "vertex".into(),
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
                entry_point: "fragment".into(),
                targets: vec![Some(ColorTargetState {
                    format: TextureFormat::bevy_default(),
                    blend: None,
                    write_mask: ColorWrites::ALL,
                })],
            }),
            zero_initialize_workgroup_memory: false,
        };

        let pipeline_id = pipeline_cache.queue_render_pipeline(descriptor);

        Self {
            layout,
            sampler,
            pipeline_id,
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct FilterVertexWithBlur {
    pub position: [f32; 2],
    pub source_uv: [f32; 2],
    pub blur_uv: [f32; 2],
}

pub fn vertices_with_blur_offset(
    blur_offset: (f32, f32),
    size: UVec2,
) -> [FilterVertexWithBlur; 4] {
    let point = (0, 0);
    let source_width = size.x as f32;
    let source_height = size.y as f32;
    let source_left = point.0;
    let source_top = point.1;
    let source_right = source_left + size.x;
    let source_bottom = source_top + size.y;
    [
        FilterVertexWithBlur {
            position: [0.0, 0.0],
            source_uv: [
                source_left as f32 / source_width,
                source_top as f32 / source_height,
            ],
            blur_uv: [
                (source_left as f32 + blur_offset.0) / source_width,
                (source_top as f32 + blur_offset.1) / source_height,
            ],
        },
        FilterVertexWithBlur {
            position: [1.0, 0.0],
            source_uv: [
                source_right as f32 / source_width,
                source_top as f32 / source_height,
            ],
            blur_uv: [
                (source_right as f32 + blur_offset.0) / source_width,
                (source_top as f32 + blur_offset.1) / source_height,
            ],
        },
        FilterVertexWithBlur {
            position: [1.0, 1.0],
            source_uv: [
                source_right as f32 / source_width,
                source_bottom as f32 / source_height,
            ],
            blur_uv: [
                (source_right as f32 + blur_offset.0) / source_width,
                (source_bottom as f32 + blur_offset.1) / source_height,
            ],
        },
        FilterVertexWithBlur {
            position: [0.0, 1.0],
            source_uv: [
                source_left as f32 / source_width,
                source_bottom as f32 / source_height,
            ],
            blur_uv: [
                (source_left as f32 + blur_offset.0) / source_width,
                (source_bottom as f32 + blur_offset.1) / source_height,
            ],
        },
    ]
}

#[derive(Resource, Deref)]
pub struct RectVertexIndicesBuffer(pub RawBufferVec<u32>);

impl FromWorld for RectVertexIndicesBuffer {
    fn from_world(world: &mut World) -> Self {
        let render_device = world.resource::<RenderDevice>();
        let render_queue = world.resource::<RenderQueue>();

        let mut ibo = RawBufferVec::new(BufferUsages::INDEX);
        ibo.extend([0, 1, 2, 0, 2, 3]);
        ibo.write_buffer(render_device, render_queue);
        Self(ibo)
    }
}
