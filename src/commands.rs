use bevy::{
    asset::Handle,
    ecs::component::Component,
    math::Vec2,
    mesh::Mesh,
    prelude::{Deref, DerefMut},
    render::extract_component::ExtractComponent,
};

use crate::{
    render::{
        blend_pipeline::BlendMode,
        material::{BitmapMaterial, BlendMaterialKey, ColorMaterial, GradientMaterial},
    },
    swf_runtime::transform::Transform,
};

use swf::CharacterId;

#[derive(Debug)]
pub(crate) enum ShapeCommand {
    RenderShape {
        transform: Transform,
        id: CharacterId,
        shape_depth_layer: String,
        blend_mode: BlendMode,
        /// 通过ratio是否有值判断该Shape是否为形状补间
        ratio: Option<u16>,
    },
    RenderBitmap {
        bitmap_material: BitmapMaterial,
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
