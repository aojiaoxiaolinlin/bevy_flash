use bevy::{
    asset::{Asset, Handle},
    math::{Mat4, Vec3, Vec4},
    prelude::Image,
    reflect::TypePath,
    render::render_resource::{AsBindGroup, ShaderType},
    sprite::{AlphaMode2d, Material2d},
};
use bytemuck::{Pod, Zeroable};
use ruffle_render::{
    shape_utils::GradientType, tessellator::Gradient, transform::Transform as RuffleTransform,
};
use swf::GradientSpread;
use swf_macro::SwfMaterial;

use super::{
    BITMAP_MATERIAL_SHADER_HANDLE, GRADIENT_MATERIAL_SHADER_HANDLE,
    SWF_COLOR_MATERIAL_SHADER_HANDLE,
};

pub trait SwfMaterial: AsBindGroup + TypePath + Asset + Material2d + Clone {
    fn update_swf_material(&mut self, swf_transform: SwfTransform);
    fn world_transform(&self) -> Mat4;
    fn set_alpha_mode2d(&mut self, alpha_mode2d: AlphaMode2d);
}

#[derive(AsBindGroup, TypePath, Asset, Debug, Clone, Default, SwfMaterial)]
pub struct GradientMaterial {
    pub alpha_mode2d: AlphaMode2d,
    #[uniform(0)]
    pub gradient: GradientUniforms,
    #[texture(1)]
    #[sampler(2)]
    pub texture: Option<Handle<Image>>,
    #[uniform(3)]
    pub texture_transform: Mat4,
    #[uniform(4)]
    pub transform: SwfTransform,
}

impl Material2d for GradientMaterial {
    fn vertex_shader() -> bevy::render::render_resource::ShaderRef {
        GRADIENT_MATERIAL_SHADER_HANDLE.into()
    }
    fn fragment_shader() -> bevy::render::render_resource::ShaderRef {
        GRADIENT_MATERIAL_SHADER_HANDLE.into()
    }
    fn alpha_mode(&self) -> AlphaMode2d {
        self.alpha_mode2d
    }
}

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

#[derive(AsBindGroup, TypePath, Asset, Debug, Clone, Default, SwfMaterial)]
pub struct SwfColorMaterial {
    pub alpha_mode2d: AlphaMode2d,
    #[uniform(0)]
    pub transform: SwfTransform,
}

impl Material2d for SwfColorMaterial {
    fn vertex_shader() -> bevy::render::render_resource::ShaderRef {
        SWF_COLOR_MATERIAL_SHADER_HANDLE.into()
    }
    fn fragment_shader() -> bevy::render::render_resource::ShaderRef {
        SWF_COLOR_MATERIAL_SHADER_HANDLE.into()
    }
    fn alpha_mode(&self) -> AlphaMode2d {
        self.alpha_mode2d
    }
}

#[derive(AsBindGroup, TypePath, Asset, Debug, Clone, Default, SwfMaterial)]
pub struct BitmapMaterial {
    pub alpha_mode2d: AlphaMode2d,
    #[texture(0)]
    #[sampler(1)]
    pub texture: Handle<Image>,
    #[uniform(2)]
    pub texture_transform: Mat4,
    #[uniform(3)]
    pub transform: SwfTransform,
}

impl Material2d for BitmapMaterial {
    fn vertex_shader() -> bevy::render::render_resource::ShaderRef {
        BITMAP_MATERIAL_SHADER_HANDLE.into()
    }
    fn fragment_shader() -> bevy::render::render_resource::ShaderRef {
        BITMAP_MATERIAL_SHADER_HANDLE.into()
    }
    fn alpha_mode(&self) -> AlphaMode2d {
        self.alpha_mode2d
    }
}

#[derive(Debug, Clone, Default, ShaderType)]
pub struct SwfTransform {
    pub world_transform: Mat4,
    pub mult_color: Vec4,
    pub add_color: Vec4,
}

impl SwfTransform {
    pub fn scale(&self) -> Vec3 {
        Vec3::new(
            self.world_transform.x_axis.x,
            self.world_transform.y_axis.y,
            1.0,
        )
    }
}

impl From<RuffleTransform> for SwfTransform {
    fn from(transform: RuffleTransform) -> Self {
        let matrix = transform.matrix;
        let color_transform = transform.color_transform;
        SwfTransform {
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
