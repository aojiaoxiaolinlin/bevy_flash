mod filter_driven_node;
mod filter_node;
mod main_transparent_pass_2d_node;
mod upscaling;

use bevy::{
    app::Plugin,
    asset::AssetId,
    ecs::{resource::Resource, schedule::IntoScheduleConfigs},
    mesh::Mesh,
    platform::collections::hash_map::Entry,
    prelude::{Deref, DerefMut},
    render::{
        Render, RenderApp, RenderSystems,
        graph::CameraDriverLabel,
        render_graph::{RenderGraph, RenderGraphExt, RenderLabel, RenderSubGraph, ViewNodeRunner},
        render_resource::CachedRenderPipelineId,
        sync_world::{MainEntity, MainEntityHashMap},
    },
    utils::default,
};
use filter_driven_node::{
    OffscreenTextureMultiPassPostProcessingDriverLabel,
    OffscreenTextureMultiPassPostProcessingDriverNode,
};

use crate::{
    assets::MaterialType,
    render::{
        graph::{
            filter_node::FilterPostProcessingNode,
            main_transparent_pass_2d_node::OffscreenMainTransparentPass2dNode,
            upscaling::OffscreenUpscalingNode,
        },
        material::{BitmapMaterial, GradientMaterial},
    },
};

#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderSubGraph)]
pub struct OffscreenCore2d;

#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderLabel)]
pub enum OffscreenNode2d {
    MainTransparentPass,
    FilterPostProcessing,
    Upscaling,
}

pub struct FlashFilterRenderPlugin;

impl Plugin for FlashFilterRenderPlugin {
    fn build(&self, app: &mut bevy::app::App) {
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app.add_systems(
            Render,
            upscaling::prepare_offscreen_view_upscaling_pipelines
                .in_set(RenderSystems::Prepare)
                .ambiguous_with_all(),
        );
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

impl From<&MaterialType> for DrawType {
    fn from(value: &MaterialType) -> Self {
        match value {
            MaterialType::Color(_) => DrawType::Color,
            MaterialType::Gradient(gradient) => DrawType::Gradient(gradient.id()),
            MaterialType::Bitmap(bitmap) => DrawType::Bitmap(bitmap.id()),
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
