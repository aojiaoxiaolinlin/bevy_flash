use bevy::{
    asset::Handle,
    ecs::component::Component,
    math::Vec2,
    mesh::Mesh,
    prelude::{Deref, DerefMut},
    render::extract_component::ExtractComponent,
};

use crate::{
    assets::{MaterialType, Shape},
    render::{
        blend_pipeline::BlendMode,
        material::{BitmapMaterial, BlendMaterialKey},
    },
    swf_runtime::transform::Transform,
};

#[derive(Debug)]
pub(crate) enum ShapeCommand {
    RenderShape {
        draw_shape: Shape,
        transform: Transform,
        blend_mode: BlendMode,
    },
    RenderBitmap {
        mesh: Handle<Mesh>,
        material: Handle<BitmapMaterial>,
        transform: Transform,
        size: Vec2,
    },
}

/// 每帧所有Shape的绘制命令
#[derive(Component, Debug, Default, Deref, DerefMut)]
pub(crate) struct DrawShapes(pub Vec<ShapeCommand>);

#[derive(Debug, Clone)]
pub struct ShapeMeshDraw {
    pub material_type: MaterialType,
    pub mesh: Handle<Mesh>,
    pub blend: BlendMaterialKey,
}

#[derive(Component, Debug, Clone, Default, Deref, DerefMut, ExtractComponent)]
pub struct OffscreenDrawCommands(pub Vec<ShapeMeshDraw>);
