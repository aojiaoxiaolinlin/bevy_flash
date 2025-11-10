use bevy::{
    asset::Handle,
    ecs::component::Component,
    math::Vec2,
    mesh::Mesh,
    prelude::{Deref, DerefMut},
    render::extract_component::ExtractComponent,
};

use crate::{
    assets::Shape,
    render::{blend_pipeline::BlendMode, material::BitmapMaterial},
    swf_runtime::transform::Transform,
};

#[derive(Debug, Clone)]
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
        blend_mode: BlendMode,
    },
}

/// 每帧所有Shape的绘制命令
#[derive(Component, Debug, Default, Deref, DerefMut)]
pub(crate) struct DrawShapes(pub Vec<ShapeCommand>);

#[derive(Component, Debug, Clone, Default, Deref, DerefMut, ExtractComponent)]
pub(crate) struct OffscreenDrawShapes(pub Vec<ShapeCommand>);
