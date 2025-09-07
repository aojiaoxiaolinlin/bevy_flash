use bevy::{
    ecs::{query::QueryState, world::World},
    render::render_graph::{Node, RenderLabel},
};

use crate::render::{ExtractedOffscreenTexture, offscreen_texture::SortedOffscreenTextures};

#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderLabel)]
pub struct OffscreenTextureMultiPassPostProcessingDriverLabel;

pub struct OffscreenTextureMultiPassPostProcessingDriverNode {
    offscreen_textures: QueryState<&'static ExtractedOffscreenTexture>,
}

impl OffscreenTextureMultiPassPostProcessingDriverNode {
    pub fn new(world: &mut World) -> Self {
        Self {
            offscreen_textures: world.query(),
        }
    }
}

impl Node for OffscreenTextureMultiPassPostProcessingDriverNode {
    fn update(&mut self, world: &mut World) {
        self.offscreen_textures.update_archetypes(world);
    }

    fn run<'w>(
        &self,
        graph: &mut bevy::render::render_graph::RenderGraphContext,
        _render_context: &mut bevy::render::renderer::RenderContext<'w>,
        world: &'w bevy::ecs::world::World,
    ) -> Result<(), bevy::render::render_graph::NodeRunError> {
        let sorted_offscreen_textures = world.resource::<SortedOffscreenTextures>();

        for sorted_offscreen_texture in &sorted_offscreen_textures.0 {
            let Ok(offscreen_texture) = self
                .offscreen_textures
                .get_manual(world, sorted_offscreen_texture.entity)
            else {
                continue;
            };
            graph.run_sub_graph(
                offscreen_texture.render_graph,
                vec![],
                Some(sorted_offscreen_texture.entity),
            )?;
        }
        Ok(())
    }
}
