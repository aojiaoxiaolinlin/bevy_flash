use bevy::{
    asset::{weak_handle, Handle},
    math::Vec2,
    prelude::{Component, Shader},
    render::{extract_component::ExtractComponent, render_resource::ShaderType},
};

pub const BLUR_FILTER_SHADER_HANDLE: Handle<Shader> =
    weak_handle!("f59e3d1c-7a24-4b8c-82a3-1d94e6f2c705");

#[derive(Component, ExtractComponent, ShaderType, Default, Clone, Copy)]
pub struct BlurFilterUniforms {
    pub direction: Vec2,
    pub full_size: f32,
    pub m: f32,
    pub first_weight: f32,
    pub last_offset: f32,
    pub last_weight: f32,
}
