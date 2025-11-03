use bevy::{
    render::{
        diagnostic::RecordDiagnostics,
        mesh::{RenderMesh, allocator::MeshAllocator},
        render_asset::RenderAssets,
        render_graph::ViewNode,
        render_phase::TrackedRenderPass,
        render_resource::{CommandEncoderDescriptor, PipelineCache, RenderPassDescriptor},
        sync_world::MainEntity,
    },
    sprite_render::PreparedMaterial2d,
};

use crate::render::{
    graph::OffscreenFlashShapeRenderPhases,
    material::{BitmapMaterial, ColorMaterial, GradientMaterial},
    offscreen_texture::{ExtractedOffscreenTexture, FilterBindGroup, FilterOffsets, ViewTarget},
};

#[derive(Default)]
pub struct OffscreenMainTransparentPass2dNode;

impl ViewNode for OffscreenMainTransparentPass2dNode {
    type ViewQuery = (
        &'static ExtractedOffscreenTexture,
        &'static ViewTarget,
        &'static FilterOffsets,
    );

    fn run<'w>(
        &self,
        graph: &mut bevy::render::render_graph::RenderGraphContext,
        render_context: &mut bevy::render::renderer::RenderContext<'w>,
        (_, target, filter_offsets): bevy::ecs::query::QueryItem<'w, '_, Self::ViewQuery>,
        world: &'w bevy::ecs::world::World,
    ) -> Result<(), bevy::render::render_graph::NodeRunError> {
        let Some(transparent_phases) = world.get_resource::<OffscreenFlashShapeRenderPhases>()
        else {
            return Ok(());
        };
        let view_entity: MainEntity = graph.view_entity().into();

        let Some(transparent_phase) = transparent_phases.get(&view_entity) else {
            return Ok(());
        };

        // 准备资源
        let pipeline_cache = world.resource::<PipelineCache>();
        let filter_bind_group = world.resource::<FilterBindGroup>();
        let render_meshes = world.resource::<RenderAssets<RenderMesh>>();
        let mesh_allocator = world.resource::<MeshAllocator>();
        let color_materials = world.resource::<RenderAssets<PreparedMaterial2d<ColorMaterial>>>();
        let gradient_materials =
            world.resource::<RenderAssets<PreparedMaterial2d<GradientMaterial>>>();
        let texture_materials =
            world.resource::<RenderAssets<PreparedMaterial2d<BitmapMaterial>>>();

        let diagnostics = render_context.diagnostic_recorder();

        let color_attachments = [Some(target.get_color_attachment())];

        render_context.add_command_buffer_generation_task(move |render_device| {
            let mut command_encoder =
                render_device.create_command_encoder(&CommandEncoderDescriptor {
                    label: Some("offscreen_main_transparent_pass_2d_command_encoder"),
                });

            {
                let view_bind_group = &filter_bind_group.view_bind_group;
                let render_pass = command_encoder.begin_render_pass(&RenderPassDescriptor {
                    label: Some("offscreen_main_transparent_pass_2d"),
                    color_attachments: &color_attachments,
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                let mut render_pass = TrackedRenderPass::new(&render_device, render_pass);
                let pass_span =
                    diagnostics.pass_span(&mut render_pass, "offscreen_main_transparent_pass_2d");
                if !transparent_phase.is_empty() {
                    for item in transparent_phase {
                        let Some(pipeline) = pipeline_cache.get_render_pipeline(item.pipeline_id)
                        else {
                            continue;
                        };
                        let Some(gpu_mesh) = render_meshes.get(item.mesh_asset_id) else {
                            continue;
                        };
                        let Some(vertex_buffer_slice) =
                            mesh_allocator.mesh_vertex_slice(&item.mesh_asset_id)
                        else {
                            continue;
                        };
                        // let bind_group = match item.draw_type {
                        //     super::DrawType::Color(asset_id) => {
                        //         let Some(material) = color_materials.get(asset_id) else {
                        //             continue;
                        //         };
                        //         &material.bind_group
                        //     }
                        //     super::DrawType::Gradient(asset_id) => {
                        //         let Some(material) = gradient_materials.get(asset_id) else {
                        //             continue;
                        //         };
                        //         &material.bind_group
                        //     }
                        //     super::DrawType::Bitmap(asset_id) => {
                        //         let Some(material) = texture_materials.get(asset_id) else {
                        //             continue;
                        //         };
                        //         &material.bind_group
                        //     }
                        // };
                        render_pass.set_render_pipeline(pipeline);
                        render_pass.set_vertex_buffer(0, vertex_buffer_slice.buffer.slice(..));
                        render_pass.set_bind_group(
                            0,
                            view_bind_group,
                            &[filter_offsets.view_offset],
                        );
                        // render_pass.set_bind_group(1, bind_group, &[]);
                        let batch_range = 0..1;
                        match &gpu_mesh.buffer_info {
                            bevy::render::mesh::RenderMeshBufferInfo::Indexed {
                                count,
                                index_format,
                            } => {
                                let Some(index_buffer_slice) =
                                    mesh_allocator.mesh_index_slice(&item.mesh_asset_id)
                                else {
                                    continue;
                                };
                                render_pass.set_index_buffer(
                                    index_buffer_slice.buffer.slice(..),
                                    0,
                                    *index_format,
                                );

                                render_pass.draw_indexed(
                                    index_buffer_slice.range.start
                                        ..(index_buffer_slice.range.start + count),
                                    vertex_buffer_slice.range.start as i32,
                                    batch_range.clone(),
                                );
                            }
                            bevy::render::mesh::RenderMeshBufferInfo::NonIndexed => {
                                render_pass.draw(vertex_buffer_slice.range, batch_range.clone());
                            }
                        }
                    }
                }
                pass_span.end(&mut render_pass);
            }

            command_encoder.finish()
        });

        Ok(())
    }
}
