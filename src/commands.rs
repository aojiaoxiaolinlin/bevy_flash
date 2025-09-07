use bevy::{
    asset::Handle,
    ecs::component::Component,
    math::Vec2,
    prelude::{Deref, DerefMut},
    render::{extract_component::ExtractComponent, mesh::Mesh},
};
use swf::CharacterId;

use crate::{
    render::{
        blend_pipeline::BlendMode,
        material::{BitmapMaterial, BlendMaterialKey, ColorMaterial, GradientMaterial},
    },
    swf_runtime::transform::Transform,
};

#[derive(Debug)]
pub(crate) enum ShapeCommand {
    RenderShape {
        transform: Transform,
        // Graphic 对应的 CharacterId
        id: CharacterId,
        shape_depth_layer: String,
        blend_mode: BlendMode,
    },
    RenderBitmap {
        bitmap_material: BitmapMaterial,
        // Bitmap 对应的 CharacterId
        id: CharacterId,
        shape_depth_layer: String,
        size: Vec2,
    },
}

#[derive(Debug, Clone)]
pub enum MaterialType {
    Color(Handle<ColorMaterial>),
    Gradient(Handle<GradientMaterial>),
    Bitmap(Handle<BitmapMaterial>),
}

#[derive(Debug, Clone)]
pub struct ShapeMeshDraw {
    pub material_type: MaterialType,
    pub mesh: Handle<Mesh>,
    pub blend: BlendMaterialKey,
}

#[derive(Component, Debug, Clone, Default, Deref, DerefMut, ExtractComponent)]
pub struct OffscreenDrawCommands(pub Vec<ShapeMeshDraw>);
