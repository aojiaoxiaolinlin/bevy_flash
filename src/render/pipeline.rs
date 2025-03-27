use std::borrow::Cow;

use bevy::{
    asset::{Handle, weak_handle},
    ecs::{
        resource::Resource,
        system::{Query, Res, ResMut, SystemState},
        world::FromWorld,
    },
    image::BevyDefault,
    math::Mat4,
    render::{
        mesh::{PrimitiveTopology, VertexBufferLayout},
        render_resource::{
            BindGroupLayout, BindGroupLayoutEntries, BlendState, CachedRenderPipelineId,
            ColorTargetState, ColorWrites, FragmentState, FrontFace, MultisampleState,
            PipelineCache, PolygonMode, PrimitiveState, RenderPipelineDescriptor, Sampler,
            SamplerBindingType, SamplerDescriptor, Shader, ShaderStages, SpecializedRenderPipeline,
            SpecializedRenderPipelines, TextureFormat, TextureSampleType, VertexFormat,
            VertexState, VertexStepMode,
            binding_types::{sampler, texture_2d, uniform_buffer},
        },
        renderer::RenderDevice,
        sync_world::{MainEntity, MainEntityHashMap},
        texture::GpuImage,
        view::Msaa,
    },
};

use super::{MeshDrawType, SwfVertex, material::GradientUniforms};

pub const INTERMEDIATE_TEXTURE_MESH: Handle<Shader> =
    weak_handle!("f1e2d3c4-b5a6-4978-8c9d-0e1f2a3b4c5d");

pub const INTERMEDIATE_TEXTURE_GRADIENT: Handle<Shader> =
    weak_handle!("a1b2c3d4-e5f6-4789-8a9b-0c1d2e3f4a5b");

#[derive(Resource, Clone)]
pub struct IntermediateTexturePipeline {
    pub view_bind_group_layout: BindGroupLayout,
    pub gradient_bind_group_layout: BindGroupLayout,
    pub bitmap_bind_group_layout: BindGroupLayout,
    pub sampler: Sampler,
}

impl FromWorld for IntermediateTexturePipeline {
    fn from_world(world: &mut bevy::ecs::world::World) -> Self {
        let mut system_state: SystemState<Res<RenderDevice>> = SystemState::new(world);
        let render_device = system_state.get_mut(world);
        let render_device = render_device.into_inner();

        let view_bind_group_layout = render_device.create_bind_group_layout(
            "纹理变换矩阵布局",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::VERTEX,
                (
                    uniform_buffer::<[[f32; 4]; 4]>(false),
                    uniform_buffer::<Mat4>(false),
                ),
            ),
        );
        let gradient_bind_group_layout = render_device.create_bind_group_layout(
            "渐变纹理布局",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::VERTEX_FRAGMENT,
                (
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
                    uniform_buffer::<Mat4>(false),
                    uniform_buffer::<GradientUniforms>(false),
                ),
            ),
        );

        let bitmap_bind_group_layout = render_device.create_bind_group_layout(
            "位图纹理",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::VERTEX_FRAGMENT,
                (
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
                    uniform_buffer::<[[f32; 4]; 4]>(false),
                    uniform_buffer::<GradientUniforms>(false),
                ),
            ),
        );

        let sampler = render_device.create_sampler(&SamplerDescriptor::default());

        IntermediateTexturePipeline {
            view_bind_group_layout,
            gradient_bind_group_layout,
            bitmap_bind_group_layout,
            sampler,
        }
    }
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    #[repr(transparent)]
    pub struct IntermediateTextureKey:u8 {
        const NONE     = 0;
        const COLOR    = 1 << 1;
        const GRADIENT = 1 << 2;
        const BITMAP   = 1 << 3;
        const MSAA     = 1 << 4;
    }
}

impl SpecializedRenderPipeline for IntermediateTexturePipeline {
    type Key = IntermediateTextureKey;

    fn specialize(&self, key: Self::Key) -> RenderPipelineDescriptor {
        let mut shader_defs = Vec::new();
        let vertex_buffer_layout = if key.contains(IntermediateTextureKey::COLOR) {
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
        if key.contains(IntermediateTextureKey::COLOR) {
            shader_defs.push("VERTEX_COLORS".into());
        }

        let bind_group_layout = if key.contains(IntermediateTextureKey::GRADIENT) {
            vec![
                self.view_bind_group_layout.clone(),
                self.gradient_bind_group_layout.clone(),
            ]
        } else if key.contains(IntermediateTextureKey::BITMAP) {
            vec![
                self.view_bind_group_layout.clone(),
                self.bitmap_bind_group_layout.clone(),
            ]
        } else {
            vec![self.view_bind_group_layout.clone()]
        };

        let shader = if key.contains(IntermediateTextureKey::COLOR) {
            INTERMEDIATE_TEXTURE_MESH
        } else {
            INTERMEDIATE_TEXTURE_GRADIENT
        };

        RenderPipelineDescriptor {
            label: Some(Cow::from("intermediate_render_pipeline")),
            layout: bind_group_layout,
            push_constant_ranges: vec![],
            vertex: VertexState {
                shader: shader.clone(),
                shader_defs: shader_defs.clone(),
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
                shader_defs,
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

#[derive(Resource, Default)]
pub struct IntermediateRenderPhases(pub MainEntityHashMap<CachedRenderPipelineId>);

pub fn specialize_meshes(
    mut specialized_render_pipelines: ResMut<
        SpecializedRenderPipelines<IntermediateTexturePipeline>,
    >,
    mut intermediate_texture_phases: ResMut<IntermediateRenderPhases>,
    intermediate_texture_pipeline: Res<IntermediateTexturePipeline>,
    pipeline_cache: Res<PipelineCache>,
    query: Query<(MainEntity, &SwfVertex)>,
) {
    for (entity, swf_vertex) in query.iter() {
        let key = match swf_vertex.mesh_draw_type {
            MeshDrawType::Color(_) => IntermediateTextureKey::COLOR,
            MeshDrawType::Gradient(_) => IntermediateTextureKey::GRADIENT,
            MeshDrawType::Bitmap => IntermediateTextureKey::BITMAP,
        };
        let pipeline_id = specialized_render_pipelines.specialize(
            &pipeline_cache,
            &intermediate_texture_pipeline,
            key,
        );
        intermediate_texture_phases
            .0
            .insert(entity.into(), pipeline_id);
    }
}
