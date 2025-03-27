use bevy::{
    ecs::{query::QueryState, world::World},
    log::warn,
    render::{
        diagnostic::RecordDiagnostics,
        render_asset::RenderAssets,
        render_graph::{Node, RenderLabel},
        render_resource::{
            BindGroupEntries, BufferInitDescriptor, BufferUsages, CommandEncoderDescriptor,
            IndexFormat, PipelineCache, RenderPassDescriptor,
        },
        renderer::RenderDevice,
        sync_world::MainEntity,
        texture::GpuImage,
    },
};
use bytemuck::{Pod, Zeroable};

use super::{
    IntermediateTexture, IntermediateTextures, MeshDrawType, SwfVertex, TextureCacheInfo,
    pipeline::{IntermediateRenderPhases, IntermediateTexturePipeline},
};

#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderLabel)]
pub struct IntermediateTextureDriverLabel;

pub struct IntermediateTextureDriverNode {
    intermediate_textures: QueryState<(MainEntity, &'static IntermediateTexture)>,
    swf_vertex: QueryState<(MainEntity, &'static SwfVertex)>,
}

#[repr(C)]
#[derive(Default, Clone, Copy, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
}

impl Vertex {
    pub fn new(position: [f32; 3]) -> Self {
        Self { position }
    }
}

#[repr(C)]
#[derive(Default, Clone, Copy, Pod, Zeroable)]
pub struct VertexColor {
    pub position: [f32; 3],
    pub color: [f32; 4],
}

impl VertexColor {
    pub fn new(position: [f32; 3], color: [f32; 4]) -> Self {
        Self { position, color }
    }
}

impl IntermediateTextureDriverNode {
    pub fn new(world: &mut World) -> Self {
        Self {
            intermediate_textures: world.query(),
            swf_vertex: world.query(),
        }
    }
}

impl Node for IntermediateTextureDriverNode {
    fn update(&mut self, world: &mut bevy::ecs::world::World) {
        self.intermediate_textures.update_archetypes(world);
        self.swf_vertex.update_archetypes(world);
    }
    fn run<'w>(
        &self,
        graph: &mut bevy::render::render_graph::RenderGraphContext,
        render_context: &mut bevy::render::renderer::RenderContext<'w>,
        world: &'w bevy::ecs::world::World,
    ) -> Result<(), bevy::render::render_graph::NodeRunError> {
        let intermediate_texture_cache = world.resource::<IntermediateTextures>();
        let intermediate_texture_pipeline = world.resource::<IntermediateTexturePipeline>();
        let render_device = world.resource::<RenderDevice>();
        let pipeline_cache = world.resource::<PipelineCache>();
        let intermediate_render_phases = world.resource::<IntermediateRenderPhases>();
        let gpu_images = world.resource::<RenderAssets<GpuImage>>();
        let diagnostics = render_context.diagnostic_recorder();

        let sampler = &intermediate_texture_pipeline.sampler;
        for (entity, intermediate_texture) in self.intermediate_textures.iter_manual(world) {
            if !intermediate_texture.is_draw {
                continue;
            }
            let mut command_encoder =
                render_device.create_command_encoder(&CommandEncoderDescriptor {
                    label: Some("intermediate_texture_command_encoder"),
                });

            let color_attachment = intermediate_texture_cache
                .0
                .get(&TextureCacheInfo {
                    filter_rect: intermediate_texture.filter_rect,
                    main_entity: entity.into(),
                })
                .map(|a| a.get_attachment());

            let mut render_pass = command_encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("intermediate_texture_render_pass"),
                color_attachments: &[color_attachment],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            let pass_span = diagnostics.pass_span(&mut render_pass, "main_opaque_pass_2d");
            let swf_vertex = self
                .swf_vertex
                .iter_manual(world)
                .filter(|(entity, _)| intermediate_texture.children.contains(entity))
                .map(|(entity, vertex)| (entity, vertex))
                .collect::<Vec<_>>();

            for (child, mesh) in swf_vertex {
                let Some(pipeline_id) = intermediate_render_phases.0.get(&MainEntity::from(child))
                else {
                    continue;
                };
                let Some(pipeline) = pipeline_cache.get_render_pipeline(*pipeline_id) else {
                    continue;
                };

                let index_buffer = render_device.create_buffer_with_data(&BufferInitDescriptor {
                    label: Some("索引缓冲"),
                    contents: bytemuck::cast_slice(&mesh.indices),
                    usage: BufferUsages::INDEX,
                });
                let filter_rect = intermediate_texture.filter_rect;
                let delta = filter_rect - intermediate_texture.size;
                let scale = intermediate_texture.scale;
                let view_matrix = [
                    [2.0 * scale.x / filter_rect.x as f32, 0.0, 0.0, 0.0],
                    [0.0, -2.0 * scale.y / filter_rect.y as f32, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                    [
                        -1.0 + (2.0 * (delta.x as f32 / 2.0)) / filter_rect.x as f32,
                        1.0 - (2.0 * (delta.y as f32 / 2.0)) / filter_rect.y as f32,
                        0.0,
                        1.0,
                    ],
                ];
                let view_buffer = render_device.create_buffer_with_data(&BufferInitDescriptor {
                    label: Some("View Matrix Buffer"),
                    contents: bytemuck::cast_slice(&[view_matrix]),
                    usage: BufferUsages::UNIFORM,
                });
                let world_transform =
                    render_device.create_buffer_with_data(&BufferInitDescriptor {
                        label: Some("swf_world_transform"),
                        contents: bytemuck::cast_slice(&[intermediate_texture.world_transform]),
                        usage: BufferUsages::UNIFORM,
                    });

                let view_bind_group = render_device.create_bind_group(
                    "view_bind_group",
                    &intermediate_texture_pipeline.view_bind_group_layout,
                    &BindGroupEntries::sequential((
                        view_buffer.as_entire_binding(),
                        world_transform.as_entire_binding(),
                    )),
                );

                render_pass.set_pipeline(pipeline);
                render_pass.set_bind_group(0, &view_bind_group, &[]);
                render_pass.set_index_buffer(*index_buffer.slice(..), IndexFormat::Uint32);

                match &mesh.mesh_draw_type {
                    MeshDrawType::Color(swf_color) => {
                        let vertex_buffer =
                            render_device.create_buffer_with_data(&BufferInitDescriptor {
                                label: Some("带有颜色的顶点"),
                                contents: bytemuck::cast_slice(&swf_color),
                                usage: BufferUsages::VERTEX,
                            });

                        render_pass.set_vertex_buffer(0, *vertex_buffer.slice(..));
                        render_pass.draw_indexed(0..mesh.indices.len() as u32, 0, 0..1);
                    }
                    MeshDrawType::Gradient(gradient) => {
                        let vertex_buffer =
                            render_device.create_buffer_with_data(&BufferInitDescriptor {
                                label: Some("仅有顶点"),
                                contents: bytemuck::cast_slice(&gradient.vertex),
                                usage: BufferUsages::VERTEX,
                            });
                        let gradient_uniform_buffer =
                            render_device.create_buffer_with_data(&BufferInitDescriptor {
                                label: Some("渐变缓冲区"),
                                contents: bytemuck::cast_slice(&[gradient.gradient]),
                                usage: BufferUsages::UNIFORM,
                            });
                        let texture_transform_buffer =
                            render_device.create_buffer_with_data(&BufferInitDescriptor {
                                label: Some("渐变纹理变换缓冲区"),
                                contents: bytemuck::cast_slice(&[gradient.texture_transform]),
                                usage: BufferUsages::UNIFORM,
                            });
                        let texture_view = &gpu_images.get(&gradient.texture).unwrap().texture_view;
                        let bind_group = render_device.create_bind_group(
                            Some("渐变纹理绑定组"),
                            &intermediate_texture_pipeline.gradient_bind_group_layout,
                            &BindGroupEntries::sequential((
                                texture_view,
                                sampler,
                                texture_transform_buffer.as_entire_binding(),
                                gradient_uniform_buffer.as_entire_binding(),
                            )),
                        );
                        render_pass.set_bind_group(1, &bind_group, &[]);
                        render_pass.set_vertex_buffer(0, *vertex_buffer.slice(..));
                        render_pass.draw_indexed(0..mesh.indices.len() as u32, 0, 0..1);
                    }
                    _ => {
                        warn!("位图应该不会使用滤镜吧！😊，如果有再做吧！！！")
                    }
                }
            }
            pass_span.end(&mut render_pass);
            drop(render_pass);
            render_context.add_command_buffer(command_encoder.finish());
        }

        Ok(())
    }
}
