use crate::{
    render::{
        filter_render::{
            BevelFilterPipeline, BlurFilterPipeline, ColorMatrixFilterPipeline, Filters,
            GlowFilterPipeline, SourceTextureLayout, get_filter_vertex_with_double_blur,
        },
        offscreen_render::{ExtractedOffscreenCamera, FilterBindGroup, FilterOffsets, ViewTarget},
    },
    swf_runtime::filter::Filter::{
        BevelFilter, BlurFilter, ColorMatrixFilter, ConvolutionFilter, DropShadowFilter,
        GlowFilter, GradientBevelFilter, GradientGlowFilter,
    },
};
use bevy::{
    ecs::entity::Entity,
    log::warn_once,
    render::{
        render_graph::ViewNode,
        render_phase::TrackedRenderPass,
        render_resource::{
            BindGroup, BindGroupEntries, BindGroupLayout, BufferInitDescriptor, BufferUsages,
            IndexFormat, Operations, PipelineCache, RenderPassColorAttachment,
            RenderPassDescriptor, RenderPipeline, TexelCopyTextureInfo, TextureAspect,
            TextureDescriptor, TextureDimension, TextureUsages, TextureView, TextureViewDescriptor,
        },
        renderer::RenderContext,
    },
};

#[derive(Default)]
pub struct FilterPostProcessingNode;

impl ViewNode for FilterPostProcessingNode {
    type ViewQuery = (
        &'static ExtractedOffscreenCamera,
        &'static Filters,
        &'static FilterOffsets,
        &'static ViewTarget,
    );

    fn run<'w>(
        &self,
        _graph: &mut bevy::render::render_graph::RenderGraphContext,
        render_context: &mut bevy::render::renderer::RenderContext<'w>,
        (offscreen_camera, filters, offsets, view_target): bevy::ecs::query::QueryItem<
            'w,
            '_,
            Self::ViewQuery,
        >,
        world: &'w bevy::ecs::world::World,
    ) -> Result<(), bevy::render::render_graph::NodeRunError> {
        let pipeline_cache = world.resource::<PipelineCache>();
        let source_texture_layout = world.resource::<SourceTextureLayout>();
        let blur_filter_pipeline = world.resource::<BlurFilterPipeline>();
        let color_matrix_filter_pipeline = world.resource::<ColorMatrixFilterPipeline>();
        let glow_filter_pipeline = world.resource::<GlowFilterPipeline>();
        let bevel_filter_pipeline = world.resource::<BevelFilterPipeline>();

        let filter_bind_group = world.resource::<FilterBindGroup>();

        let source_layout =
            &pipeline_cache.get_bind_group_layout(&source_texture_layout.source_layout);
        let blur_layout =
            &pipeline_cache.get_bind_group_layout(&source_texture_layout.blur_texture_layout);

        let mut color_pass_index = 0;
        let mut blur_pass_index = 0;
        let mut glow_pass_index = 0;
        let mut bevel_pass_index = 0;

        // 以下算法均来自于Ruffle
        let size = offscreen_camera.size;
        for filter in filters.iter() {
            match filter {
                BlurFilter(blur_filter) => {
                    let Some(pipeline) =
                        pipeline_cache.get_render_pipeline(blur_filter_pipeline.pipeline_id)
                    else {
                        continue;
                    };
                    let blur_offsets = &offsets.blur_offsets;

                    let blur_bind_group = filter_bind_group
                        .blur_bind_group
                        .as_ref()
                        .expect("必然存在");

                    apply_blur(
                        blur_filter,
                        blur_bind_group,
                        blur_offsets,
                        render_context,
                        pipeline,
                        blur_filter_pipeline,
                        source_layout,
                        view_target,
                        &mut blur_pass_index,
                    );
                }
                GlowFilter(glow_filter) => {
                    let Some(blur_filter_render_pipeline) =
                        pipeline_cache.get_render_pipeline(blur_filter_pipeline.pipeline_id)
                    else {
                        continue;
                    };
                    let Some(glow_filter_render_pipeline) =
                        pipeline_cache.get_render_pipeline(glow_filter_pipeline.pipeline_id)
                    else {
                        continue;
                    };

                    let blur_offsets = &offsets.blur_offsets;

                    let blur_bind_group = filter_bind_group
                        .blur_bind_group
                        .as_ref()
                        .expect("必然存在");

                    let temp_texture_view = copy_source_texture(render_context, view_target);
                    apply_blur(
                        &glow_filter.inner_blur_filter(),
                        blur_bind_group,
                        blur_offsets,
                        render_context,
                        blur_filter_render_pipeline,
                        blur_filter_pipeline,
                        source_layout,
                        view_target,
                        &mut blur_pass_index,
                    );
                    let offset = offsets.glow_offsets[glow_pass_index];
                    glow_pass_index += 1;

                    let glow_bind_group = filter_bind_group
                        .glow_bind_group
                        .as_ref()
                        .expect("必然存在");

                    let post_process = view_target.post_process_write();

                    let render_device = render_context.render_device();
                    let bind_group = render_device.create_bind_group(
                        Some("source_bind_group"),
                        source_layout,
                        &BindGroupEntries::sequential((
                            &temp_texture_view,
                            &glow_filter_pipeline.sampler,
                        )),
                    );

                    let blur_bind_group = render_device.create_bind_group(
                        Some("blur_filter_bind_group"),
                        blur_layout,
                        &BindGroupEntries::single(post_process.source),
                    );

                    let mut render_pass = get_render_pass(
                        render_context,
                        post_process.destination,
                        "glow_filter_render_pass",
                    );
                    render_pass.set_render_pipeline(glow_filter_render_pipeline);
                    render_pass.set_bind_group(0, &bind_group, &[]);
                    render_pass.set_bind_group(1, &blur_bind_group, &[]);
                    render_pass.set_bind_group(2, &glow_bind_group, &[offset]);
                    render_pass.draw(0..3, 0..1);
                }
                ColorMatrixFilter(_) => {
                    let Some(pipeline) = pipeline_cache
                        .get_render_pipeline(color_matrix_filter_pipeline.pipeline_id)
                    else {
                        continue;
                    };

                    let offset = offsets.color_offsets[color_pass_index];
                    color_pass_index += 1;

                    let post_process = view_target.post_process_write();
                    let render_device = render_context.render_device();

                    let source_bind_group = render_device.create_bind_group(
                        Some("color_matrix_bind_group"),
                        source_layout,
                        &BindGroupEntries::sequential((
                            post_process.source,
                            &color_matrix_filter_pipeline.sampler,
                        )),
                    );
                    let mut render_pass = get_render_pass(
                        render_context,
                        post_process.destination,
                        "color_matrix_filter_render_pass",
                    );
                    render_pass.set_render_pipeline(pipeline);
                    render_pass.set_bind_group(0, &source_bind_group, &[]);
                    render_pass.set_bind_group(
                        1,
                        filter_bind_group
                            .color_matrix_bind_group
                            .as_ref()
                            .expect("必然存在"),
                        &[offset],
                    );
                    render_pass.draw(0..3, 0..1);
                }
                BevelFilter(bevel_filter) => {
                    let Some(blur_filter_render_pipeline) =
                        pipeline_cache.get_render_pipeline(blur_filter_pipeline.pipeline_id)
                    else {
                        continue;
                    };
                    let Some(bevel_filter_render_pipeline) =
                        pipeline_cache.get_render_pipeline(bevel_filter_pipeline.pipeline_id)
                    else {
                        continue;
                    };

                    let blur_bind_group = filter_bind_group
                        .blur_bind_group
                        .as_ref()
                        .expect("必然存在");

                    let blur_offsets = &offsets.blur_offsets;

                    let temp_texture_view = copy_source_texture(render_context, view_target);
                    apply_blur(
                        &bevel_filter.inner_blur_filter(),
                        blur_bind_group,
                        blur_offsets,
                        render_context,
                        blur_filter_render_pipeline,
                        blur_filter_pipeline,
                        source_layout,
                        view_target,
                        &mut blur_pass_index,
                    );

                    let offset = offsets.bevel_offsets[bevel_pass_index];
                    bevel_pass_index += 1;

                    let bevel_bind_group = filter_bind_group
                        .bevel_bind_group
                        .as_ref()
                        .expect("必然存在");

                    let post_process = view_target.post_process_write();
                    // TODO:
                    let distance = bevel_filter.distance.to_f32();
                    let angle = bevel_filter.angle.to_f32();
                    let filter_vertex_with_double_blur =
                        get_filter_vertex_with_double_blur(distance, angle, size.as_vec2());

                    let render_device = render_context.render_device();
                    let vertex_buffer =
                        render_device.create_buffer_with_data(&BufferInitDescriptor {
                            label: Some("bevel_filter_with_double_uv"),
                            contents: bytemuck::cast_slice(&filter_vertex_with_double_blur),
                            usage: BufferUsages::VERTEX,
                        });

                    let indices = vec![0, 1, 2, 0, 2, 3];
                    let indices_buffer =
                        render_device.create_buffer_with_data(&BufferInitDescriptor {
                            label: Some("bevel_filter_quad_indices"),
                            contents: bytemuck::cast_slice(&indices),
                            usage: BufferUsages::INDEX,
                        });

                    let bind_group = render_device.create_bind_group(
                        "bevel_filter_bind_group",
                        source_layout,
                        &BindGroupEntries::sequential((
                            &temp_texture_view,
                            &bevel_filter_pipeline.sampler,
                        )),
                    );

                    let blur_bind_group = render_device.create_bind_group(
                        Some("blur_filter_bind_group"),
                        blur_layout,
                        &BindGroupEntries::single(post_process.source),
                    );

                    let mut render_pass = get_render_pass(
                        render_context,
                        post_process.destination,
                        "bevel_filter_render_pass",
                    );
                    render_pass.set_render_pipeline(bevel_filter_render_pipeline);
                    render_pass.set_bind_group(0, &bind_group, &[]);
                    render_pass.set_bind_group(1, &blur_bind_group, &[]);
                    render_pass.set_bind_group(2, &bevel_bind_group, &[offset]);
                    render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
                    render_pass.set_index_buffer(indices_buffer.slice(..), IndexFormat::Uint32);
                    render_pass.draw_indexed(0..indices.len() as u32, 0, 0..1);
                }
                DropShadowFilter(..) => {
                    warn_once!("DropShadowFilter 滤镜尚未实现，我需要帮助!!!");
                }
                ConvolutionFilter(..) => {
                    warn_once!("ConvolutionFilter 滤镜尚未实现，我需要帮助!!!");
                }
                GradientBevelFilter(..) => {
                    warn_once!("GradientBevelFilter 滤镜尚未实现，我需要帮助!!!");
                }
                GradientGlowFilter(..) => {
                    warn_once!("GradientGlowFilter 滤镜尚未实现，我需要帮助!!!");
                }
            }
        }
        Ok(())
    }
}

fn apply_blur<'w>(
    blur_filter: &swf::BlurFilter,
    blur_bind_group: &BindGroup,
    blur_offsets: &Vec<u32>,
    render_context: &mut RenderContext<'w>,
    pipeline: &RenderPipeline,
    blur_filter_pipeline: &BlurFilterPipeline,
    source_layout: &BindGroupLayout,
    view_target: &ViewTarget,
    blur_pass_index: &mut usize,
) {
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

            // let Some(offset) = blur_offsets.get(*blur_pass_index) else {
            //     return;
            // };
            let offset = blur_offsets[*blur_pass_index];
            *blur_pass_index += 1;

            let render_device = render_context.render_device();

            let post_process = view_target.post_process_write();
            let bind_group = render_device.create_bind_group(
                Some("source_bind_group"),
                source_layout,
                &BindGroupEntries::sequential((post_process.source, &blur_filter_pipeline.sampler)),
            );
            let mut render_pass = get_render_pass(
                render_context,
                post_process.destination,
                "blur_filter_render_pass",
            );
            render_pass.set_render_pipeline(pipeline);
            render_pass.set_bind_group(0, &bind_group, &[]);
            render_pass.set_bind_group(1, blur_bind_group, &[offset]);
            render_pass.draw(0..3, 0..1);
        }
    }
}

fn copy_source_texture<'a, 'w>(
    render_context: &'a mut RenderContext<'w>,
    view_target: &ViewTarget,
) -> TextureView {
    let source_texture = view_target.post_process_write().source_texture;
    let source = source_texture.as_image_copy();

    let size = source_texture.size();
    let temp_texture = render_context
        .render_device()
        .create_texture(&TextureDescriptor {
            label: None,
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: view_target.main_texture_format(),
            usage: TextureUsages::COPY_DST | TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
    render_context.command_encoder().copy_texture_to_texture(
        source,
        TexelCopyTextureInfo {
            texture: &temp_texture,
            mip_level: 0,
            origin: Default::default(),
            aspect: TextureAspect::All,
        },
        size,
    );
    // 还原
    view_target.post_process_write();
    temp_texture.create_view(&TextureViewDescriptor::default())
}

fn get_render_pass<'a, 'w>(
    render_context: &'a mut RenderContext<'w>,
    view: &TextureView,
    label: &str,
) -> TrackedRenderPass<'a> {
    render_context.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: Operations::default(),
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
    })
}
