use super::{
    OffscreenViewTarget,
    extract::{ExtractedFrame, Op},
    texture_cache::{FrameInternalTextureCache, TextureAllocationSite, TexturePurpose},
};
use anyhow::{Context as ContextExt, Result, ensure};
use bevy::{
    asset::AssetId,
    mesh::MeshVertexBufferLayoutRef,
    platform::collections::{HashMap, HashSet},
    prelude::*,
    render::{
        mesh::{RenderMesh, RenderMeshBufferInfo, allocator::MeshAllocator},
        render_asset::RenderAssets,
        render_resource::*,
        renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery},
        texture::GpuImage,
        view::Msaa,
    },
};
use bytemuck::{Pod, Zeroable};
use std::{borrow::Cow, num::NonZeroU64, sync::atomic::AtomicU64};
use vatf::animation::{
    AnimBevelFilter, AnimBlurFilter, AnimColorMatrixFilter, AnimConvolutionFilter,
    AnimDropShadowFilter, AnimFilter, AnimGlowFilter, AnimGradientFilter, AnimGradientRecord,
};

use crate::vab_asset::VabBlendMode;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct DrawUniform {
    row0: [f32; 4],
    row1: [f32; 4],
    uv0: [f32; 4],
    uv1: [f32; 4],
    multiply: [f32; 4],
    add: [f32; 4],
    material: [f32; 4],
    viewport: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CompositeUniform {
    rect: [f32; 4],
    viewport: [f32; 4],
    mode: [f32; 4],
    color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct BlurUniform {
    direction: [f32; 2],
    full_size: f32,
    m: f32,
    m2: f32,
    first_weight: f32,
    last_offset: f32,
    last_weight: f32,
    padding: [[f32; 4]; 6],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GlowUniform {
    color: [f32; 4],
    strength: f32,
    inner: u32,
    knockout: u32,
    composite_source: u32,
    blur_offset: [f32; 2],
    offset_padding: [f32; 2],
    padding: [[f32; 4]; 5],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ColorMatrixUniform {
    rows: [[f32; 4]; 4],
    offsets: [f32; 4],
    padding: [[f32; 4]; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct BevelUniform {
    highlight_color: [f32; 4],
    shadow_color: [f32; 4],
    strength: f32,
    bevel_type: u32,
    knockout: u32,
    composite_source: u32,
    blur_offset: [f32; 2],
    offset_padding: [f32; 2],
    padding: [[f32; 4]; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ConvolutionUniform {
    source_size: [f32; 2],
    divisor: f32,
    bias: f32,
    rows: u32,
    cols: u32,
    preserve_alpha: u32,
    clamp_edges: u32,
    default_color: [f32; 4],
    padding: [[f32; 4]; 5],
}

#[derive(Clone, Hash, PartialEq, Eq)]
struct ConvolutionKernelKey {
    rows: u8,
    cols: u8,
    weights: Vec<u32>,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GradientFilterUniform {
    strength: f32,
    filter_type: u32,
    knockout: u32,
    composite_source: u32,
    kind: [u32; 4],
    blur_offset: [f32; 2],
    offset_padding: [f32; 2],
    padding: [[f32; 4]; 5],
}

#[derive(Clone, Hash, PartialEq, Eq)]
struct GradientRampKey(Vec<[u8; 5]>);

#[derive(Resource, Default, Debug)]
pub struct FlashRenderDiagnostics {
    /// Detailed collection is enabled by VabDiagnosticsPlugin.
    pub(crate) enabled: bool,
    pub(crate) gpu_timing: bool,
    pub rendered_views: u64,
    pub draw_calls: u64,
    pub pending_views: usize,
    pub errors: Vec<String>,
    pub texture_allocations: u64,
    pub texture_reuses: u64,
    pub texture_first_in_bucket_allocations: u64,
    pub texture_exhausted_bucket_allocations: u64,
    pub texture_allocated_bytes: u64,
    pub live_transient_textures: usize,
    pub peak_transient_textures: usize,
    pub live_transient_bytes: u64,
    pub peak_transient_bytes: u64,
    /// All textures still owned by the shared pool, both borrowed and idle.
    pub pooled_textures: usize,
    /// Descriptor-derived texel/block payload. Backend allocation padding and
    /// driver metadata are not exposed by wgpu and are therefore excluded.
    pub pooled_texture_bytes: u64,
    pub pooled_idle_textures: usize,
    pub pooled_idle_bytes: u64,
    pub texture_pool_buckets: usize,
    pub largest_texture_pool_bucket: usize,
    pub largest_texture_pool_bucket_bytes: u64,
    pub vab_instances_extracted: usize,
    pub vab_instances_prepared: usize,
    pub vab_instances_queued: usize,
    pub unsupported_vab_layers: usize,
    pub vab_draw_instances_prepared: usize,
    pub vab_draw_packets: usize,
    pub vab_filter_layers: usize,
    pub vab_mask_layers: usize,
    pub vab_filter_output_pixels: u64,
    /// Filter shader passes actually submitted during the latest scene frame.
    /// Cached layers contribute zero passes.
    pub vab_filter_passes: u64,
    /// Sum of render-target pixel areas over those filter shader passes.
    /// This counts processed pixels, not unique output pixels or GPU fragments.
    pub vab_filter_processed_pixels: u64,
    pub vab_draw_calls: AtomicU64,
    pub vab_sample_cache_hits: usize,
    pub vab_sample_cache_misses: usize,
    pub vab_filter_cache_hits: usize,
    pub vab_filter_cache_misses: usize,
    pub vab_filter_cache_entries: usize,
    pub vab_filter_cache_bytes: u64,
    pub material_bind_group_creations: u64,
    pub material_bind_group_reuses: u64,
    pub instance_buffer_allocations: u64,
    pub instance_buffer_reuses: u64,
    pub instance_bind_group_creations: u64,
    pub instance_bind_group_reuses: u64,
    pub filter_uniform_buffer_allocations: u64,
    pub filter_uniform_buffer_reuses: u64,
    /// CPU time spent extracting and sampling visible VAB entities in the latest frame.
    pub vab_extract_cpu_ns: u64,
    /// CPU time spent building per-view scaled operation data in the latest frame.
    pub vab_queue_cpu_ns: u64,
    /// CPU time spent packing and uploading instance data in the latest frame.
    pub vab_prepare_buffers_cpu_ns: u64,
    /// CPU time spent preparing draw packets and bind groups in the latest frame.
    pub vab_prepare_bind_groups_cpu_ns: u64,
}

#[derive(Resource)]
pub struct FlashGpu {
    uniform_layout: BindGroupLayout,
    texture_layout: BindGroupLayout,
    pipeline_layout: PipelineLayout,
    glow_pipeline_layout: PipelineLayout,
    convolution_pipeline_layout: PipelineLayout,
    convolution_kernel_layout: BindGroupLayout,
    gradient_filter_pipeline_layout: PipelineLayout,
    shape_shader: ShaderModule,
    composite_shader: ShaderModule,
    blur_shader: ShaderModule,
    glow_shader: ShaderModule,
    bevel_shader: ShaderModule,
    color_matrix_shader: ShaderModule,
    convolution_shader: ShaderModule,
    gradient_filter_shader: ShaderModule,
    alpha_mask_shader: ShaderModule,
    shapes: HashMap<(MeshVertexBufferLayoutRef, u32), RenderPipeline>,
    composites: HashMap<(TextureFormat, u32, bool), RenderPipeline>,
    layer_composites: HashMap<(TextureFormat, u32, VabBlendMode), RenderPipeline>,
    blurs: HashMap<TextureFormat, RenderPipeline>,
    glows: HashMap<TextureFormat, RenderPipeline>,
    bevels: HashMap<TextureFormat, RenderPipeline>,
    color_matrices: HashMap<TextureFormat, RenderPipeline>,
    convolutions: HashMap<TextureFormat, RenderPipeline>,
    convolution_kernels: HashMap<ConvolutionKernelKey, BindGroup>,
    gradient_filters: HashMap<TextureFormat, RenderPipeline>,
    gradient_ramps: HashMap<GradientRampKey, BindGroup>,
    alpha_masks: HashMap<(TextureFormat, u32), RenderPipeline>,
    white: BindGroup,
    sampler: Sampler,
}

impl FromWorld for FlashGpu {
    fn from_world(world: &mut World) -> Self {
        let device = world.resource::<RenderDevice>();
        let uniform_layout = device.create_bind_group_layout(
            "flash_uniform_layout",
            &[BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::VERTEX_FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: NonZeroU64::new(128),
                },
                count: None,
            }],
        );
        let texture_layout = device.create_bind_group_layout(
            "flash_texture_layout",
            &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: true },
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Sampler(SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        );
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("flash_layout"),
            bind_group_layouts: &[Some(&uniform_layout), Some(&texture_layout)],
            immediate_size: 0,
        });
        let glow_pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("flash_glow_layout"),
            bind_group_layouts: &[
                Some(&uniform_layout),
                Some(&texture_layout),
                Some(&texture_layout),
            ],
            immediate_size: 0,
        });
        let convolution_kernel_layout = device.create_bind_group_layout(
            "flash_convolution_kernel_layout",
            &[BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        );
        let convolution_pipeline_layout =
            device.create_pipeline_layout(&PipelineLayoutDescriptor {
                label: Some("flash_convolution_layout"),
                bind_group_layouts: &[
                    Some(&uniform_layout),
                    Some(&texture_layout),
                    Some(&convolution_kernel_layout),
                ],
                immediate_size: 0,
            });
        let gradient_filter_pipeline_layout =
            device.create_pipeline_layout(&PipelineLayoutDescriptor {
                label: Some("flash_gradient_filter_layout"),
                bind_group_layouts: &[
                    Some(&uniform_layout),
                    Some(&texture_layout),
                    Some(&texture_layout),
                    Some(&texture_layout),
                ],
                immediate_size: 0,
            });
        let shape_shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("flash_shape"),
            source: ShaderSource::Wgsl(Cow::Borrowed(include_str!(concat!(
                env!("OUT_DIR"),
                "/shape.wgsl"
            )))),
        });
        let composite_shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("flash_composite"),
            source: ShaderSource::Wgsl(Cow::Borrowed(include_str!(concat!(
                env!("OUT_DIR"),
                "/composite.wgsl"
            )))),
        });
        let blur_shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("flash_blur"),
            source: ShaderSource::Wgsl(Cow::Borrowed(include_str!(concat!(
                env!("OUT_DIR"),
                "/blur.wgsl"
            )))),
        });
        let glow_shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("flash_glow"),
            source: ShaderSource::Wgsl(Cow::Borrowed(include_str!(concat!(
                env!("OUT_DIR"),
                "/glow.wgsl"
            )))),
        });
        let bevel_shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("flash_bevel"),
            source: ShaderSource::Wgsl(Cow::Borrowed(include_str!(concat!(
                env!("OUT_DIR"),
                "/bevel.wgsl"
            )))),
        });
        let color_matrix_shader =
            device.create_and_validate_shader_module(ShaderModuleDescriptor {
                label: Some("flash_color_matrix"),
                source: ShaderSource::Wgsl(Cow::Borrowed(include_str!(concat!(
                    env!("OUT_DIR"),
                    "/color_matrix.wgsl"
                )))),
            });
        let convolution_shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("flash_convolution"),
            source: ShaderSource::Wgsl(Cow::Borrowed(include_str!(concat!(
                env!("OUT_DIR"),
                "/convolution.wgsl"
            )))),
        });
        let gradient_filter_shader =
            device.create_and_validate_shader_module(ShaderModuleDescriptor {
                label: Some("flash_gradient_filter"),
                source: ShaderSource::Wgsl(Cow::Borrowed(include_str!(concat!(
                    env!("OUT_DIR"),
                    "/gradient_filter.wgsl"
                )))),
            });
        let alpha_mask_shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("flash_alpha_mask"),
            source: ShaderSource::Wgsl(Cow::Borrowed(include_str!(concat!(
                env!("OUT_DIR"),
                "/alpha_mask.wgsl"
            )))),
        });
        let sampler = device.create_sampler(&SamplerDescriptor {
            label: Some("flash_linear_clamp"),
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            ..Default::default()
        });
        let white_texture = device.create_texture_with_data(
            world.resource::<bevy::render::renderer::RenderQueue>(),
            &TextureDescriptor {
                label: Some("flash_white"),
                size: Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: TextureFormat::Rgba8Unorm,
                usage: TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            TextureDataOrder::LayerMajor,
            &[255; 4],
        );
        let white = texture_group(
            device,
            &texture_layout,
            &white_texture.create_view(&Default::default()),
            &sampler,
        );
        Self {
            uniform_layout,
            texture_layout,
            pipeline_layout,
            glow_pipeline_layout,
            convolution_pipeline_layout,
            convolution_kernel_layout,
            gradient_filter_pipeline_layout,
            shape_shader,
            composite_shader,
            blur_shader,
            glow_shader,
            bevel_shader,
            color_matrix_shader,
            convolution_shader,
            gradient_filter_shader,
            alpha_mask_shader,
            shapes: HashMap::default(),
            composites: HashMap::default(),
            layer_composites: HashMap::default(),
            blurs: HashMap::default(),
            glows: HashMap::default(),
            bevels: HashMap::default(),
            color_matrices: HashMap::default(),
            convolutions: HashMap::default(),
            convolution_kernels: HashMap::default(),
            gradient_filters: HashMap::default(),
            gradient_ramps: HashMap::default(),
            alpha_masks: HashMap::default(),
            white,
            sampler,
        }
    }
}

fn texture_group(
    device: &RenderDevice,
    layout: &BindGroupLayout,
    view: &TextureView,
    sampler: &Sampler,
) -> BindGroup {
    device.create_bind_group(
        "flash_texture",
        layout,
        &[
            BindGroupEntry {
                binding: 0,
                resource: BindingResource::TextureView(view),
            },
            BindGroupEntry {
                binding: 1,
                resource: BindingResource::Sampler(sampler),
            },
        ],
    )
}

fn gradient_ramp_bytes(colors: &[AnimGradientRecord]) -> Vec<u8> {
    let mut ramp = vec![0; 256 * 4];
    let Some(first) = colors.first() else {
        return ramp;
    };
    for ratio in 0..=255u16 {
        let next_index = colors
            .iter()
            .position(|record| u16::from(record.ratio) >= ratio)
            .unwrap_or(colors.len() - 1);
        let next = &colors[next_index];
        let previous = if next_index == 0 {
            first
        } else {
            &colors[next_index - 1]
        };
        let span = f32::from(next.ratio.saturating_sub(previous.ratio));
        let factor = if span == 0.0 {
            0.0
        } else {
            (ratio as f32 - f32::from(previous.ratio)) / span
        }
        .clamp(0.0, 1.0);
        let offset = usize::from(ratio) * 4;
        for (channel, (a, b)) in [
            (previous.color_r, next.color_r),
            (previous.color_g, next.color_g),
            (previous.color_b, next.color_b),
            (previous.color_a, next.color_a),
        ]
        .into_iter()
        .enumerate()
        {
            ramp[offset + channel] =
                (f32::from(a) + (f32::from(b) - f32::from(a)) * factor).clamp(0.0, 255.0) as u8;
        }
    }
    ramp
}

impl FlashGpu {
    fn shape_pipeline(
        &mut self,
        device: &RenderDevice,
        mesh: &RenderMesh,
        samples: u32,
    ) -> Result<RenderPipeline> {
        let key = (mesh.layout.clone(), samples);
        if let Some(pipeline) = self.shapes.get(&key) {
            return Ok(pipeline.clone());
        }
        let layout = mesh.layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_COLOR.at_shader_location(1),
        ])?;
        let buffers = [Some(RawVertexBufferLayout {
            array_stride: layout.array_stride,
            step_mode: layout.step_mode,
            attributes: &layout.attributes,
        })];
        let pipeline = self.pipeline(
            device,
            &self.shape_shader,
            &buffers,
            TextureFormat::Rgba8Unorm,
            samples,
            Some(BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        );
        self.shapes.insert(key, pipeline.clone());
        Ok(pipeline)
    }
    fn composite_pipeline(
        &mut self,
        device: &RenderDevice,
        format: TextureFormat,
        samples: u32,
        blend: bool,
    ) -> RenderPipeline {
        let key = (format, samples, blend);
        if let Some(pipeline) = self.composites.get(&key) {
            return pipeline.clone();
        }
        let pipeline = self.pipeline(
            device,
            &self.composite_shader,
            &[],
            format,
            samples,
            blend.then_some(BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        );
        self.composites.insert(key, pipeline.clone());
        pipeline
    }
    fn layer_composite_pipeline(
        &mut self,
        device: &RenderDevice,
        format: TextureFormat,
        samples: u32,
        blend: VabBlendMode,
    ) -> RenderPipeline {
        let key = (format, samples, blend);
        if let Some(pipeline) = self.layer_composites.get(&key) {
            return pipeline.clone();
        }
        let pipeline = self.pipeline(
            device,
            &self.composite_shader,
            &[],
            format,
            samples,
            Some(fixed_blend_state(blend)),
        );
        self.layer_composites.insert(key, pipeline.clone());
        pipeline
    }
    fn blur_pipeline(&mut self, device: &RenderDevice, format: TextureFormat) -> RenderPipeline {
        if let Some(pipeline) = self.blurs.get(&format) {
            return pipeline.clone();
        }
        let pipeline = self.pipeline(device, &self.blur_shader, &[], format, 1, None);
        self.blurs.insert(format, pipeline.clone());
        pipeline
    }
    fn glow_pipeline(&mut self, device: &RenderDevice, format: TextureFormat) -> RenderPipeline {
        if let Some(pipeline) = self.glows.get(&format) {
            return pipeline.clone();
        }
        let pipeline = device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: Some("flash_glow_pipeline"),
            layout: Some(&self.glow_pipeline_layout),
            vertex: RawVertexState {
                module: &self.glow_shader,
                entry_point: Some("vertex"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(RawFragmentState {
                module: &self.glow_shader,
                entry_point: Some("fragment"),
                targets: &[Some(ColorTargetState {
                    format,
                    blend: None,
                    write_mask: ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        self.glows.insert(format, pipeline.clone());
        pipeline
    }
    fn bevel_pipeline(&mut self, device: &RenderDevice, format: TextureFormat) -> RenderPipeline {
        if let Some(pipeline) = self.bevels.get(&format) {
            return pipeline.clone();
        }
        let pipeline = device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: Some("flash_bevel_pipeline"),
            layout: Some(&self.glow_pipeline_layout),
            vertex: RawVertexState {
                module: &self.bevel_shader,
                entry_point: Some("vertex"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(RawFragmentState {
                module: &self.bevel_shader,
                entry_point: Some("fragment"),
                targets: &[Some(ColorTargetState {
                    format,
                    blend: None,
                    write_mask: ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        self.bevels.insert(format, pipeline.clone());
        pipeline
    }
    fn color_matrix_pipeline(
        &mut self,
        device: &RenderDevice,
        format: TextureFormat,
    ) -> RenderPipeline {
        if let Some(pipeline) = self.color_matrices.get(&format) {
            return pipeline.clone();
        }
        let pipeline = self.pipeline(device, &self.color_matrix_shader, &[], format, 1, None);
        self.color_matrices.insert(format, pipeline.clone());
        pipeline
    }
    fn convolution_pipeline(
        &mut self,
        device: &RenderDevice,
        format: TextureFormat,
    ) -> RenderPipeline {
        if let Some(pipeline) = self.convolutions.get(&format) {
            return pipeline.clone();
        }
        let pipeline = device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: Some("flash_convolution_pipeline"),
            layout: Some(&self.convolution_pipeline_layout),
            vertex: RawVertexState {
                module: &self.convolution_shader,
                entry_point: Some("vertex"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(RawFragmentState {
                module: &self.convolution_shader,
                entry_point: Some("fragment"),
                targets: &[Some(ColorTargetState {
                    format,
                    blend: None,
                    write_mask: ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        self.convolutions.insert(format, pipeline.clone());
        pipeline
    }

    fn convolution_kernel_group(
        &mut self,
        device: &RenderDevice,
        filter: &AnimConvolutionFilter,
    ) -> BindGroup {
        let key = ConvolutionKernelKey {
            rows: filter.num_matrix_rows,
            cols: filter.num_matrix_cols,
            weights: filter.matrix.iter().map(|value| value.to_bits()).collect(),
        };
        if let Some(group) = self.convolution_kernels.get(&key) {
            return group.clone();
        }
        let buffer = device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("flash_convolution_kernel"),
            contents: bytemuck::cast_slice(&filter.matrix),
            usage: BufferUsages::STORAGE,
        });
        let group = device.create_bind_group(
            "flash_convolution_kernel",
            &self.convolution_kernel_layout,
            &[BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        );
        self.convolution_kernels.insert(key, group.clone());
        group
    }
    fn gradient_filter_pipeline(
        &mut self,
        device: &RenderDevice,
        format: TextureFormat,
    ) -> RenderPipeline {
        if let Some(pipeline) = self.gradient_filters.get(&format) {
            return pipeline.clone();
        }
        let pipeline = device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: Some("flash_gradient_filter_pipeline"),
            layout: Some(&self.gradient_filter_pipeline_layout),
            vertex: RawVertexState {
                module: &self.gradient_filter_shader,
                entry_point: Some("vertex"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(RawFragmentState {
                module: &self.gradient_filter_shader,
                entry_point: Some("fragment"),
                targets: &[Some(ColorTargetState {
                    format,
                    blend: None,
                    write_mask: ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        self.gradient_filters.insert(format, pipeline.clone());
        pipeline
    }

    fn gradient_ramp_group(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        colors: &[AnimGradientRecord],
    ) -> Result<BindGroup> {
        ensure!(
            colors.windows(2).all(|pair| pair[0].ratio <= pair[1].ratio),
            "gradient filter ratios must be non-decreasing"
        );
        let key = GradientRampKey(
            colors
                .iter()
                .map(|record| {
                    [
                        record.ratio,
                        record.color_r,
                        record.color_g,
                        record.color_b,
                        record.color_a,
                    ]
                })
                .collect(),
        );
        if let Some(group) = self.gradient_ramps.get(&key) {
            return Ok(group.clone());
        }
        let bytes = gradient_ramp_bytes(colors);
        let texture = device.create_texture_with_data(
            queue,
            &TextureDescriptor {
                label: Some("flash_gradient_filter_ramp"),
                size: Extent3d {
                    width: 256,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                // Flash/Ruffle filter math operates on encoded sRGB values in
                // an Unorm working surface. Sampling this ramp must therefore
                // not perform automatic sRGB decoding.
                format: TextureFormat::Rgba8Unorm,
                usage: TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            TextureDataOrder::LayerMajor,
            &bytes,
        );
        let group = texture_group(
            device,
            &self.texture_layout,
            &texture.create_view(&Default::default()),
            &self.sampler,
        );
        self.gradient_ramps.insert(key, group.clone());
        Ok(group)
    }
    fn alpha_mask_pipeline(
        &mut self,
        device: &RenderDevice,
        format: TextureFormat,
        samples: u32,
    ) -> RenderPipeline {
        let key = (format, samples);
        if let Some(pipeline) = self.alpha_masks.get(&key) {
            return pipeline.clone();
        }
        let pipeline = device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: Some("flash_alpha_mask_pipeline"),
            layout: Some(&self.glow_pipeline_layout),
            vertex: RawVertexState {
                module: &self.alpha_mask_shader,
                entry_point: Some("vertex"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(RawFragmentState {
                module: &self.alpha_mask_shader,
                entry_point: Some("fragment"),
                targets: &[Some(ColorTargetState {
                    format,
                    blend: Some(BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: MultisampleState {
                count: samples,
                ..Default::default()
            },
            multiview_mask: None,
            cache: None,
        });
        self.alpha_masks.insert(key, pipeline.clone());
        pipeline
    }
    fn pipeline(
        &self,
        device: &RenderDevice,
        shader: &ShaderModule,
        buffers: &[Option<RawVertexBufferLayout>],
        format: TextureFormat,
        samples: u32,
        blend: Option<BlendState>,
    ) -> RenderPipeline {
        device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: Some("flash_pipeline"),
            layout: Some(&self.pipeline_layout),
            vertex: RawVertexState {
                module: shader,
                entry_point: Some("vertex"),
                buffers,
                compilation_options: Default::default(),
            },
            fragment: Some(RawFragmentState {
                module: shader,
                entry_point: Some("fragment"),
                targets: &[Some(ColorTargetState {
                    format,
                    blend,
                    write_mask: ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: MultisampleState {
                count: samples,
                ..Default::default()
            },
            multiview_mask: None,
            cache: None,
        })
    }
}

pub(super) fn fixed_blend_state(mode: VabBlendMode) -> BlendState {
    match mode {
        VabBlendMode::Add => BlendState {
            color: BlendComponent {
                src_factor: BlendFactor::One,
                dst_factor: BlendFactor::One,
                operation: BlendOperation::Add,
            },
            alpha: BlendComponent::OVER,
        },
        VabBlendMode::Subtract => BlendState {
            color: BlendComponent {
                src_factor: BlendFactor::One,
                dst_factor: BlendFactor::One,
                operation: BlendOperation::ReverseSubtract,
            },
            alpha: BlendComponent::OVER,
        },
        VabBlendMode::Screen => BlendState {
            color: BlendComponent {
                src_factor: BlendFactor::One,
                dst_factor: BlendFactor::OneMinusSrc,
                operation: BlendOperation::Add,
            },
            alpha: BlendComponent::OVER,
        },
        // Temporary approximation used by the original bevy_flash renderer.
        // This compares premultiplied attachment values, so translucent edges
        // differ from Flash/Ruffle's straight-color Lighten equation. Keep the
        // mode isolated so a destination-texture pass can replace it later.
        VabBlendMode::Lighten => BlendState {
            color: BlendComponent {
                src_factor: BlendFactor::One,
                dst_factor: BlendFactor::One,
                operation: BlendOperation::Max,
            },
            alpha: BlendComponent::OVER,
        },
        _ => BlendState::PREMULTIPLIED_ALPHA_BLENDING,
    }
}

struct PreparedDraw {
    mesh: AssetId<Mesh>,
    pipeline: RenderPipeline,
    texture: BindGroup,
    offset: u32,
}

enum PreparedFilter {
    Blur(Vec<u32>),
    Glow {
        blur_offsets: Vec<u32>,
        glow_offset: u32,
    },
    Bevel {
        blur_offsets: Vec<u32>,
        bevel_offset: u32,
    },
    ColorMatrix(u32),
    Convolution {
        offset: u32,
        kernel: BindGroup,
    },
    Gradient {
        blur_offsets: Vec<u32>,
        effect_offset: u32,
        ramp: BindGroup,
    },
}

enum PreparedOp {
    Draw(PreparedDraw),
    Mask {
        mask: Vec<PreparedOp>,
        content: Vec<PreparedOp>,
        bounds: [f32; 4],
        offset: u32,
    },
    Layer {
        ops: Vec<PreparedOp>,
        bounds: [f32; 4],
        offset: u32,
        filters: Vec<PreparedFilter>,
        composite: RenderPipeline,
    },
}
pub(super) struct PreparedLayer {
    ops: Vec<PreparedOp>,
    uniforms: BindGroup,
    pub(super) bounds: [f32; 4],
}

/// Count the shader passes that `render_ops` will execute for this prepared
/// layer. Intermediate clears, shape draws, masks and layer composites are
/// deliberately excluded: this metric isolates actual filter work.
pub(super) fn filter_workload(layer: &PreparedLayer) -> (u64, u64) {
    fn visit(ops: &[PreparedOp]) -> (u64, u64) {
        let mut passes = 0u64;
        let mut pixels = 0u64;
        for op in ops {
            match op {
                PreparedOp::Draw(_) => {}
                PreparedOp::Mask { mask, content, .. } => {
                    for children in [mask, content] {
                        let (child_passes, child_pixels) = visit(children);
                        passes = passes.saturating_add(child_passes);
                        pixels = pixels.saturating_add(child_pixels);
                    }
                }
                PreparedOp::Layer {
                    ops: children,
                    bounds,
                    filters,
                    ..
                } => {
                    let (child_passes, child_pixels) = visit(children);
                    passes = passes.saturating_add(child_passes);
                    pixels = pixels.saturating_add(child_pixels);
                    let own_passes = filters.iter().fold(0u64, |count, filter| {
                        let filter_passes = match filter {
                            PreparedFilter::Blur(offsets) => offsets.len() as u64,
                            PreparedFilter::Glow { blur_offsets, .. }
                            | PreparedFilter::Bevel { blur_offsets, .. }
                            | PreparedFilter::Gradient { blur_offsets, .. } => {
                                blur_offsets.len() as u64 + 1
                            }
                            PreparedFilter::ColorMatrix(_) | PreparedFilter::Convolution { .. } => {
                                1
                            }
                        };
                        count.saturating_add(filter_passes)
                    });
                    passes = passes.saturating_add(own_passes);
                    let area = (bounds[2] as u64).saturating_mul(bounds[3] as u64);
                    pixels = pixels.saturating_add(area.saturating_mul(own_passes));
                }
            }
        }
        (passes, pixels)
    }
    visit(&layer.ops)
}

#[derive(Component)]
pub struct PreparedFrame {
    layer: PreparedLayer,
    output_offset: u32,
    output: RenderPipeline,
}

struct Uniforms {
    bytes: Vec<u8>,
    alignment: usize,
}

#[derive(Default)]
pub(super) struct ReusableUniformBuffer {
    buffer: Option<Buffer>,
    bind_group: Option<BindGroup>,
    capacity: u64,
}

impl ReusableUniformBuffer {
    fn upload(
        &mut self,
        bytes: &[u8],
        label: &'static str,
        device: &RenderDevice,
        queue: &RenderQueue,
        layout: &BindGroupLayout,
    ) -> (BindGroup, bool) {
        let required = u64::try_from(bytes.len()).expect("uniform buffer exceeds u64");
        let mut allocated = false;
        if required > self.capacity {
            self.capacity = required.max(128).next_power_of_two();
            let buffer = device.create_buffer(&BufferDescriptor {
                label: Some(label),
                size: self.capacity,
                usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.bind_group = Some(device.create_bind_group(
                label,
                layout,
                &[BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::Buffer(BufferBinding {
                        buffer: &buffer,
                        offset: 0,
                        size: NonZeroU64::new(128),
                    }),
                }],
            ));
            self.buffer = Some(buffer);
            allocated = true;
        }
        let buffer = self
            .buffer
            .as_ref()
            .expect("non-empty uniforms allocate a buffer");
        queue.write_buffer(buffer, 0, bytes);
        (
            self.bind_group
                .as_ref()
                .expect("uniform buffer has a bind group")
                .clone(),
            allocated,
        )
    }
}
impl Uniforms {
    fn push(&mut self, value: &impl Pod) -> Result<u32> {
        let offset = u32::try_from(self.bytes.len()).context("too many draw uniforms")?;
        self.bytes.extend_from_slice(bytemuck::bytes_of(value));
        self.bytes.resize(
            self.bytes.len().div_ceil(self.alignment) * self.alignment,
            0,
        );
        Ok(offset)
    }
}

pub(super) fn pixel_bounds(bounds: [f32; 4]) -> Result<[f32; 4]> {
    ensure!(
        bounds.iter().all(|x| x.is_finite()),
        "non-finite render bounds"
    );
    let x = bounds[0].floor();
    let y = bounds[1].floor();
    let width = ((bounds[0] + bounds[2]).ceil() - x) as u32;
    let height = ((bounds[1] + bounds[3]).ceil() - y) as u32;
    Ok([
        x,
        y,
        classify_texture_dimension(width) as f32,
        classify_texture_dimension(height) as f32,
    ])
}

/// Nearby animation frames commonly differ by only a few pixels. Classing the
/// complete logical render bounds keeps shader UVs and blur texel sizes matched
/// to the allocated texture while allowing those frames to share pool buckets.
fn classify_texture_dimension(value: u32) -> u32 {
    let quantum = match value {
        0..=256 => 16,
        257..=1024 => 32,
        _ => 64,
    };
    value.div_ceil(quantum) * quantum
}

fn prepare_blur(
    filter: &AnimBlurFilter,
    size: [f32; 2],
    uniforms: &mut Uniforms,
) -> Result<Vec<u32>> {
    let mut result = Vec::new();
    for _ in 0..filter.num_passes {
        for (blur, direction) in [
            (filter.blur_x, [1.0 / size[0], 0.0]),
            (filter.blur_y, [0.0, 1.0 / size[1]]),
        ] {
            let full_size = (blur as f32 / 65536.0).clamp(1.0, 255.0);
            if full_size <= 1.0 {
                continue;
            }
            let radius = (full_size - 1.0) / 2.0;
            let m = radius.ceil() - 1.0;
            let alpha = ((radius - m) * 255.0).floor() / 255.0;
            let (last_offset, last_weight) = if alpha > 0.0 {
                (1.0 / (1.0 / alpha + 1.0), alpha + 1.0)
            } else {
                (0.0, 1.0)
            };
            result.push(uniforms.push(&BlurUniform {
                direction,
                full_size,
                m,
                m2: m * 2.0,
                first_weight: alpha,
                last_offset,
                last_weight,
                padding: [[0.0; 4]; 6],
            })?);
        }
    }
    Ok(result)
}

fn prepare_glow(
    filter: &AnimGlowFilter,
    size: [f32; 2],
    uniforms: &mut Uniforms,
) -> Result<PreparedFilter> {
    let blur_offsets = prepare_blur(
        &AnimBlurFilter {
            blur_x: filter.blur_x,
            blur_y: filter.blur_y,
            num_passes: filter.num_passes,
        },
        size,
        uniforms,
    )?;
    let glow_offset = uniforms.push(&glow_uniform(filter))?;
    Ok(PreparedFilter::Glow {
        blur_offsets,
        glow_offset,
    })
}

fn prepare_drop_shadow(
    filter: &AnimDropShadowFilter,
    size: [f32; 2],
    uniforms: &mut Uniforms,
) -> Result<PreparedFilter> {
    let blur_offsets = prepare_blur(
        &AnimBlurFilter {
            blur_x: filter.blur_x,
            blur_y: filter.blur_y,
            num_passes: filter.num_passes,
        },
        size,
        uniforms,
    )?;
    let distance = filter.distance as f32 / 65536.0;
    let angle = filter.angle as f32 / 65536.0;
    let glow_offset = uniforms.push(&GlowUniform {
        color: [
            filter.color_r as f32 / 255.0,
            filter.color_g as f32 / 255.0,
            filter.color_b as f32 / 255.0,
            filter.color_a as f32 / 255.0,
        ],
        strength: filter.strength as f32 / 256.0,
        inner: u32::from(filter.flags & 0x80 != 0),
        knockout: u32::from(filter.flags & 0x40 != 0),
        composite_source: u32::from(filter.flags & 0x20 != 0),
        blur_offset: [
            -(angle.cos() * distance) / size[0],
            -(angle.sin() * distance) / size[1],
        ],
        offset_padding: [0.0; 2],
        padding: [[0.0; 4]; 5],
    })?;
    Ok(PreparedFilter::Glow {
        blur_offsets,
        glow_offset,
    })
}

fn prepare_bevel(
    filter: &AnimBevelFilter,
    size: [f32; 2],
    uniforms: &mut Uniforms,
) -> Result<PreparedFilter> {
    let blur_offsets = prepare_blur(
        &AnimBlurFilter {
            blur_x: filter.blur_x,
            blur_y: filter.blur_y,
            num_passes: filter.num_passes,
        },
        size,
        uniforms,
    )?;
    let distance = filter.distance as f32 / 65536.0;
    let angle = filter.angle as f32 / 65536.0;
    let bevel_offset = uniforms.push(&BevelUniform {
        highlight_color: [
            filter.highlight_color_r as f32 / 255.0,
            filter.highlight_color_g as f32 / 255.0,
            filter.highlight_color_b as f32 / 255.0,
            filter.highlight_color_a as f32 / 255.0,
        ],
        shadow_color: [
            filter.shadow_color_r as f32 / 255.0,
            filter.shadow_color_g as f32 / 255.0,
            filter.shadow_color_b as f32 / 255.0,
            filter.shadow_color_a as f32 / 255.0,
        ],
        strength: filter.strength as f32 / 256.0,
        bevel_type: if filter.flags & 0x10 != 0 {
            2
        } else if filter.flags & 0x80 != 0 {
            1
        } else {
            0
        },
        knockout: u32::from(filter.flags & 0x40 != 0),
        // Ruffle always composites the source for Bevel; SWF has no separate
        // hide-object behavior for this filter.
        composite_source: 1,
        blur_offset: [
            (angle.cos() * distance) / size[0],
            (angle.sin() * distance) / size[1],
        ],
        offset_padding: [0.0; 2],
        padding: [[0.0; 4]; 4],
    })?;
    Ok(PreparedFilter::Bevel {
        blur_offsets,
        bevel_offset,
    })
}

#[allow(clippy::too_many_arguments)]
fn prepare_gradient_filter(
    filter: &AnimGradientFilter,
    size: [f32; 2],
    bevel: bool,
    gpu: &mut FlashGpu,
    device: &RenderDevice,
    queue: &RenderQueue,
    uniforms: &mut Uniforms,
) -> Result<PreparedFilter> {
    let blur_offsets = prepare_blur(
        &AnimBlurFilter {
            blur_x: filter.blur_x,
            blur_y: filter.blur_y,
            num_passes: filter.num_passes,
        },
        size,
        uniforms,
    )?;
    let distance = filter.distance as f32 / 65536.0;
    let angle = filter.angle as f32 / 65536.0;
    let direction = if bevel { 1.0 } else { -1.0 };
    let effect_offset = uniforms.push(&GradientFilterUniform {
        strength: filter.strength as f32 / 256.0,
        filter_type: if filter.flags & 0x10 != 0 {
            2
        } else if filter.flags & 0x80 != 0 {
            1
        } else {
            0
        },
        knockout: u32::from(filter.flags & 0x40 != 0),
        composite_source: u32::from(filter.flags & 0x20 != 0),
        kind: [u32::from(bevel), 0, 0, 0],
        blur_offset: [
            direction * angle.cos() * distance / size[0],
            direction * angle.sin() * distance / size[1],
        ],
        offset_padding: [0.0; 2],
        padding: [[0.0; 4]; 5],
    })?;
    Ok(PreparedFilter::Gradient {
        blur_offsets,
        effect_offset,
        ramp: gpu.gradient_ramp_group(device, queue, &filter.colors)?,
    })
}

fn glow_uniform(filter: &AnimGlowFilter) -> GlowUniform {
    GlowUniform {
        color: [
            filter.color_r as f32 / 255.0,
            filter.color_g as f32 / 255.0,
            filter.color_b as f32 / 255.0,
            filter.color_a as f32 / 255.0,
        ],
        strength: filter.strength as f32 / 256.0,
        inner: u32::from(filter.flags & 0x80 != 0),
        knockout: u32::from(filter.flags & 0x40 != 0),
        composite_source: u32::from(filter.flags & 0x20 != 0),
        blur_offset: [0.0; 2],
        offset_padding: [0.0; 2],
        padding: [[0.0; 4]; 5],
    }
}

fn color_matrix_uniform(filter: &AnimColorMatrixFilter) -> ColorMatrixUniform {
    let matrix = &filter.matrix;
    ColorMatrixUniform {
        rows: [
            [matrix[0], matrix[1], matrix[2], matrix[3]],
            [matrix[5], matrix[6], matrix[7], matrix[8]],
            [matrix[10], matrix[11], matrix[12], matrix[13]],
            [matrix[15], matrix[16], matrix[17], matrix[18]],
        ],
        offsets: [
            matrix[4] / 255.0,
            matrix[9] / 255.0,
            matrix[14] / 255.0,
            matrix[19] / 255.0,
        ],
        padding: [[0.0; 4]; 3],
    }
}

fn prepare_convolution(
    filter: &AnimConvolutionFilter,
    size: [f32; 2],
    gpu: &mut FlashGpu,
    device: &RenderDevice,
    uniforms: &mut Uniforms,
) -> Result<PreparedFilter> {
    let rows = usize::from(filter.num_matrix_rows);
    let cols = usize::from(filter.num_matrix_cols);
    ensure!(
        rows > 0 && cols > 0,
        "convolution matrix dimensions must be non-zero"
    );
    ensure!(
        rows.checked_mul(cols) == Some(filter.matrix.len()),
        "convolution matrix length {} does not match {}x{} dimensions",
        filter.matrix.len(),
        cols,
        rows
    );
    ensure!(
        filter.matrix.iter().all(|value| value.is_finite())
            && filter.divisor.is_finite()
            && filter.bias.is_finite(),
        "convolution parameters must be finite"
    );
    let bytes = filter.matrix.len().saturating_mul(size_of::<f32>());
    ensure!(
        bytes <= device.limits().max_storage_buffer_binding_size as usize,
        "convolution matrix requires {bytes} bytes, exceeding the device storage binding limit"
    );
    let offset = uniforms.push(&ConvolutionUniform {
        source_size: size,
        // Flash ignores zero and uses the property default of one.
        divisor: if filter.divisor == 0.0 {
            1.0
        } else {
            filter.divisor
        },
        bias: filter.bias / 255.0,
        rows: u32::from(filter.num_matrix_rows),
        cols: u32::from(filter.num_matrix_cols),
        preserve_alpha: u32::from(filter.flags & 0x01 != 0),
        clamp_edges: u32::from(filter.flags & 0x02 != 0),
        default_color: [
            filter.default_color_r as f32 / 255.0,
            filter.default_color_g as f32 / 255.0,
            filter.default_color_b as f32 / 255.0,
            filter.default_color_a as f32 / 255.0,
        ],
        padding: [[0.0; 4]; 5],
    })?;
    Ok(PreparedFilter::Convolution {
        offset,
        kernel: gpu.convolution_kernel_group(device, filter),
    })
}

#[allow(clippy::too_many_arguments)]
fn prepare_ops(
    ops: &[Op],
    bounds: [f32; 4],
    samples: u32,
    gpu: &mut FlashGpu,
    device: &RenderDevice,
    queue: &RenderQueue,
    meshes: &RenderAssets<RenderMesh>,
    images: &RenderAssets<GpuImage>,
    uniforms: &mut Uniforms,
) -> Result<Vec<PreparedOp>> {
    let mut result = Vec::new();
    for op in ops {
        result.push(match op {
            Op::Draw(draw) => {
                let mesh = meshes.get(draw.mesh).context("pending GPU mesh")?;
                let texture = match draw.texture {
                    Some(id) => {
                        let image = images.get(id).context("pending GPU image")?;
                        texture_group(
                            device,
                            &gpu.texture_layout,
                            &image.texture_view,
                            &image.sampler,
                        )
                    }
                    None => gpu.white.clone(),
                };
                let m = draw.transform.matrix;
                let c = draw.transform.color_transform;
                let uv = draw.uv;
                let offset = uniforms.push(&DrawUniform {
                    row0: [m.a, m.c, m.tx, 0.0],
                    row1: [m.b, m.d, m.ty, 0.0],
                    uv0: [uv.x_axis.x, uv.y_axis.x, uv.z_axis.x, 0.0],
                    uv1: [uv.x_axis.y, uv.y_axis.y, uv.z_axis.y, 0.0],
                    multiply: [c.r_multiply, c.g_multiply, c.b_multiply, c.a_multiply],
                    add: [c.r_add, c.g_add, c.b_add, c.a_add],
                    material: [
                        draw.gradient[0],
                        draw.gradient[1],
                        draw.gradient[2],
                        draw.kind as f32,
                    ],
                    viewport: bounds,
                })?;
                PreparedOp::Draw(PreparedDraw {
                    mesh: draw.mesh,
                    pipeline: gpu.shape_pipeline(device, mesh, samples)?,
                    texture,
                    offset,
                })
            }
            Op::Mask {
                mask,
                content,
                bounds: child,
            } => {
                let child = pixel_bounds(*child)?;
                ensure!(
                    child[2] > 0.0
                        && child[3] > 0.0
                        && child[2] <= device.limits().max_texture_dimension_2d as f32
                        && child[3] <= device.limits().max_texture_dimension_2d as f32,
                    "invalid mask dimensions"
                );
                let mask = prepare_ops(
                    mask, child, samples, gpu, device, queue, meshes, images, uniforms,
                )?;
                let content = prepare_ops(
                    content, child, samples, gpu, device, queue, meshes, images, uniforms,
                )?;
                let offset = uniforms.push(&CompositeUniform {
                    rect: child,
                    viewport: bounds,
                    mode: [0.0; 4],
                    color: [0.0; 4],
                })?;
                PreparedOp::Mask {
                    mask,
                    content,
                    bounds: child,
                    offset,
                }
            }
            Op::Layer {
                ops,
                bounds: child,
                filters,
                blend,
            } => {
                let child = pixel_bounds(*child)?;
                ensure!(
                    child[2] > 0.0
                        && child[3] > 0.0
                        && child[2] <= device.limits().max_texture_dimension_2d as f32
                        && child[3] <= device.limits().max_texture_dimension_2d as f32,
                    "invalid layer dimensions"
                );
                let ops = prepare_ops(
                    ops, child, samples, gpu, device, queue, meshes, images, uniforms,
                )?;
                let mut prepared_filters = Vec::new();
                for filter in filters {
                    match filter {
                        AnimFilter::BlurFilter(filter) => {
                            prepared_filters.push(PreparedFilter::Blur(prepare_blur(
                                filter,
                                [child[2], child[3]],
                                uniforms,
                            )?))
                        }
                        AnimFilter::GlowFilter(filter) => prepared_filters.push(prepare_glow(
                            filter,
                            [child[2], child[3]],
                            uniforms,
                        )?),
                        AnimFilter::ColorMatrixFilter(filter) => {
                            prepared_filters.push(PreparedFilter::ColorMatrix(
                                uniforms.push(&color_matrix_uniform(filter))?,
                            ))
                        }
                        AnimFilter::DropShadowFilter(filter) => prepared_filters
                            .push(prepare_drop_shadow(filter, [child[2], child[3]], uniforms)?),
                        AnimFilter::BevelFilter(filter) => prepared_filters.push(prepare_bevel(
                            filter,
                            [child[2], child[3]],
                            uniforms,
                        )?),
                        AnimFilter::ConvolutionFilter(filter) => {
                            prepared_filters.push(prepare_convolution(
                                filter,
                                [child[2], child[3]],
                                gpu,
                                device,
                                uniforms,
                            )?)
                        }
                        AnimFilter::GradientGlowFilter(filter) => {
                            prepared_filters.push(prepare_gradient_filter(
                                filter,
                                [child[2], child[3]],
                                false,
                                gpu,
                                device,
                                queue,
                                uniforms,
                            )?)
                        }
                        AnimFilter::GradientBevelFilter(filter) => {
                            prepared_filters.push(prepare_gradient_filter(
                                filter,
                                [child[2], child[3]],
                                true,
                                gpu,
                                device,
                                queue,
                                uniforms,
                            )?)
                        }
                    }
                }
                let offset = uniforms.push(&CompositeUniform {
                    rect: child,
                    viewport: bounds,
                    mode: [0.0; 4],
                    color: [0.0; 4],
                })?;
                PreparedOp::Layer {
                    ops,
                    bounds: child,
                    offset,
                    filters: prepared_filters,
                    composite: gpu.layer_composite_pipeline(
                        device,
                        TextureFormat::Rgba8Unorm,
                        samples,
                        *blend,
                    ),
                }
            }
        });
    }
    Ok(result)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn prepare_layer(
    ops: &[Op],
    bounds: [f32; 4],
    samples: u32,
    gpu: &mut FlashGpu,
    device: &RenderDevice,
    queue: &RenderQueue,
    meshes: &RenderAssets<RenderMesh>,
    images: &RenderAssets<GpuImage>,
    uniform_buffer: &mut ReusableUniformBuffer,
) -> Result<(PreparedLayer, bool)> {
    let mut uniforms = Uniforms {
        bytes: Vec::new(),
        alignment: (device.limits().min_uniform_buffer_offset_alignment as usize).max(128),
    };
    let ops = prepare_ops(
        ops,
        bounds,
        samples,
        gpu,
        device,
        queue,
        meshes,
        images,
        &mut uniforms,
    )?;
    let (uniforms, allocated) = uniform_buffer.upload(
        &uniforms.bytes,
        "flash_layer_uniforms",
        device,
        queue,
        &gpu.uniform_layout,
    );
    Ok((
        PreparedLayer {
            ops,
            uniforms,
            bounds,
        },
        allocated,
    ))
}

#[derive(Resource, Default)]
pub(super) struct OffscreenUniformBuffers(HashMap<Entity, ReusableUniformBuffer>);

#[allow(clippy::too_many_arguments)]
pub fn prepare_frames(
    mut commands: Commands,
    query: Query<(Entity, &ExtractedFrame, &OffscreenViewTarget, &Msaa)>,
    mut gpu: ResMut<FlashGpu>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    meshes: Res<RenderAssets<RenderMesh>>,
    images: Res<RenderAssets<GpuImage>>,
    mut uniform_buffers: ResMut<OffscreenUniformBuffers>,
    mut diagnostics: ResMut<FlashRenderDiagnostics>,
) {
    diagnostics.pending_views = 0;
    diagnostics.errors.clear();
    let mut live = HashSet::<Entity>::default();
    for (entity, frame, target, msaa) in &query {
        live.insert(entity);
        commands.entity(entity).remove::<PreparedFrame>();
        let result = (|| -> Result<PreparedFrame> {
            if let Some(error) = &frame.error {
                anyhow::bail!("{error}");
            }
            let output = images
                .get(target.target.handle.id())
                .context("pending output Image")?;
            ensure!(
                output.texture_descriptor.format == TextureFormat::Rgba8UnormSrgb,
                "output Image must use Rgba8UnormSrgb"
            );
            ensure!(
                output.texture_descriptor.size.width == target.size.x
                    && output.texture_descriptor.size.height == target.size.y
                    && target.size.x > 0
                    && target.size.y > 0,
                "output Image size mismatch"
            );
            ensure!(
                output
                    .texture
                    .usage()
                    .contains(TextureUsages::RENDER_ATTACHMENT),
                "output Image needs RENDER_ATTACHMENT usage"
            );
            let bounds = [
                target.origin.x,
                target.origin.y,
                target.size.x as f32,
                target.size.y as f32,
            ];
            let mut uniforms = Uniforms {
                bytes: Vec::new(),
                alignment: (device.limits().min_uniform_buffer_offset_alignment as usize).max(128),
            };
            let ops = prepare_ops(
                &frame.ops,
                bounds,
                msaa.samples(),
                &mut gpu,
                &device,
                &queue,
                &meshes,
                &images,
                &mut uniforms,
            )?;
            let output_offset = uniforms.push(&CompositeUniform {
                rect: bounds,
                viewport: bounds,
                mode: [1.0, 0.0, 0.0, 0.0],
                color: [0.0; 4],
            })?;
            let (uniforms, allocated) = uniform_buffers.0.entry(entity).or_default().upload(
                &uniforms.bytes,
                "flash_frame_uniforms",
                &device,
                &queue,
                &gpu.uniform_layout,
            );
            if allocated {
                diagnostics.filter_uniform_buffer_allocations += 1;
            } else {
                diagnostics.filter_uniform_buffer_reuses += 1;
            }
            Ok(PreparedFrame {
                layer: PreparedLayer {
                    ops,
                    uniforms,
                    bounds,
                },
                output_offset,
                output: gpu.composite_pipeline(&device, output.texture_descriptor.format, 1, false),
            })
        })();
        match result {
            Ok(prepared) => {
                commands.entity(entity).insert(prepared);
            }
            Err(error) => {
                let error = error.to_string();
                if error.starts_with("pending") {
                    diagnostics.pending_views += 1;
                } else {
                    diagnostics.errors.push(error);
                }
            }
        }
    }
    uniform_buffers.0.retain(|entity, _| live.contains(entity));
}

fn descriptor(width: u32, height: u32) -> TextureDescriptor<'static> {
    TextureDescriptor {
        label: None,
        size: Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::Rgba8Unorm,
        usage: TextureUsages::RENDER_ATTACHMENT
            | TextureUsages::TEXTURE_BINDING
            | TextureUsages::COPY_SRC,
        view_formats: &[],
    }
}

#[allow(clippy::too_many_arguments)]
fn render_single_texture_filter_pass(
    source: &TextureView,
    destination: &TextureView,
    offset: u32,
    label: &'static str,
    prepared: &PreparedLayer,
    gpu: &FlashGpu,
    pipeline: &RenderPipeline,
    device: &RenderDevice,
    context: &mut RenderContext,
) {
    let texture = texture_group(device, &gpu.texture_layout, source, &gpu.sampler);
    let attachment = [Some(RenderPassColorAttachment {
        view: destination,
        resolve_target: None,
        depth_slice: None,
        ops: Operations {
            load: LoadOp::Clear(Default::default()),
            store: StoreOp::Store,
        },
    })];
    let mut pass = context.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some(label),
        color_attachments: &attachment,
        ..Default::default()
    });
    pass.set_render_pipeline(pipeline);
    pass.set_bind_group(0, &prepared.uniforms, &[offset]);
    pass.set_bind_group(1, &texture, &[]);
    pass.draw(0..6, 0..1);
}

#[allow(clippy::too_many_arguments)]
fn render_convolution_pass(
    source: &TextureView,
    destination: &TextureView,
    offset: u32,
    kernel: &BindGroup,
    prepared: &PreparedLayer,
    gpu: &FlashGpu,
    pipeline: &RenderPipeline,
    device: &RenderDevice,
    context: &mut RenderContext,
) {
    let texture = texture_group(device, &gpu.texture_layout, source, &gpu.sampler);
    let attachment = [Some(RenderPassColorAttachment {
        view: destination,
        resolve_target: None,
        depth_slice: None,
        ops: Operations {
            load: LoadOp::Clear(Default::default()),
            store: StoreOp::Store,
        },
    })];
    let mut pass = context.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some("flash_convolution"),
        color_attachments: &attachment,
        ..Default::default()
    });
    pass.set_render_pipeline(pipeline);
    pass.set_bind_group(0, &prepared.uniforms, &[offset]);
    pass.set_bind_group(1, &texture, &[]);
    pass.set_bind_group(2, kernel, &[]);
    pass.draw(0..6, 0..1);
}

#[allow(clippy::too_many_arguments)]
fn render_blur_and_two_texture_filter(
    child: &mut super::texture_cache::TextureTarget,
    bounds: [f32; 4],
    blur_offsets: &[u32],
    effect_offset: u32,
    blur_label: &'static str,
    effect_label: &'static str,
    effect_pipeline: &RenderPipeline,
    extra_group: Option<&BindGroup>,
    prepared: &PreparedLayer,
    gpu: &mut FlashGpu,
    device: &RenderDevice,
    pool: &mut FrameInternalTextureCache,
    context: &mut RenderContext,
    draw_calls: &mut u64,
) {
    // The original and blurred images must remain live together. One scratch
    // lease is enough for any number of ping-pong blur passes and is promoted
    // directly when it receives the final effect, avoiding a copy pass.
    let mut scratch = pool.acquire(
        device,
        descriptor(bounds[2] as u32, bounds[3] as u32),
        TextureAllocationSite {
            purpose: TexturePurpose::FilterScratch,
            bounds,
        },
    );
    let blur_pipeline = gpu.blur_pipeline(device, TextureFormat::Rgba8Unorm);
    let mut blurred_in_other = false;
    for (pass_index, blur_offset) in blur_offsets.iter().enumerate() {
        let (source, destination) = if pass_index == 0 {
            (child.main_view(), child.other_view())
        } else if blurred_in_other {
            (child.other_view(), &scratch.default_view)
        } else {
            (&scratch.default_view, child.other_view())
        };
        render_single_texture_filter_pass(
            source,
            destination,
            *blur_offset,
            blur_label,
            prepared,
            gpu,
            &blur_pipeline,
            device,
            context,
        );
        blurred_in_other = pass_index == 0 || !blurred_in_other;
        *draw_calls += 1;
    }
    let original = child.main_view();
    let blurred = if blur_offsets.is_empty() {
        original
    } else if blurred_in_other {
        child.other_view()
    } else {
        &scratch.default_view
    };
    let result_in_other = !blurred_in_other || blur_offsets.is_empty();
    let destination = if result_in_other {
        child.other_view()
    } else {
        &scratch.default_view
    };
    let source_group = texture_group(device, &gpu.texture_layout, original, &gpu.sampler);
    let blurred_group = texture_group(device, &gpu.texture_layout, blurred, &gpu.sampler);
    let attachment = [Some(RenderPassColorAttachment {
        view: destination,
        resolve_target: None,
        depth_slice: None,
        ops: Operations {
            load: LoadOp::Clear(Default::default()),
            store: StoreOp::Store,
        },
    })];
    let mut pass = context.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some(effect_label),
        color_attachments: &attachment,
        ..Default::default()
    });
    pass.set_render_pipeline(effect_pipeline);
    pass.set_bind_group(0, &prepared.uniforms, &[effect_offset]);
    pass.set_bind_group(1, &source_group, &[]);
    pass.set_bind_group(2, &blurred_group, &[]);
    if let Some(extra_group) = extra_group {
        pass.set_bind_group(3, extra_group, &[]);
    }
    pass.draw(0..6, 0..1);
    drop(pass);
    if result_in_other {
        child.flip_main();
    } else {
        child.replace_main_with(&mut scratch);
    }
    *draw_calls += 1;
}

#[allow(clippy::too_many_arguments)]
fn render_ops(
    ops: &[PreparedOp],
    target: &super::texture_cache::TextureTarget,
    prepared: &PreparedLayer,
    gpu: &mut FlashGpu,
    device: &RenderDevice,
    pool: &mut FrameInternalTextureCache,
    msaa: &Msaa,
    meshes: &RenderAssets<RenderMesh>,
    allocator: &MeshAllocator,
    context: &mut RenderContext,
    draw_calls: &mut u64,
) {
    let mut index = 0;
    while index < ops.len() {
        if let PreparedOp::Mask {
            mask,
            content,
            bounds,
            offset,
        } = &ops[index]
        {
            let mut mask_target = pool.get(
                device,
                descriptor(bounds[2] as u32, bounds[3] as u32),
                msaa,
                TextureAllocationSite {
                    purpose: TexturePurpose::Mask,
                    bounds: *bounds,
                },
            );
            clear_target(&mask_target, Srgba::NONE, context);
            render_ops(
                mask,
                &mask_target,
                prepared,
                gpu,
                device,
                pool,
                msaa,
                meshes,
                allocator,
                context,
                draw_calls,
            );
            mask_target.release_msaa_attachment();
            let mut content_target = pool.get(
                device,
                descriptor(bounds[2] as u32, bounds[3] as u32),
                msaa,
                TextureAllocationSite {
                    purpose: TexturePurpose::MaskContent,
                    bounds: *bounds,
                },
            );
            clear_target(&content_target, Srgba::NONE, context);
            render_ops(
                content,
                &content_target,
                prepared,
                gpu,
                device,
                pool,
                msaa,
                meshes,
                allocator,
                context,
                draw_calls,
            );
            content_target.release_msaa_attachment();
            let content_group = texture_group(
                device,
                &gpu.texture_layout,
                content_target.main_view(),
                &gpu.sampler,
            );
            let mask_group = texture_group(
                device,
                &gpu.texture_layout,
                mask_target.main_view(),
                &gpu.sampler,
            );
            let pipeline =
                gpu.alpha_mask_pipeline(device, TextureFormat::Rgba8Unorm, msaa.samples());
            let attachment = [Some(target.load_color_attachment())];
            let mut pass = context.begin_tracked_render_pass(RenderPassDescriptor {
                label: Some("flash_alpha_mask"),
                color_attachments: &attachment,
                ..Default::default()
            });
            pass.set_render_pipeline(&pipeline);
            pass.set_bind_group(0, &prepared.uniforms, &[*offset]);
            pass.set_bind_group(1, &content_group, &[]);
            pass.set_bind_group(2, &mask_group, &[]);
            pass.draw(0..6, 0..1);
            *draw_calls += 1;
            index += 1;
        } else if let PreparedOp::Layer {
            ops: children,
            bounds,
            offset,
            filters,
            composite,
        } = &ops[index]
        {
            let mut child = pool.get(
                device,
                descriptor(bounds[2] as u32, bounds[3] as u32),
                msaa,
                TextureAllocationSite {
                    purpose: TexturePurpose::Layer,
                    bounds: *bounds,
                },
            );
            clear_target(&child, Srgba::NONE, context);
            render_ops(
                children, &child, prepared, gpu, device, pool, msaa, meshes, allocator, context,
                draw_calls,
            );
            child.release_msaa_attachment();
            if !filters.is_empty() {
                child.enable_ping_pong(pool.acquire(
                    device,
                    descriptor(bounds[2] as u32, bounds[3] as u32),
                    TextureAllocationSite {
                        purpose: TexturePurpose::PingPong,
                        bounds: *bounds,
                    },
                ));
            }
            for filter in filters {
                match filter {
                    PreparedFilter::Blur(offsets) => {
                        let pipeline = gpu.blur_pipeline(device, TextureFormat::Rgba8Unorm);
                        for blur_offset in offsets {
                            let write = child.post_process_write();
                            render_single_texture_filter_pass(
                                write.source,
                                write.destination,
                                *blur_offset,
                                "flash_blur",
                                prepared,
                                gpu,
                                &pipeline,
                                device,
                                context,
                            );
                            *draw_calls += 1;
                        }
                    }
                    PreparedFilter::Glow {
                        blur_offsets,
                        glow_offset,
                    } => {
                        let glow_pipeline = gpu.glow_pipeline(device, TextureFormat::Rgba8Unorm);
                        render_blur_and_two_texture_filter(
                            &mut child,
                            *bounds,
                            blur_offsets,
                            *glow_offset,
                            "flash_glow_blur",
                            "flash_glow",
                            &glow_pipeline,
                            None,
                            prepared,
                            gpu,
                            device,
                            pool,
                            context,
                            draw_calls,
                        );
                    }
                    PreparedFilter::Bevel {
                        blur_offsets,
                        bevel_offset,
                    } => {
                        let bevel_pipeline = gpu.bevel_pipeline(device, TextureFormat::Rgba8Unorm);
                        render_blur_and_two_texture_filter(
                            &mut child,
                            *bounds,
                            blur_offsets,
                            *bevel_offset,
                            "flash_bevel_blur",
                            "flash_bevel",
                            &bevel_pipeline,
                            None,
                            prepared,
                            gpu,
                            device,
                            pool,
                            context,
                            draw_calls,
                        );
                    }
                    PreparedFilter::ColorMatrix(offset) => {
                        let pipeline = gpu.color_matrix_pipeline(device, TextureFormat::Rgba8Unorm);
                        let write = child.post_process_write();
                        render_single_texture_filter_pass(
                            write.source,
                            write.destination,
                            *offset,
                            "flash_color_matrix",
                            prepared,
                            gpu,
                            &pipeline,
                            device,
                            context,
                        );
                        *draw_calls += 1;
                    }
                    PreparedFilter::Convolution { offset, kernel } => {
                        let pipeline = gpu.convolution_pipeline(device, TextureFormat::Rgba8Unorm);
                        let write = child.post_process_write();
                        render_convolution_pass(
                            write.source,
                            write.destination,
                            *offset,
                            kernel,
                            prepared,
                            gpu,
                            &pipeline,
                            device,
                            context,
                        );
                        *draw_calls += 1;
                    }
                    PreparedFilter::Gradient {
                        blur_offsets,
                        effect_offset,
                        ramp,
                    } => {
                        let pipeline =
                            gpu.gradient_filter_pipeline(device, TextureFormat::Rgba8Unorm);
                        render_blur_and_two_texture_filter(
                            &mut child,
                            *bounds,
                            blur_offsets,
                            *effect_offset,
                            "flash_gradient_filter_blur",
                            "flash_gradient_filter",
                            &pipeline,
                            Some(ramp),
                            prepared,
                            gpu,
                            device,
                            pool,
                            context,
                            draw_calls,
                        );
                    }
                }
            }
            let texture =
                texture_group(device, &gpu.texture_layout, child.main_view(), &gpu.sampler);
            let attachment = [Some(target.load_color_attachment())];
            let mut pass = context.begin_tracked_render_pass(RenderPassDescriptor {
                label: Some("flash_layer_composite"),
                color_attachments: &attachment,
                ..Default::default()
            });
            pass.set_render_pipeline(composite);
            pass.set_bind_group(0, &prepared.uniforms, &[*offset]);
            pass.set_bind_group(1, &texture, &[]);
            pass.draw(0..6, 0..1);
            *draw_calls += 1;
            // Pass ends before child lease is returned; later siblings can reuse it.
            index += 1;
        } else {
            let attachment = [Some(target.load_color_attachment())];
            let mut pass = context.begin_tracked_render_pass(RenderPassDescriptor {
                label: Some("flash_shapes"),
                color_attachments: &attachment,
                ..Default::default()
            });
            while let Some(PreparedOp::Draw(draw)) = ops.get(index) {
                let Some(mesh) = meshes.get(draw.mesh) else {
                    index += 1;
                    continue;
                };
                let Some(vertices) = allocator.mesh_vertex_slice(&draw.mesh) else {
                    index += 1;
                    continue;
                };
                pass.set_render_pipeline(&draw.pipeline);
                pass.set_bind_group(0, &prepared.uniforms, &[draw.offset]);
                pass.set_bind_group(1, &draw.texture, &[]);
                pass.set_vertex_buffer(0, vertices.buffer.slice(..));
                match &mesh.buffer_info {
                    RenderMeshBufferInfo::Indexed {
                        count,
                        index_format,
                    } => {
                        if let Some(indices) = allocator.mesh_index_slice(&draw.mesh) {
                            pass.set_index_buffer(indices.buffer.slice(..), *index_format);
                            pass.draw_indexed(
                                indices.range.start..indices.range.start + count,
                                vertices.range.start as i32,
                                0..1,
                            );
                        }
                    }
                    RenderMeshBufferInfo::NonIndexed => pass.draw(vertices.range, 0..1),
                }
                *draw_calls += 1;
                index += 1;
            }
        }
    }
}

fn clear_target(
    target: &super::texture_cache::TextureTarget,
    color: Srgba,
    context: &mut RenderContext,
) {
    let attachment = [Some(target.get_color_attachment(color))];
    context.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some("flash_clear"),
        color_attachments: &attachment,
        ..Default::default()
    });
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_prepared_layer(
    prepared: &PreparedLayer,
    clear: Srgba,
    gpu: &mut FlashGpu,
    device: &RenderDevice,
    pool: &mut FrameInternalTextureCache,
    msaa: &Msaa,
    meshes: &RenderAssets<RenderMesh>,
    allocator: &MeshAllocator,
    context: &mut RenderContext,
    draw_calls: &mut u64,
) -> super::texture_cache::TextureTarget {
    let mut surface = pool.get(
        device,
        descriptor(prepared.bounds[2] as u32, prepared.bounds[3] as u32),
        msaa,
        TextureAllocationSite {
            purpose: TexturePurpose::Root,
            bounds: prepared.bounds,
        },
    );
    clear_target(&surface, clear, context);
    render_ops(
        &prepared.ops,
        &surface,
        prepared,
        gpu,
        device,
        pool,
        msaa,
        meshes,
        allocator,
        context,
        draw_calls,
    );
    surface.release_msaa_attachment();
    surface
}

#[allow(clippy::too_many_arguments)]
pub fn render_frame(
    view: ViewQuery<(
        &OffscreenViewTarget,
        &Msaa,
        Option<&PreparedFrame>,
        Option<&super::RasterOnce>,
    )>,
    mut gpu: ResMut<FlashGpu>,
    device: Res<RenderDevice>,
    mut pool: ResMut<FrameInternalTextureCache>,
    meshes: Res<RenderAssets<RenderMesh>>,
    images: Res<RenderAssets<GpuImage>>,
    allocator: Res<MeshAllocator>,
    mut context: RenderContext,
    mut diagnostics: ResMut<FlashRenderDiagnostics>,
) {
    let (target, msaa, prepared, once) = view.into_inner();
    if once.is_some_and(|once| once.complete()) {
        return;
    }
    let Some(prepared) = prepared else {
        return;
    };
    let Some(output) = images.get(target.target.handle.id()) else {
        return;
    };
    fn ready(
        ops: &[PreparedOp],
        meshes: &RenderAssets<RenderMesh>,
        allocator: &MeshAllocator,
    ) -> bool {
        ops.iter().all(|op| match op {
            PreparedOp::Draw(draw) => meshes.get(draw.mesh).is_some_and(|mesh| {
                allocator.mesh_vertex_slice(&draw.mesh).is_some()
                    && (!matches!(mesh.buffer_info, RenderMeshBufferInfo::Indexed { .. })
                        || allocator.mesh_index_slice(&draw.mesh).is_some())
            }),
            PreparedOp::Layer { ops, .. } => ready(ops, meshes, allocator),
            PreparedOp::Mask { mask, content, .. } => {
                ready(mask, meshes, allocator) && ready(content, meshes, allocator)
            }
        })
    }
    if once.is_some() && !ready(&prepared.layer.ops, &meshes, &allocator) {
        return;
    }
    let surface = render_prepared_layer(
        &prepared.layer,
        target.clear_color,
        &mut gpu,
        &device,
        &mut pool,
        msaa,
        &meshes,
        &allocator,
        &mut context,
        &mut diagnostics.draw_calls,
    );
    let texture = texture_group(
        &device,
        &gpu.texture_layout,
        surface.main_view(),
        &gpu.sampler,
    );
    let attachments = [Some(RenderPassColorAttachment {
        view: &output.texture_view,
        resolve_target: None,
        depth_slice: None,
        ops: Operations {
            load: LoadOp::Clear(Default::default()),
            store: StoreOp::Store,
        },
    })];
    {
        let mut pass = context.begin_tracked_render_pass(RenderPassDescriptor {
            label: Some("flash_image_output"),
            color_attachments: &attachments,
            ..Default::default()
        });
        pass.set_render_pipeline(&prepared.output);
        pass.set_bind_group(0, &prepared.layer.uniforms, &[prepared.output_offset]);
        pass.set_bind_group(1, &texture, &[]);
        pass.draw(0..6, 0..1);
    }
    if let Some(once) = once {
        once.0.store(true, std::sync::atomic::Ordering::Release);
    }
    diagnostics.rendered_views += 1;
    diagnostics.texture_allocations = pool.allocations;
    diagnostics.texture_reuses = pool.reuses;
    diagnostics.texture_first_in_bucket_allocations = pool.first_in_bucket_allocations;
    diagnostics.texture_exhausted_bucket_allocations = pool.exhausted_bucket_allocations;
    diagnostics.texture_allocated_bytes = pool.allocated_bytes;
    diagnostics.live_transient_textures = pool.live_textures;
    diagnostics.peak_transient_textures = pool.peak_live_textures;
    diagnostics.live_transient_bytes = pool.live_bytes;
    diagnostics.peak_transient_bytes = pool.peak_live_bytes;
    diagnostics.pooled_textures = pool.pooled_textures;
    diagnostics.pooled_texture_bytes = pool.pooled_texture_bytes;
    diagnostics.pooled_idle_textures = pool.pooled_idle_textures;
    diagnostics.pooled_idle_bytes = pool.pooled_idle_bytes;
    diagnostics.texture_pool_buckets = pool.bucket_count;
    diagnostics.largest_texture_pool_bucket = pool.largest_bucket_textures;
    diagnostics.largest_texture_pool_bucket_bytes = pool.largest_bucket_bytes;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glow_uniform_matches_swf_flags_and_fixed_strength() {
        let filter = AnimGlowFilter {
            flags: 0xe7,
            color_r: 255,
            color_g: 128,
            color_b: 0,
            color_a: 64,
            blur_x: 0,
            blur_y: 0,
            strength: 384,
            num_passes: 7,
        };

        let uniform = glow_uniform(&filter);
        assert_eq!(uniform.color, [1.0, 128.0 / 255.0, 0.0, 64.0 / 255.0]);
        assert_eq!(uniform.strength, 1.5);
        assert_eq!(uniform.inner, 1);
        assert_eq!(uniform.knockout, 1);
        assert_eq!(uniform.composite_source, 1);
    }

    #[test]
    fn filter_texture_dimensions_use_bounded_size_classes() {
        assert_eq!(classify_texture_dimension(1), 16);
        assert_eq!(classify_texture_dimension(256), 256);
        assert_eq!(classify_texture_dimension(257), 288);
        assert_eq!(classify_texture_dimension(1024), 1024);
        assert_eq!(classify_texture_dimension(1025), 1088);
    }

    #[test]
    fn gradient_ramp_interpolates_rgba_records() {
        let ramp = gradient_ramp_bytes(&[
            AnimGradientRecord {
                ratio: 0,
                color_r: 255,
                color_g: 0,
                color_b: 0,
                color_a: 0,
            },
            AnimGradientRecord {
                ratio: 255,
                color_r: 0,
                color_g: 0,
                color_b: 255,
                color_a: 255,
            },
        ]);

        assert_eq!(&ramp[0..4], &[255, 0, 0, 0]);
        assert_eq!(&ramp[128 * 4..128 * 4 + 4], &[127, 0, 128, 128]);
        assert_eq!(&ramp[255 * 4..256 * 4], &[0, 0, 255, 255]);
    }
}
