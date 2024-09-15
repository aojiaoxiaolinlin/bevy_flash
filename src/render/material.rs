use bevy::{
    asset::{Asset, Handle},
    prelude::Image,
    reflect::TypePath,
    render::render_resource::{AsBindGroup, ShaderType},
    sprite::Material2d,
};
use glam::{Mat4, Vec4};
use ruffle_render::transform::Transform;

#[derive(AsBindGroup, TypePath, Asset, Debug, Clone, Default)]
pub struct GradientMaterial {
    #[uniform(0)]
    pub gradient: Gradient,
    #[texture(1)]
    #[sampler(2)]
    pub texture: Option<Handle<Image>>,
    #[uniform(3)]
    pub texture_transform: Mat4,
    #[uniform(4)]
    pub transform: SWFTransform,
}

impl Material2d for GradientMaterial {
    fn vertex_shader() -> bevy::render::render_resource::ShaderRef {
        "shaders/gradient.wgsl".into()
    }
    fn fragment_shader() -> bevy::render::render_resource::ShaderRef {
        "shaders/gradient.wgsl".into()
    }
}

#[derive(Debug, Clone, Default, ShaderType)]
pub struct Gradient {
    pub focal_point: f32,
    pub interpolation: i32,
    pub shape: i32,
    pub repeat: i32,
}

#[derive(AsBindGroup, TypePath, Asset, Debug, Clone, Default)]
pub struct SWFColorMaterial {
    #[uniform(0)]
    pub transform: SWFTransform,
}

impl Material2d for SWFColorMaterial {
    fn vertex_shader() -> bevy::render::render_resource::ShaderRef {
        "shaders/color.wgsl".into()
    }
    fn fragment_shader() -> bevy::render::render_resource::ShaderRef {
        "shaders/color.wgsl".into()
    }
}

#[derive(AsBindGroup, TypePath, Asset, Debug, Clone, Default)]
pub struct BitmapMaterial {
    #[texture(0)]
    #[sampler(1)]
    pub texture: Handle<Image>,
    #[uniform(2)]
    pub texture_transform: Mat4,
    #[uniform(3)]
    pub transform: SWFTransform,
}

impl Material2d for BitmapMaterial {
    fn vertex_shader() -> bevy::render::render_resource::ShaderRef {
        "shaders/bitmap.wgsl".into()
    }
    fn fragment_shader() -> bevy::render::render_resource::ShaderRef {
        "shaders/bitmap.wgsl".into()
    }
}

#[derive(Debug, Clone, Default, ShaderType)]
pub struct SWFTransform {
    world_transform: Mat4,
    mult_color: Vec4,
    add_color: Vec4,
}

impl From<Transform> for SWFTransform {
    fn from(transform: Transform) -> Self {
        let matrix = transform.matrix;
        let color_transform = transform.color_transform;
        SWFTransform {
            world_transform: Mat4::from_cols_array_2d(&[
                [matrix.a, matrix.b, 0.0, 0.0],
                [matrix.c, matrix.d, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [
                    matrix.tx.to_pixels() as f32,
                    matrix.ty.to_pixels() as f32,
                    0.0,
                    1.0,
                ],
            ]),
            mult_color: Vec4::from_array(color_transform.mult_rgba_normalized()),
            add_color: Vec4::from_array(color_transform.add_rgba_normalized()),
        }
    }
}
