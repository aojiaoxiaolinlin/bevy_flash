use bevy::{
    asset::{Asset, Handle, uuid_handle},
    image::Image,
    math::{Mat4, Vec4},
    reflect::TypePath,
    render::render_resource::{AsBindGroup, ShaderType},
    shader::{Shader, ShaderRef},
    sprite_render::{AlphaMode2d, Material2d},
};

use bytemuck::{Pod, Zeroable};

use swf::GradientSpread;

use crate::swf_runtime::{shape_utils::GradientType, tessellator::Gradient, transform::Transform};

pub const SWF_COLOR_MATERIAL_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("8c2a5b0f-3e6d-4f8a-b217-84d2f5e1c9b3");
pub const GRADIENT_MATERIAL_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("5e9f1a78-9b34-4c15-8d7e-2a3b0f47d862");
pub const BITMAP_MATERIAL_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("a34c7d82-1f5b-4a9e-93d8-6b7e20c45a1f");
pub const FLASH_COMMON_MATERIAL_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("e53b9f82-6a4c-4d5b-91e7-4f2a63b8c5d9");

bitflags::bitflags! {
    #[derive(Default, Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct BlendModelKey:u8 {
        const NORMAL                            = 0;
        const BLEND_ADD                         = 1 << 0;  // Additive blending
        const BLEND_SUBTRACT                    = 1 << 1;  // Subtractive blending
        const BLEND_SCREEN                      = 1 << 2;  // Screen blending
        const BLEND_LIGHTEN                     = 1 << 3;  // Lighten blending
        const BLEND_DARKEN                      = 1 << 4;  // Darken blending
        const BLEND_MULTIPLY                    = 1 << 5;  // Multiply blending
    }
}

macro_rules! material2d {
    ($name:ident, $shader:expr) => {
        impl Material2d for $name {
            fn vertex_shader() -> ShaderRef {
                $shader.into()
            }
            fn fragment_shader() -> ShaderRef {
                $shader.into()
            }
            fn alpha_mode(&self) -> AlphaMode2d {
                AlphaMode2d::Blend
            }
        }
    };
}

#[derive(AsBindGroup, TypePath, Asset, Debug, Clone, Default)]
pub struct GradientMaterial {
    #[texture(0)]
    #[sampler(1)]
    pub texture: Handle<Image>,
    #[uniform(2)]
    pub gradient: GradientUniforms,
    #[uniform(3)]
    pub texture_transform: Mat4,
}

material2d!(GradientMaterial, GRADIENT_MATERIAL_SHADER_HANDLE);

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, ShaderType, Pod, Zeroable)]
pub struct GradientUniforms {
    pub focal_point: f32,
    pub interpolation: i32,
    pub shape: i32,
    pub repeat: i32,
}
impl From<Gradient> for GradientUniforms {
    fn from(gradient: Gradient) -> Self {
        Self {
            focal_point: gradient.focal_point.to_f32().clamp(-0.98, 0.98),
            interpolation: (gradient.interpolation == swf::GradientInterpolation::LinearRgb) as i32,
            shape: match gradient.gradient_type {
                GradientType::Linear => 1,
                GradientType::Radial => 2,
                GradientType::Focal => 3,
            },
            repeat: match gradient.repeat_mode {
                GradientSpread::Pad => 1,
                GradientSpread::Reflect => 2,
                GradientSpread::Repeat => 3,
            },
        }
    }
}

#[derive(AsBindGroup, TypePath, Asset, Debug, Clone, Copy, Default)]
pub struct ColorMaterial {}

material2d!(ColorMaterial, SWF_COLOR_MATERIAL_SHADER_HANDLE);

#[derive(AsBindGroup, TypePath, Asset, Debug, Clone, Default)]
pub struct BitmapMaterial {
    #[texture(0)]
    #[sampler(1)]
    pub texture: Handle<Image>,
    #[uniform(2)]
    pub texture_transform: Mat4,
}

material2d!(BitmapMaterial, BITMAP_MATERIAL_SHADER_HANDLE);

#[derive(Debug, Clone, Copy, ShaderType)]
pub struct TransformUniform {
    pub world_matrix: Mat4,
    pub mult_color: Vec4,
    pub add_color: Vec4,
}

impl From<Transform> for TransformUniform {
    fn from(transform: Transform) -> Self {
        let matrix = transform.matrix;
        let color_transform = transform.color_transform;
        Self {
            world_matrix: Mat4::from_cols_array_2d(&[
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

impl Default for TransformUniform {
    fn default() -> Self {
        Self {
            world_matrix: Mat4::IDENTITY,
            mult_color: Vec4::ONE,
            add_color: Vec4::ZERO,
        }
    }
}
