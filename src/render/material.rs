use bevy::{
    asset::{Asset, Handle},
    ecs::system::lifetimeless::SRes,
    math::{Mat4, Vec3, Vec4},
    prelude::Image,
    reflect::TypePath,
    render::{
        render_asset::{PrepareAssetError, RenderAsset},
        render_phase::{DrawFunctions, SetItemPipeline},
        render_resource::{
            AsBindGroup, BindGroup, BindingResources, BlendComponent, BlendFactor, BlendOperation,
            BlendState, ShaderType,
        },
        renderer::RenderDevice,
    },
    sprite::{
        AlphaMode2d, DrawMesh2d, Material2d, Material2dBindGroupId, Material2dPipeline,
        Material2dProperties, Mesh2dPipelineKey, SetMaterial2dBindGroup, SetMesh2dBindGroup,
        SetMesh2dViewBindGroup, alpha_mode_pipeline_key,
    },
};
use bytemuck::{Pod, Zeroable};

use swf::GradientSpread;

use crate::{
    render::sort_item::OffscreenTransparent2d,
    swf_runtime::{shape_utils::GradientType, tessellator::Gradient, transform::Transform},
};

use super::{
    BITMAP_MATERIAL_SHADER_HANDLE, GRADIENT_MATERIAL_SHADER_HANDLE,
    SWF_COLOR_MATERIAL_SHADER_HANDLE,
};

pub trait SwfMaterial: AsBindGroup + TypePath + Asset + Material2d + Clone {
    fn update_swf_material(&mut self, swf_transform: MaterialTransform);
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
            fn update_swf_material(&mut self, swf_transform: MaterialTransform) {
                self.transform = swf_transform;
            }
            fn set_blend_key(&mut self, blend_key: BlendMaterialKey) {
                self.blend_key = blend_key;
            }
        }
    };
}

macro_rules! material2d {
    ($name:ident, $shader:expr) => {
        impl Material2d for $name {
            fn vertex_shader() -> bevy::render::render_resource::ShaderRef {
                $shader.into()
            }
            fn fragment_shader() -> bevy::render::render_resource::ShaderRef {
                $shader.into()
            }
            fn alpha_mode(&self) -> AlphaMode2d {
                AlphaMode2d::Blend
            }

            fn specialize(
                descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
                _layout: &bevy::render::mesh::MeshVertexBufferLayoutRef,
                key: bevy::sprite::Material2dKey<Self>,
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
    #[uniform(4)]
    pub transform: MaterialTransform,
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
    #[uniform(0)]
    pub transform: MaterialTransform,
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
    #[uniform(3)]
    pub transform: MaterialTransform,
    pub blend_key: BlendMaterialKey,
}

material2d!(BitmapMaterial, BITMAP_MATERIAL_SHADER_HANDLE);
swf_material!(BitmapMaterial);

#[derive(Debug, Clone, Copy, Default, ShaderType)]
pub struct MaterialTransform {
    pub world_transform: Mat4,
    pub mult_color: Vec4,
    pub add_color: Vec4,
}

impl MaterialTransform {
    pub fn scale(&self) -> Vec3 {
        Vec3::new(
            self.world_transform.x_axis.x,
            self.world_transform.y_axis.y,
            1.0,
        )
    }
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

// 以是offscreen渲染的material

// 以下是offscreen渲染一些自定义尝试，当前未使用。
pub(super) type OffscreenDrawMaterial2d<M> = (
    SetItemPipeline,
    SetMesh2dBindGroup<1>,
    SetMaterial2dBindGroup<M, 2>,
    DrawMesh2d,
);

pub struct PreparedOffscreenMaterial2d<T: Material2d> {
    pub bindings: BindingResources,
    pub bind_group: BindGroup,
    pub key: T::Data,
    pub properties: Material2dProperties,
}

impl<T: Material2d> PreparedOffscreenMaterial2d<T> {
    pub fn get_bind_group_id(&self) -> Material2dBindGroupId {
        Material2dBindGroupId(Some(self.bind_group.id()))
    }
}

impl<M: Material2d> RenderAsset for PreparedOffscreenMaterial2d<M> {
    type SourceAsset = M;

    type Param = (
        SRes<RenderDevice>,
        SRes<Material2dPipeline<M>>,
        SRes<DrawFunctions<OffscreenTransparent2d>>,
        M::Param,
    );

    fn prepare_asset(
        material: Self::SourceAsset,
        _: bevy::asset::AssetId<Self::SourceAsset>,
        (
            render_device,
            pipeline,
            transparent_draw_functions,
            material_param,
        ): &mut bevy::ecs::system::SystemParamItem<Self::Param>,
    ) -> Result<Self, bevy::render::render_asset::PrepareAssetError<Self::SourceAsset>> {
        match material.as_bind_group(&pipeline.material2d_layout, render_device, material_param) {
            Ok(prepared) => {
                let mut mesh_pipeline_key_bits = Mesh2dPipelineKey::empty();
                mesh_pipeline_key_bits.insert(alpha_mode_pipeline_key(material.alpha_mode()));

                let draw_function_id = match material.alpha_mode() {
                    AlphaMode2d::Blend => transparent_draw_functions
                        .read()
                        .id::<OffscreenDrawMaterial2d<M>>(),
                    _ => {
                        return Err(PrepareAssetError::AsBindGroupError(
                            bevy::render::render_resource::AsBindGroupError::CreateBindGroupDirectly,
                        ));
                    }
                };

                Ok(PreparedOffscreenMaterial2d {
                    bindings: prepared.bindings,
                    bind_group: prepared.bind_group,
                    key: prepared.data,
                    properties: Material2dProperties {
                        depth_bias: material.depth_bias(),
                        alpha_mode: material.alpha_mode(),
                        mesh_pipeline_key_bits,
                        draw_function_id,
                    },
                })
            }

            Err(other) => Err(PrepareAssetError::AsBindGroupError(other)),
        }
    }
}
