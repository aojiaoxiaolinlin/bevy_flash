use bevy::{
    app::{App, Plugin},
    asset::{load_internal_asset, Handle},
    core_pipeline::tonemapping::get_lut_bind_group_layout_entries,
    prelude::{FromWorld, Resource},
    render::{
        render_resource::{
            binding_types::uniform_buffer, BindGroupLayout, BindGroupLayoutEntries,
            IntoBindGroupLayoutEntryBuilder, Shader, ShaderStages, SpecializedRenderPipeline,
        },
        renderer::RenderDevice,
        view::ViewUniform,
    },
};
use ruffle_render_wgpu::Transforms;
/// 使用UUID指定,SWF着色器Handle
pub const SWF_HANDLE: Handle<Shader> =
    Handle::weak_from_u128(251354789657743035148351631714426867038);

pub(crate) mod commands;

pub struct FlashRenderPlugin;

impl Plugin for FlashRenderPlugin {
    fn build(&self, app: &mut App) {}
}
#[derive(Resource)]
pub struct FlashPipeline {
    pub globals: BindGroupLayout,
    pub transforms: BindGroupLayout,
    pub bitmap: BindGroupLayout,
    pub gradient: BindGroupLayout,
    pub blend: BindGroupLayout,
}

impl FromWorld for FlashPipeline {
    fn from_world(world: &mut bevy::prelude::World) -> Self {
        let render_device = world.resource::<RenderDevice>();
        let tonemapping_lut_entries = get_lut_bind_group_layout_entries();

        let _view_layout = render_device.create_bind_group_layout(
            Some("Flash view layout"),
            &BindGroupLayoutEntries::with_indices(
                ShaderStages::VERTEX,
                (
                    (0, uniform_buffer::<ViewUniform>(true)),
                    (
                        1,
                        tonemapping_lut_entries[0].visibility(ShaderStages::FRAGMENT),
                    ),
                    (
                        2,
                        tonemapping_lut_entries[1].visibility(ShaderStages::FRAGMENT),
                    ),
                ),
            ),
        );

        let transforms = render_device.create_bind_group_layout(
            Some("Uniform buffer bind group layout"),
            &BindGroupLayoutEntries::sequential(
                ShaderStages::VERTEX_FRAGMENT,
                (wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: wgpu::BufferSize::new(
                        std::mem::size_of::<Transforms>() as u64
                    ),
                }
                .into_bind_group_layout_entry_builder(),),
            ),
        );
        let globals = render_device.create_bind_group_layout(
            Some("Global bind group layout"),
            &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<
                        ruffle_render_wgpu::globals::GlobalsUniform,
                    >() as u64),
                },
                count: None,
            }],
        );
        let bitmap = render_device.create_bind_group_layout(
            Some("Bitmap shape bind group layout"),
            &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<
                            ruffle_render_wgpu::TextureTransforms,
                        >() as u64),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        );

        let blend = render_device.create_bind_group_layout(
            Some("Blend bind group layout"),
            &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                    count: None,
                },
            ],
        );
        let gradient = render_device.create_bind_group_layout(
            Some("Gradient bind group layout"),
            &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<
                            ruffle_render_wgpu::TextureTransforms,
                        >() as u64),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<
                            ruffle_render_wgpu::GradientUniforms,
                        >() as u64),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        );

        Self {
            transforms,
            globals,
            bitmap,
            gradient,
            blend,
        }
    }
}

bitflags::bitflags! {
    #[derive(Debug, Hash, PartialEq, Eq, Clone, Copy)]
    #[repr(transparent)]
    pub struct FlashPipelineKey: u32 {
        const NONE = 0;
    }
}
impl SpecializedRenderPipeline for FlashPipeline {
    type Key = FlashPipelineKey;

    fn specialize(
        &self,
        key: Self::Key,
    ) -> bevy::render::render_resource::RenderPipelineDescriptor {
        todo!()
    }
}
