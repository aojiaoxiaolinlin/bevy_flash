use bevy::render::render_graph::{Node, RenderLabel};

#[derive(RenderLabel, Clone, PartialEq, Eq, Debug, Hash)]
pub struct DefineShapeLabel;

#[derive(Default)]
pub struct DefineShapeNode;

impl Node for DefineShapeNode {
    fn run<'w>(
        &self,
        graph: &mut bevy::render::render_graph::RenderGraphContext,
        render_context: &mut bevy::render::renderer::RenderContext<'w>,
        world: &'w bevy::prelude::World,
    ) -> Result<(), bevy::render::render_graph::NodeRunError> {
        dbg!("DefineShapeNode::run");
        Ok(())
    }
}
