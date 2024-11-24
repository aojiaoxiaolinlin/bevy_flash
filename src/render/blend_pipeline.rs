use std::marker::PhantomData;

use bevy::{
    asset::{AssetServer, Handle},
    prelude::{FromWorld, Resource, Shader, World},
    render::{
        render_resource::{BindGroupLayout, ShaderRef},
        renderer::RenderDevice,
    },
    sprite::{Material2d, Mesh2dPipeline, Mesh2dPipelineKey},
};
use enum_map::Enum;
use ruffle_render::blend::ExtendedBlendMode;

#[derive(Enum, Debug, Copy, Clone)]
pub enum TrivialBlend {
    Normal,
    Add,
    Subtract,
    Screen,
}

#[derive(Enum, Debug, Copy, Clone)]
pub enum ComplexBlend {
    Multiply,   // Can't be trivial, 0 alpha is special case
    Lighten,    // Might be trivial but I can't reproduce the right colors
    Darken,     // Might be trivial but I can't reproduce the right colors
    Difference, // Can't be trivial, relies on abs operation
    Invert,     // May be trivial using a constant? Hard because it's without premultiplied alpha
    Alpha,      // Can't be trivial, requires layer tracking
    Erase,      // Can't be trivial, requires layer tracking
    Overlay,    // Can't be trivial, big math expression
    HardLight,  // Can't be trivial, big math expression
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum BlendType {
    Trivial(TrivialBlend),
    Complex(ComplexBlend),
}

impl BlendType {
    pub fn from(mode: ExtendedBlendMode) -> BlendType {
        match mode {
            ExtendedBlendMode::Normal => BlendType::Trivial(TrivialBlend::Normal),
            ExtendedBlendMode::Layer => BlendType::Trivial(TrivialBlend::Normal),
            ExtendedBlendMode::Add => BlendType::Trivial(TrivialBlend::Add),
            ExtendedBlendMode::Subtract => BlendType::Trivial(TrivialBlend::Subtract),
            ExtendedBlendMode::Screen => BlendType::Trivial(TrivialBlend::Screen),
            ExtendedBlendMode::Alpha => BlendType::Complex(ComplexBlend::Alpha),
            ExtendedBlendMode::Multiply => BlendType::Complex(ComplexBlend::Multiply),
            ExtendedBlendMode::Lighten => BlendType::Complex(ComplexBlend::Lighten),
            ExtendedBlendMode::Darken => BlendType::Complex(ComplexBlend::Darken),
            ExtendedBlendMode::Difference => BlendType::Complex(ComplexBlend::Difference),
            ExtendedBlendMode::Invert => BlendType::Complex(ComplexBlend::Invert),
            ExtendedBlendMode::Erase => BlendType::Complex(ComplexBlend::Erase),
            ExtendedBlendMode::Overlay => BlendType::Complex(ComplexBlend::Overlay),
            ExtendedBlendMode::HardLight => BlendType::Complex(ComplexBlend::HardLight),
            ExtendedBlendMode::Shader => unreachable!(),
        }
    }
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    #[repr(transparent)]
    pub struct Mesh2dBlendPipelineKey:u32 {
        const NONE = 0;
        const NORMAL = 1 << 0;
        const ADD = 1 << 1;
        const SUBTRACT = 1 << 2;
        const SCREEN = 1 << 3;
    }
}
#[allow(dead_code)]
pub struct Mesh2dBlendPipeline {
    mesh2d_pipeline: Mesh2dPipeline,
}
impl FromWorld for Mesh2dBlendPipeline {
    fn from_world(world: &mut bevy::prelude::World) -> Self {
        Self {
            mesh2d_pipeline: Mesh2dPipeline::from_world(world),
        }
    }
}

#[derive(Clone, Eq, PartialEq, Hash)]
pub struct Material2dBlendKey<M: Material2d> {
    pub mesh_key: Mesh2dPipelineKey,
    pub blend_key: Mesh2dBlendPipelineKey,
    pub bind_group_data: M::Data,
}

#[derive(Resource)]
pub struct Material2dPipeline<M: Material2d> {
    pub mesh2d_pipeline: Mesh2dPipeline,
    pub material2d_layout: BindGroupLayout,
    pub vertex_shader: Option<Handle<Shader>>,
    pub fragment_shader: Option<Handle<Shader>>,
    marker: PhantomData<M>,
}

impl<M: Material2d> Clone for Material2dPipeline<M> {
    fn clone(&self) -> Self {
        Self {
            mesh2d_pipeline: self.mesh2d_pipeline.clone(),
            material2d_layout: self.material2d_layout.clone(),
            vertex_shader: self.vertex_shader.clone(),
            fragment_shader: self.fragment_shader.clone(),
            marker: PhantomData,
        }
    }
}

impl<M: Material2d> FromWorld for Material2dPipeline<M> {
    fn from_world(world: &mut World) -> Self {
        let asset_server = world.resource::<AssetServer>();
        let render_device = world.resource::<RenderDevice>();
        let material2d_layout = M::bind_group_layout(render_device);

        Material2dPipeline {
            mesh2d_pipeline: world.resource::<Mesh2dPipeline>().clone(),
            material2d_layout,
            vertex_shader: match M::vertex_shader() {
                ShaderRef::Default => None,
                ShaderRef::Handle(handle) => Some(handle),
                ShaderRef::Path(path) => Some(asset_server.load(path)),
            },
            fragment_shader: match M::fragment_shader() {
                ShaderRef::Default => None,
                ShaderRef::Handle(handle) => Some(handle),
                ShaderRef::Path(path) => Some(asset_server.load(path)),
            },
            marker: PhantomData,
        }
    }
}
