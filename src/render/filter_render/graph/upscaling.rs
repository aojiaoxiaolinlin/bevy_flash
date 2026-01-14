use std::sync::Mutex;

use bevy::{
    core_pipeline::blit::BlitPipeline,
    render::{
        diagnostic::RecordDiagnostics,
        render_graph::ViewNode,
        render_resource::{BindGroup, PipelineCache, RenderPassDescriptor, TextureViewId},
    },
};

use crate::render::{
    filter_render::ViewUpscalingPipeline,
    offscreen_render::{ExtractedOffscreenCamera, ViewTarget},
};

#[derive(Default)]
pub struct OffscreenUpscalingNode {
    cached_texture_bind_group: Mutex<Option<(TextureViewId, BindGroup)>>,
}

impl ViewNode for OffscreenUpscalingNode {
    type ViewQuery = (
        &'static ViewTarget,
        &'static ViewUpscalingPipeline,
        &'static ExtractedOffscreenCamera,
    );

    fn run<'w>(
        &self,
        _graph: &mut bevy::render::render_graph::RenderGraphContext,
        render_context: &mut bevy::render::renderer::RenderContext<'w>,
        (target, upscaling_target, offscreen_texture): bevy::ecs::query::QueryItem<
            'w,
            '_,
            Self::ViewQuery,
        >,
        world: &'w bevy::ecs::world::World,
    ) -> Result<(), bevy::render::render_graph::NodeRunError> {
        let pipeline_cache = world.get_resource::<PipelineCache>().unwrap();
        let blit_pipeline = world.get_resource::<BlitPipeline>().unwrap();

        let diagnostics = render_context.diagnostic_recorder();

        let main_texture_view = target.main_texture_view();
        let mut bind_group = self.cached_texture_bind_group.lock().unwrap();
        let bind_group = match &mut *bind_group {
            Some((id, bind_group)) if main_texture_view.id() == *id => bind_group,
            cached_bind_group => {
                let bind_group = blit_pipeline.create_bind_group(
                    render_context.render_device(),
                    main_texture_view,
                    pipeline_cache,
                );

                let (_, bind_group) =
                    cached_bind_group.insert((main_texture_view.id(), bind_group));
                bind_group
            }
        };
        let Some(pipeline) = pipeline_cache.get_render_pipeline(upscaling_target.0) else {
            return Ok(());
        };
        let pass_descriptor = RenderPassDescriptor {
            label: Some("upscaling"),
            color_attachments: &[Some(
                target.out_texture_color_attachment(Some(offscreen_texture.clear_color.into())),
            )],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        };
        let mut render_pass = render_context
            .command_encoder()
            .begin_render_pass(&pass_descriptor);

        let pass_span = diagnostics.pass_span(&mut render_pass, "upscaling");

        render_pass.set_pipeline(pipeline);
        render_pass.set_bind_group(0, bind_group, &[]);
        render_pass.draw(0..3, 0..1);

        pass_span.end(&mut render_pass);

        Ok(())
    }
}
