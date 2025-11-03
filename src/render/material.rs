use bevy::{
    asset::{Asset, Handle, uuid_handle},
    math::{Mat4, Vec3, Vec4},
    prelude::Image,
    reflect::TypePath,
    render::render_resource::{
        AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, ShaderType,
    },
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

pub trait SwfMaterial: AsBindGroup + TypePath + Asset + Material2d + Clone {
    fn set_blend_key(&mut self, blend_key: BlendMaterialKey);
}

bitflags::bitflags! {
    #[derive(Default, Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct BlendMaterialKey:u8 {
        const NORMAL                            = 0;
        const BLEND_ADD                         = 1 << 0;  // Additive blending
        const BLEND_SUBTRACT                    = 1 << 1;  // Subtractive blending
        const BLEND_SCREEN                      = 1 << 2;  // Screen blending
        const BLEND_LIGHTEN                     = 1 << 3;  // Lighten blending
        const BLEND_DARKEN                      = 1 << 4;  // Darken blending
        const BLEND_MULTIPLY                    = 1 << 5;  // Multiply blending
    }
}

impl From<&ColorMaterial> for BlendMaterialKey {
    fn from(value: &ColorMaterial) -> Self {
        value.blend_key
    }
}

impl From<&GradientMaterial> for BlendMaterialKey {
    fn from(value: &GradientMaterial) -> Self {
        value.blend_key
    }
}

impl From<&BitmapMaterial> for BlendMaterialKey {
    fn from(value: &BitmapMaterial) -> Self {
        value.blend_key
    }
}

macro_rules! swf_material {
    ($name:ident) => {
        impl SwfMaterial for $name {
            fn set_blend_key(&mut self, blend_key: BlendMaterialKey) {
                self.blend_key = blend_key;
            }
        }
    };
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

            fn specialize(
                descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
                _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
                key: bevy::sprite_render::Material2dKey<Self>,
            ) -> bevy::ecs::error::Result<
                (),
                bevy::render::render_resource::SpecializedMeshPipelineError,
            > {
                if let Some(fragment) = &mut descriptor.fragment {
                    if let Some(target) = &mut fragment.targets[0] {
                        if key.bind_group_data.contains(BlendMaterialKey::BLEND_ADD) {
                            target.blend = Some(BlendState {
                                color: BlendComponent {
                                    src_factor: BlendFactor::One,
                                    dst_factor: BlendFactor::One,
                                    operation: BlendOperation::Add,
                                },
                                alpha: BlendComponent::OVER,
                            });
                        } else if key
                            .bind_group_data
                            .contains(BlendMaterialKey::BLEND_MULTIPLY)
                        {
                            target.blend = Some(BlendState {
                                color: BlendComponent {
                                    src_factor: BlendFactor::Dst,
                                    dst_factor: BlendFactor::OneMinusSrcAlpha,
                                    operation: BlendOperation::Add,
                                },
                                alpha: BlendComponent::OVER,
                            });
                        } else if key
                            .bind_group_data
                            .contains(BlendMaterialKey::BLEND_SUBTRACT)
                        {
                            target.blend = Some(BlendState {
                                color: BlendComponent {
                                    src_factor: BlendFactor::One,
                                    dst_factor: BlendFactor::One,
                                    operation: BlendOperation::ReverseSubtract,
                                },
                                alpha: BlendComponent::OVER,
                            });
                        } else if key.bind_group_data.contains(BlendMaterialKey::BLEND_SCREEN) {
                            target.blend = Some(BlendState {
                                color: BlendComponent {
                                    src_factor: BlendFactor::One,
                                    dst_factor: BlendFactor::OneMinusSrc,
                                    operation: BlendOperation::Add,
                                },
                                alpha: BlendComponent::OVER,
                            });
                        } else if key
                            .bind_group_data
                            .contains(BlendMaterialKey::BLEND_LIGHTEN)
                        {
                            target.blend = Some(BlendState {
                                color: BlendComponent {
                                    src_factor: BlendFactor::One,
                                    dst_factor: BlendFactor::One,
                                    operation: BlendOperation::Max,
                                },
                                alpha: BlendComponent::OVER,
                            });
                        } else if key.bind_group_data.contains(BlendMaterialKey::BLEND_DARKEN) {
                            target.blend = Some(BlendState {
                                color: BlendComponent {
                                    src_factor: BlendFactor::One,
                                    dst_factor: BlendFactor::One,
                                    operation: BlendOperation::Min,
                                },
                                alpha: BlendComponent::OVER,
                            });
                        } else {
                            // Flash 中是预乘Alpha混合
                            target.blend = Some(BlendState::PREMULTIPLIED_ALPHA_BLENDING)
                        }
                    }
                }
                Ok(())
            }
        }
    };
}

#[derive(AsBindGroup, TypePath, Asset, Debug, Clone, Default)]
#[bind_group_data(BlendMaterialKey)]
pub struct GradientMaterial {
    #[texture(0)]
    #[sampler(1)]
    pub texture: Handle<Image>,
    #[uniform(2)]
    pub gradient: GradientUniforms,
    #[uniform(3)]
    pub texture_transform: Mat4,
    pub blend_key: BlendMaterialKey,
}

material2d!(GradientMaterial, GRADIENT_MATERIAL_SHADER_HANDLE);
swf_material!(GradientMaterial);

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
#[bind_group_data(BlendMaterialKey)]
pub struct ColorMaterial {
    pub blend_key: BlendMaterialKey,
}

material2d!(ColorMaterial, SWF_COLOR_MATERIAL_SHADER_HANDLE);
swf_material!(ColorMaterial);

#[derive(AsBindGroup, TypePath, Asset, Debug, Clone, Default)]
#[bind_group_data(BlendMaterialKey)]
pub struct BitmapMaterial {
    #[texture(0)]
    #[sampler(1)]
    pub texture: Handle<Image>,
    #[uniform(2)]
    pub texture_transform: Mat4,
    pub blend_key: BlendMaterialKey,
}

material2d!(BitmapMaterial, BITMAP_MATERIAL_SHADER_HANDLE);
swf_material!(BitmapMaterial);

#[derive(Debug, Clone, Copy, ShaderType)]
pub struct MaterialTransform {
    pub world_transform: Mat4,
    pub mult_color: Vec4,
    pub add_color: Vec4,
}

impl From<Transform> for MaterialTransform {
    fn from(transform: Transform) -> Self {
        let matrix = transform.matrix;
        let color_transform = transform.color_transform;
        Self {
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

impl Default for MaterialTransform {
    fn default() -> Self {
        Self {
            world_transform: Mat4::IDENTITY,
            mult_color: Vec4::ONE,
            add_color: Vec4::ZERO,
        }
    }
}
