use bevy::{
    asset::{Asset, Handle},
    image::Image,
    math::Mat4,
    reflect::TypePath,
    render::render_resource::{AsBindGroup, ShaderType},
};
use bytemuck::{Pod, Zeroable};

#[derive(AsBindGroup, TypePath, Asset, Debug, Clone, Default)]
pub struct GradientMaterial {
    #[texture(0)]
    #[sampler(1)]
    pub texture: Handle<Image>,
    #[uniform(2)]
    pub gradient: GradientUniformsVab,
    #[uniform(3)]
    pub texture_transform: Mat4,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, ShaderType, Pod, Zeroable)]
pub struct GradientUniformsVab {
    pub focal_point: f32,
    pub interpolation: i32,
    pub shape: i32,
    pub repeat: i32,
    pub texture_transform: [f32; 6],
}

impl GradientUniformsVab {
    pub fn from_vab(gu: &vatf::GradientUniforms) -> Self {
        Self {
            focal_point: gu.focal_point,
            interpolation: gu.interpolation,
            shape: gu.shape,
            repeat: gu.repeat,
            texture_transform: gu.texture_transform,
        }
    }
}

#[derive(AsBindGroup, TypePath, Asset, Debug, Clone, Default)]
pub struct BitmapMaterial {
    #[texture(0)]
    #[sampler(1)]
    pub texture: Handle<Image>,
    #[uniform(2)]
    pub texture_transform: Mat4,
}
