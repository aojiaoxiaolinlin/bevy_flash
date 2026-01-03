mod filter_driven_node;
mod filter_node;
mod main_transparent_pass_2d_node;
mod upscaling;

use bevy::{
    app::{App, Plugin},
    render::{
        RenderApp,
        graph::CameraDriverLabel,
        render_graph::{RenderGraph, RenderGraphExt, RenderLabel, RenderSubGraph, ViewNodeRunner},
    },
};

use self::{
    filter_driven_node::{
        OffscreenTextureMultiPassPostProcessingDriverLabel,
        OffscreenTextureMultiPassPostProcessingDriverNode,
    },
    filter_node::FilterPostProcessingNode,
    main_transparent_pass_2d_node::OffscreenMainTransparentPass2dNode,
    upscaling::OffscreenUpscalingNode,
};

#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderSubGraph)]
pub struct OffscreenCore2d;

#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderLabel)]
pub enum OffscreenNode2d {
    MainTransparentPass,
    FilterPostProcessing,
    Upscaling,
}

pub struct SwfFilterRenderGraphPlugin;

impl Plugin for SwfFilterRenderGraphPlugin {
    fn build(&self, app: &mut App) {
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };

        render_app
            .add_render_sub_graph(OffscreenCore2d)
            .add_render_graph_node::<ViewNodeRunner<OffscreenMainTransparentPass2dNode>>(
                OffscreenCore2d,
                OffscreenNode2d::MainTransparentPass,
            )
            .add_render_graph_node::<ViewNodeRunner<FilterPostProcessingNode>>(
                OffscreenCore2d,
                OffscreenNode2d::FilterPostProcessing,
            )
            .add_render_graph_node::<ViewNodeRunner<OffscreenUpscalingNode>>(
                OffscreenCore2d,
                OffscreenNode2d::Upscaling,
            )
            .add_render_graph_edges(
                OffscreenCore2d,
                (
                    OffscreenNode2d::MainTransparentPass,
                    OffscreenNode2d::FilterPostProcessing,
                    OffscreenNode2d::Upscaling,
                ),
            );
        let offscreen_texture_multi_pass_post_processing_driver_node =
            OffscreenTextureMultiPassPostProcessingDriverNode::new(render_app.world_mut());
        let mut render_graph = render_app.world_mut().resource_mut::<RenderGraph>();
        render_graph.add_node(
            OffscreenTextureMultiPassPostProcessingDriverLabel,
            offscreen_texture_multi_pass_post_processing_driver_node,
        );
        render_graph.add_node_edge(
            OffscreenTextureMultiPassPostProcessingDriverLabel,
            CameraDriverLabel,
        );
    }
}
