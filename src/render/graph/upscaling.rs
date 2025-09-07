use std::sync::Mutex;

use bevy::{
    core_pipeline::blit::{BlitPipeline, BlitPipelineKey},
    ecs::{
        component::Component,
        entity::Entity,
        system::{Commands, Query, Res, ResMut},
    },
    platform::collections::HashSet,
    render::{
        render_graph::ViewNode,
        render_resource::{
            BindGroup, BindGroupEntries, BlendState, CachedRenderPipelineId, PipelineCache,
            RenderPassDescriptor, SpecializedRenderPipelines, TextureViewId,
        },
    },
};

use crate::render::offscreen_texture::{ExtractedOffscreenTexture, ViewTarget};

#[derive(Default)]
pub struct OffscreenUpscalingNode {
    cached_texture_bind_group: Mutex<Option<(TextureViewId, BindGroup)>>,
}

impl ViewNode for OffscreenUpscalingNode {
    type ViewQuery = (
        &'static ViewTarget,
        &'static ViewUpscalingPipeline,
        &'static ExtractedOffscreenTexture,
    );

    fn run<'w>(
        &self,
        _graph: &mut bevy::render::render_graph::RenderGraphContext,
        render_context: &mut bevy::render::renderer::RenderContext<'w>,
        (target, upscaling_target, offscreen_texture): bevy::ecs::query::QueryItem<
            'w,
            Self::ViewQuery,
        >,
        world: &'w bevy::ecs::world::World,
    ) -> Result<(), bevy::render::render_graph::NodeRunError> {
        let pipeline_cache = world.get_resource::<PipelineCache>().unwrap();
        let blit_pipeline = world.get_resource::<BlitPipeline>().unwrap();

        let upscaled_texture = target.main_texture_view();
        let mut cached_bind_group = self.cached_texture_bind_group.lock().unwrap();
        let bind_group = match &mut *cached_bind_group {
            Some((id, bind_group)) if upscaled_texture.id() == *id => bind_group,
            cached_bind_group => {
                let bind_group = render_context.render_device().create_bind_group(
                    None,
                    &blit_pipeline.texture_bind_group,
                    &BindGroupEntries::sequential((upscaled_texture, &blit_pipeline.sampler)),
                );

                let (_, bind_group) = cached_bind_group.insert((upscaled_texture.id(), bind_group));
                bind_group
            }
        };
        let Some(pipeline) = pipeline_cache.get_render_pipeline(upscaling_target.0) else {
            return Ok(());
        };
        let pass_descriptor = RenderPassDescriptor {
            label: Some("upscaling_pass"),
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

        render_pass.set_pipeline(pipeline);
        render_pass.set_bind_group(0, bind_group, &[]);
        render_pass.draw(0..3, 0..1);
        Ok(())
    }
}

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
            texture_format: view_target.out_texture_format(),
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
