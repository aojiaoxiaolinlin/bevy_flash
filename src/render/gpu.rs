use super::{
    OffscreenViewTarget,
    extract::{ExtractedFrame, Op},
    texture_cache::{FrameInternalTextureCache, TextureAllocationSite, TexturePurpose},
};
use anyhow::{Context as ContextExt, Result, ensure};
use bevy::{
    asset::{AssetId, embedded_asset, load_embedded_asset},
    mesh::{MeshVertexBufferLayoutRef, VertexBufferLayout},
    platform::collections::{HashMap, HashSet},
    prelude::*,
    render::{
        extract_resource::ExtractResource,
        mesh::{RenderMesh, RenderMeshBufferInfo, allocator::MeshAllocator},
        render_asset::RenderAssets,
        render_resource::*,
        renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery},
        texture::GpuImage,
        view::Msaa,
    },
    shader::{ShaderCacheError, ShaderDefVal},
};
use bytemuck::{Pod, Zeroable};
use std::{num::NonZeroU64, sync::atomic::AtomicU64};
use vatf::animation::{
    AnimBevelFilter, AnimBlurFilter, AnimColorMatrixFilter, AnimConvolutionFilter,
    AnimDropShadowFilter, AnimFilter, AnimGlowFilter, AnimGradientFilter, AnimGradientRecord,
};

use crate::vab_asset::{ATTRIBUTE_VAB_COLOR_SRGB, ATTRIBUTE_VAB_POSITION, VabBlendMode};

/// Controls optional downsampling of blur passes to reduce GPU memory and fill-rate.
///
/// For pure `Blur` filters, the blurred layer itself can be produced at reduced resolution
/// and bilinearly upsampled during compositing. For `Glow`, `DropShadow`, `Bevel`,
/// `GradientGlow`, and `GradientBevel`, the foreground source stays at full resolution while
/// only the intermediate blur passes run at reduced resolution.
#[derive(Resource, ExtractResource, Clone, Copy, Debug, PartialEq)]
#[extract_app(bevy::render::RenderApp)]
pub struct VabFilterQuality {
    /// Resolution scale in `(0.0, 1.0]` applied to eligible blur passes.
    /// `1.0` keeps all blur passes at full resolution (default).
    pub blur_scale: f32,
    /// Minimum filter target dimension (in pixels) before downsampling applies.
    pub min_dimension: u32,
    /// Minimum scaled blur diameter (in pixels) on an axis before downsampling that axis.
    pub min_blur_radius: f32,
    /// When enabled, uniformly scaled isotropic filter symbols (e.g. expanding/shrinking
    /// glowing particles) reuse octave-quantized cached stamp textures and scale the final
    /// quad on screen, trading fixed-pixel halo width for dramatic particle cache reuse.
    /// Translation and isotropic rotation stamps are always lossless and enabled by default.
    pub reuse_scaled_stamps: bool,
}

impl Default for VabFilterQuality {
    fn default() -> Self {
        Self::full()
    }
}

impl VabFilterQuality {
    /// Full-resolution blur passes and lossless translation/rotation filter stamps (exact quality).
    pub const fn full() -> Self {
        Self {
            blur_scale: 1.0,
            min_dimension: 32,
            min_blur_radius: 3.0,
            reuse_scaled_stamps: false,
        }
    }

    /// Half-resolution blur passes (`0.5x` width/height = 1/4 blur pixels, ~1/8 blur work)
    /// while preserving full-resolution foreground geometry on Glow/DropShadow/Bevel filters,
    /// plus octave-quantized scaled particle stamp reuse.
    pub const fn half_blur() -> Self {
        Self {
            blur_scale: 0.5,
            min_dimension: 32,
            min_blur_radius: 3.0,
            reuse_scaled_stamps: true,
        }
    }

    /// Quarter-resolution blur passes and scaled particle stamps for heavy effects on low-end GPUs.
    pub const fn quarter_blur() -> Self {
        Self {
            blur_scale: 0.25,
            min_dimension: 64,
            min_blur_radius: 6.0,
            reuse_scaled_stamps: true,
        }
    }

    /// Enable or disable octave-quantized scaled stamp reuse for isotropic particle symbols.
    pub const fn with_scaled_stamps(mut self, enabled: bool) -> Self {
        self.reuse_scaled_stamps = enabled;
        self
    }

    pub(super) fn cache_key(self) -> [u32; 4] {
        let scale = if self.blur_scale.is_finite() {
            self.blur_scale.clamp(0.125, 1.0)
        } else {
            1.0
        };
        let min_blur = if self.min_blur_radius.is_finite() {
            self.min_blur_radius.max(0.0)
        } else {
            0.0
        };
        [
            scale.to_bits(),
            self.min_dimension,
            min_blur.to_bits(),
            u32::from(self.reuse_scaled_stamps),
        ]
    }
}

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

pub(super) fn register_shaders(app: &mut App) {
    embedded_asset!(app, "shaders/shape.wesl");
    embedded_asset!(app, "shaders/composite.wesl");
    embedded_asset!(app, "shaders/blur.wesl");
    embedded_asset!(app, "shaders/glow.wesl");
    embedded_asset!(app, "shaders/bevel.wesl");
    embedded_asset!(app, "shaders/color_matrix.wesl");
    embedded_asset!(app, "shaders/convolution.wesl");
    embedded_asset!(app, "shaders/gradient_filter.wesl");
    embedded_asset!(app, "shaders/alpha_mask.wesl");
    embedded_asset!(app, "shaders/complex_blend.wesl");
    embedded_asset!(app, "shaders/fullscreen.wesl");
}

const MAX_CACHED_FILTER_RESOURCES: usize = 256;

struct CachedImageBindGroup {
    texture_view: TextureViewId,
    sampler: SamplerId,
    bind_group: BindGroup,
}

#[derive(Resource)]
pub struct FlashGpu {
    uniform_layout: BindGroupLayout,
    texture_layout: BindGroupLayout,
    pipeline_layout: Vec<BindGroupLayoutDescriptor>,
    glow_pipeline_layout: Vec<BindGroupLayoutDescriptor>,
    convolution_pipeline_layout: Vec<BindGroupLayoutDescriptor>,
    convolution_kernel_layout: BindGroupLayout,
    gradient_filter_pipeline_layout: Vec<BindGroupLayoutDescriptor>,
    _fullscreen_shader: Handle<Shader>,
    shape_shader: Handle<Shader>,
    composite_shader: Handle<Shader>,
    blur_shader: Handle<Shader>,
    glow_shader: Handle<Shader>,
    bevel_shader: Handle<Shader>,
    color_matrix_shader: Handle<Shader>,
    convolution_shader: Handle<Shader>,
    gradient_filter_shader: Handle<Shader>,
    alpha_mask_shader: Handle<Shader>,
    complex_blend_shader: Handle<Shader>,
    shapes: HashMap<(MeshVertexBufferLayoutRef, u32, VabBlendMode), CachedRenderPipelineId>,
    composites: HashMap<(TextureFormat, u32, bool), CachedRenderPipelineId>,
    layer_composites: HashMap<(TextureFormat, u32, VabBlendMode), CachedRenderPipelineId>,
    blurs: HashMap<TextureFormat, CachedRenderPipelineId>,
    glows: HashMap<TextureFormat, CachedRenderPipelineId>,
    bevels: HashMap<TextureFormat, CachedRenderPipelineId>,
    color_matrices: HashMap<TextureFormat, CachedRenderPipelineId>,
    convolutions: HashMap<TextureFormat, CachedRenderPipelineId>,
    convolution_kernels: HashMap<ConvolutionKernelKey, (BindGroup, u64)>,
    gradient_filters: HashMap<TextureFormat, CachedRenderPipelineId>,
    gradient_ramps: HashMap<GradientRampKey, (BindGroup, u64)>,
    image_bind_groups: HashMap<AssetId<Image>, CachedImageBindGroup>,
    filter_cache_tick: u64,
    alpha_masks: HashMap<(TextureFormat, u32), CachedRenderPipelineId>,
    complex_blends: HashMap<(TextureFormat, u32), CachedRenderPipelineId>,
    white: BindGroup,
    sampler: Sampler,
}

impl FromWorld for FlashGpu {
    fn from_world(world: &mut World) -> Self {
        let shape_shader = load_embedded_asset!(world, "shaders/shape.wesl");
        let composite_shader = load_embedded_asset!(world, "shaders/composite.wesl");
        let blur_shader = load_embedded_asset!(world, "shaders/blur.wesl");
        let glow_shader = load_embedded_asset!(world, "shaders/glow.wesl");
        let bevel_shader = load_embedded_asset!(world, "shaders/bevel.wesl");
        let color_matrix_shader = load_embedded_asset!(world, "shaders/color_matrix.wesl");
        let convolution_shader = load_embedded_asset!(world, "shaders/convolution.wesl");
        let gradient_filter_shader = load_embedded_asset!(world, "shaders/gradient_filter.wesl");
        let alpha_mask_shader = load_embedded_asset!(world, "shaders/alpha_mask.wesl");
        let complex_blend_shader = load_embedded_asset!(world, "shaders/complex_blend.wesl");
        let fullscreen_shader = load_embedded_asset!(world, "shaders/fullscreen.wesl");
        let device = world.resource::<RenderDevice>();
        let cache = world.resource::<PipelineCache>();
        let uniform_layout = BindGroupLayoutDescriptor::new(
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
        let texture_layout = BindGroupLayoutDescriptor::new(
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
        let convolution_kernel_layout = BindGroupLayoutDescriptor::new(
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
        let pipeline_layout = vec![uniform_layout.clone(), texture_layout.clone()];
        let glow_pipeline_layout = vec![
            uniform_layout.clone(),
            texture_layout.clone(),
            texture_layout.clone(),
        ];
        let convolution_pipeline_layout = vec![
            uniform_layout.clone(),
            texture_layout.clone(),
            convolution_kernel_layout.clone(),
        ];
        let gradient_filter_pipeline_layout = vec![
            uniform_layout.clone(),
            texture_layout.clone(),
            texture_layout.clone(),
            texture_layout.clone(),
        ];
        let uniform_layout = cache.get_bind_group_layout(&uniform_layout);
        let texture_layout = cache.get_bind_group_layout(&texture_layout);
        let convolution_kernel_layout = cache.get_bind_group_layout(&convolution_kernel_layout);
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
            _fullscreen_shader: fullscreen_shader,
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
            complex_blend_shader,
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
            image_bind_groups: HashMap::default(),
            filter_cache_tick: 0,
            alpha_masks: HashMap::default(),
            complex_blends: HashMap::default(),
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
    fn image_bind_group(
        &mut self,
        device: &RenderDevice,
        id: AssetId<Image>,
        image: &GpuImage,
    ) -> BindGroup {
        let texture_view = image.texture_view.id();
        let sampler = image.sampler.id();
        let rebuild = self
            .image_bind_groups
            .get(&id)
            .is_none_or(|cached| cached.texture_view != texture_view || cached.sampler != sampler);
        if rebuild {
            let bind_group = texture_group(
                device,
                &self.texture_layout,
                &image.texture_view,
                &image.sampler,
            );
            self.image_bind_groups.insert(
                id,
                CachedImageBindGroup {
                    texture_view,
                    sampler,
                    bind_group: bind_group.clone(),
                },
            );
            bind_group
        } else {
            self.image_bind_groups[&id].bind_group.clone()
        }
    }

    pub(super) fn retain_image_bind_groups(&mut self, images: &RenderAssets<GpuImage>) {
        self.image_bind_groups
            .retain(|id, _| images.get(*id).is_some());
    }

    fn shape_pipeline(
        &mut self,
        cache: &PipelineCache,
        mesh: &RenderMesh,
        samples: u32,
        blend: VabBlendMode,
    ) -> Result<CachedRenderPipelineId> {
        let key = (mesh.layout.clone(), samples, blend);
        if let Some(id) = self.shapes.get(&key) {
            return Ok(*id);
        }
        let compact = mesh.layout.0.contains(ATTRIBUTE_VAB_POSITION);
        let (layout, shader_defs) = if compact {
            (
                mesh.layout.0.get_layout(&[
                    ATTRIBUTE_VAB_POSITION.at_shader_location(0),
                    ATTRIBUTE_VAB_COLOR_SRGB.at_shader_location(1),
                ])?,
                vec!["COMPACT_VERTEX".into()],
            )
        } else {
            (
                mesh.layout.0.get_layout(&[
                    Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
                    Mesh::ATTRIBUTE_COLOR.at_shader_location(1),
                ])?,
                Vec::new(),
            )
        };
        let id = queue_pipeline(
            cache,
            &self.shape_shader,
            shader_defs,
            self.pipeline_layout.clone(),
            vec![layout],
            TextureFormat::Rgba8Unorm,
            samples,
            Some(fixed_blend_state(blend)),
        );
        self.shapes.insert(key, id);
        Ok(id)
    }
    fn composite_pipeline(
        &mut self,
        cache: &PipelineCache,
        format: TextureFormat,
        samples: u32,
        blend: bool,
    ) -> CachedRenderPipelineId {
        let key = (format, samples, blend);
        *self.composites.entry(key).or_insert_with(|| {
            queue_pipeline(
                cache,
                &self.composite_shader,
                Vec::new(),
                self.pipeline_layout.clone(),
                vec![],
                format,
                samples,
                blend.then_some(BlendState::PREMULTIPLIED_ALPHA_BLENDING),
            )
        })
    }
    fn layer_composite_pipeline(
        &mut self,
        cache: &PipelineCache,
        format: TextureFormat,
        samples: u32,
        blend: VabBlendMode,
    ) -> CachedRenderPipelineId {
        let key = (format, samples, blend);
        *self.layer_composites.entry(key).or_insert_with(|| {
            queue_pipeline(
                cache,
                &self.composite_shader,
                Vec::new(),
                self.pipeline_layout.clone(),
                vec![],
                format,
                samples,
                Some(fixed_blend_state(blend)),
            )
        })
    }
    fn blur_pipeline(
        &mut self,
        cache: &PipelineCache,
        format: TextureFormat,
    ) -> CachedRenderPipelineId {
        *self.blurs.entry(format).or_insert_with(|| {
            queue_pipeline(
                cache,
                &self.blur_shader,
                Vec::new(),
                self.pipeline_layout.clone(),
                vec![],
                format,
                1,
                None,
            )
        })
    }
    fn glow_pipeline(
        &mut self,
        cache: &PipelineCache,
        format: TextureFormat,
    ) -> CachedRenderPipelineId {
        *self.glows.entry(format).or_insert_with(|| {
            queue_pipeline(
                cache,
                &self.glow_shader,
                Vec::new(),
                self.glow_pipeline_layout.clone(),
                vec![],
                format,
                1,
                None,
            )
        })
    }
    fn bevel_pipeline(
        &mut self,
        cache: &PipelineCache,
        format: TextureFormat,
    ) -> CachedRenderPipelineId {
        *self.bevels.entry(format).or_insert_with(|| {
            queue_pipeline(
                cache,
                &self.bevel_shader,
                Vec::new(),
                self.glow_pipeline_layout.clone(),
                vec![],
                format,
                1,
                None,
            )
        })
    }
    fn color_matrix_pipeline(
        &mut self,
        cache: &PipelineCache,
        format: TextureFormat,
    ) -> CachedRenderPipelineId {
        *self.color_matrices.entry(format).or_insert_with(|| {
            queue_pipeline(
                cache,
                &self.color_matrix_shader,
                Vec::new(),
                self.pipeline_layout.clone(),
                vec![],
                format,
                1,
                None,
            )
        })
    }
    fn convolution_pipeline(
        &mut self,
        cache: &PipelineCache,
        format: TextureFormat,
    ) -> CachedRenderPipelineId {
        *self.convolutions.entry(format).or_insert_with(|| {
            queue_pipeline(
                cache,
                &self.convolution_shader,
                Vec::new(),
                self.convolution_pipeline_layout.clone(),
                vec![],
                format,
                1,
                None,
            )
        })
    }
    fn gradient_filter_pipeline(
        &mut self,
        cache: &PipelineCache,
        format: TextureFormat,
    ) -> CachedRenderPipelineId {
        *self.gradient_filters.entry(format).or_insert_with(|| {
            queue_pipeline(
                cache,
                &self.gradient_filter_shader,
                Vec::new(),
                self.gradient_filter_pipeline_layout.clone(),
                vec![],
                format,
                1,
                None,
            )
        })
    }
    fn convolution_kernel_group(
        &mut self,
        device: &RenderDevice,
        filter: &AnimConvolutionFilter,
    ) -> BindGroup {
        self.filter_cache_tick = self.filter_cache_tick.wrapping_add(1);
        let tick = self.filter_cache_tick;
        let key = ConvolutionKernelKey {
            rows: filter.num_matrix_rows,
            cols: filter.num_matrix_cols,
            weights: filter.matrix.iter().map(|value| value.to_bits()).collect(),
        };
        if let Some((group, last_used)) = self.convolution_kernels.get_mut(&key) {
            *last_used = tick;
            return group.clone();
        }
        if self.convolution_kernels.len() >= MAX_CACHED_FILTER_RESOURCES
            && let Some(oldest) = self
                .convolution_kernels
                .iter()
                .min_by_key(|(_, (_, last_used))| *last_used)
                .map(|(k, _)| k.clone())
        {
            self.convolution_kernels.remove(&oldest);
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
        self.convolution_kernels.insert(key, (group.clone(), tick));
        group
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
        self.filter_cache_tick = self.filter_cache_tick.wrapping_add(1);
        let tick = self.filter_cache_tick;
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
        if let Some((group, last_used)) = self.gradient_ramps.get_mut(&key) {
            *last_used = tick;
            return Ok(group.clone());
        }
        if self.gradient_ramps.len() >= MAX_CACHED_FILTER_RESOURCES
            && let Some(oldest) = self
                .gradient_ramps
                .iter()
                .min_by_key(|(_, (_, last_used))| *last_used)
                .map(|(k, _)| k.clone())
        {
            self.gradient_ramps.remove(&oldest);
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
        self.gradient_ramps.insert(key, (group.clone(), tick));
        Ok(group)
    }
    fn alpha_mask_pipeline(
        &mut self,
        cache: &PipelineCache,
        format: TextureFormat,
        samples: u32,
    ) -> CachedRenderPipelineId {
        let key = (format, samples);
        *self.alpha_masks.entry(key).or_insert_with(|| {
            queue_pipeline(
                cache,
                &self.alpha_mask_shader,
                Vec::new(),
                self.glow_pipeline_layout.clone(),
                vec![],
                format,
                samples,
                Some(BlendState::PREMULTIPLIED_ALPHA_BLENDING),
            )
        })
    }
    fn complex_blend_pipeline(
        &mut self,
        cache: &PipelineCache,
        format: TextureFormat,
        samples: u32,
    ) -> CachedRenderPipelineId {
        let key = (format, samples);
        *self.complex_blends.entry(key).or_insert_with(|| {
            queue_pipeline(
                cache,
                &self.complex_blend_shader,
                Vec::new(),
                self.glow_pipeline_layout.clone(),
                vec![],
                format,
                samples,
                None,
            )
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn queue_pipeline(
    cache: &PipelineCache,
    shader: &Handle<Shader>,
    shader_defs: Vec<ShaderDefVal>,
    layout: Vec<BindGroupLayoutDescriptor>,
    buffers: Vec<VertexBufferLayout>,
    format: TextureFormat,
    samples: u32,
    blend: Option<BlendState>,
) -> CachedRenderPipelineId {
    cache.queue_render_pipeline(RenderPipelineDescriptor {
        label: Some("flash_pipeline".into()),
        layout,
        vertex: VertexState {
            shader: shader.clone(),
            shader_defs: shader_defs.clone(),
            entry_point: Some("vertex".into()),
            buffers,
            ..default()
        },
        fragment: Some(FragmentState {
            shader: shader.clone(),
            shader_defs,
            entry_point: Some("fragment".into()),
            targets: vec![Some(ColorTargetState {
                format,
                blend,
                write_mask: ColorWrites::ALL,
            })],
            ..default()
        }),
        multisample: MultisampleState {
            count: samples,
            ..default()
        },
        ..default()
    })
}

fn ready_pipeline(cache: &PipelineCache, id: CachedRenderPipelineId) -> Result<RenderPipeline> {
    if let Some(pipeline) = cache.get_render_pipeline(id) {
        return Ok(pipeline.clone());
    }
    // Shader assets and imported modules can arrive after the first queue pass.
    match cache.get_render_pipeline_state(id) {
        CachedPipelineState::Err(
            ShaderCacheError::ShaderNotLoaded(_) | ShaderCacheError::ShaderImportNotYetAvailable,
        ) => {}
        CachedPipelineState::Err(error) => {
            anyhow::bail!("Flash pipeline compilation failed: {error}")
        }
        _ => {}
    }
    anyhow::bail!("pending Flash pipeline")
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
enum FilterPipeline {
    Blur,
    Glow,
    Bevel,
    ColorMatrix,
    Convolution,
    Gradient,
    Mask,
}

/// A complete snapshot for one submission. Rendering never queues a pipeline.
#[derive(Default)]
struct PreparedPipelines(HashMap<FilterPipeline, RenderPipeline>);
impl PreparedPipelines {
    fn get(&self, kind: FilterPipeline) -> RenderPipeline {
        self.0
            .get(&kind)
            .expect("pipeline preflight must cover every prepared filter")
            .clone()
    }
}

impl FlashGpu {
    fn request_ops(
        &mut self,
        ops: &[Op],
        samples: u32,
        cache: &PipelineCache,
        meshes: &RenderAssets<RenderMesh>,
        ids: &mut Vec<CachedRenderPipelineId>,
        filters: &mut HashMap<FilterPipeline, CachedRenderPipelineId>,
    ) -> Result<()> {
        let format = TextureFormat::Rgba8Unorm;
        for op in ops {
            match op {
                Op::Draw(draw) => {
                    if let Some(mesh) = meshes.get(draw.mesh) {
                        ids.push(self.shape_pipeline(cache, mesh, samples, draw.blend)?);
                    }
                }
                Op::Mask { mask, content, .. } => {
                    filters.insert(
                        FilterPipeline::Mask,
                        self.alpha_mask_pipeline(cache, format, samples),
                    );
                    self.request_ops(mask, samples, cache, meshes, ids, filters)?;
                    self.request_ops(content, samples, cache, meshes, ids, filters)?;
                }
                Op::Layer {
                    ops,
                    filters: effects,
                    blend,
                    ..
                } => {
                    if complex_blend_mode_id(*blend).is_some() {
                        ids.push(self.complex_blend_pipeline(cache, format, samples));
                    } else {
                        ids.push(self.layer_composite_pipeline(cache, format, samples, *blend));
                    }
                    self.request_ops(ops, samples, cache, meshes, ids, filters)?;
                    for effect in effects {
                        let (kind, id) = match effect {
                            AnimFilter::BlurFilter(_) => {
                                (FilterPipeline::Blur, self.blur_pipeline(cache, format))
                            }
                            AnimFilter::GlowFilter(_) | AnimFilter::DropShadowFilter(_) => {
                                (FilterPipeline::Glow, self.glow_pipeline(cache, format))
                            }
                            AnimFilter::BevelFilter(_) => {
                                (FilterPipeline::Bevel, self.bevel_pipeline(cache, format))
                            }
                            AnimFilter::ColorMatrixFilter(_) => (
                                FilterPipeline::ColorMatrix,
                                self.color_matrix_pipeline(cache, format),
                            ),
                            AnimFilter::ConvolutionFilter(_) => (
                                FilterPipeline::Convolution,
                                self.convolution_pipeline(cache, format),
                            ),
                            AnimFilter::GradientGlowFilter(_)
                            | AnimFilter::GradientBevelFilter(_) => (
                                FilterPipeline::Gradient,
                                self.gradient_filter_pipeline(cache, format),
                            ),
                        };
                        filters.insert(kind, id);
                        if matches!(
                            kind,
                            FilterPipeline::Glow | FilterPipeline::Bevel | FilterPipeline::Gradient
                        ) {
                            filters.insert(FilterPipeline::Blur, self.blur_pipeline(cache, format));
                        }
                    }
                }
            }
        }
        Ok(())
    }
    fn prepare_pipelines(
        &mut self,
        ops: &[Op],
        samples: u32,
        cache: &PipelineCache,
        meshes: &RenderAssets<RenderMesh>,
        output: Option<CachedRenderPipelineId>,
    ) -> Result<PreparedPipelines> {
        let mut ids = Vec::new();
        let mut filters = HashMap::default();
        // Queue the entire tree before checking readiness, allowing parallel compilation.
        self.request_ops(ops, samples, cache, meshes, &mut ids, &mut filters)?;
        ids.extend(output);
        ids.extend(filters.values().copied());
        for id in ids {
            ready_pipeline(cache, id)?;
        }
        Ok(PreparedPipelines(
            filters
                .into_iter()
                .map(|(kind, id)| ready_pipeline(cache, id).map(|pipeline| (kind, pipeline)))
                .collect::<Result<_>>()?,
        ))
    }
}

fn complex_blend_mode_id(mode: VabBlendMode) -> Option<f32> {
    match mode {
        VabBlendMode::Multiply => Some(1.0),
        VabBlendMode::Lighten => Some(2.0),
        VabBlendMode::Darken => Some(3.0),
        VabBlendMode::Difference => Some(4.0),
        VabBlendMode::Invert => Some(5.0),
        VabBlendMode::Alpha => Some(6.0),
        VabBlendMode::Erase => Some(7.0),
        VabBlendMode::Overlay => Some(8.0),
        VabBlendMode::HardLight => Some(9.0),
        VabBlendMode::Normal
        | VabBlendMode::Layer
        | VabBlendMode::Add
        | VabBlendMode::Subtract
        | VabBlendMode::Screen => None,
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
    Blur {
        offsets: Vec<u32>,
        blur_size: Option<[u32; 2]>,
    },
    Glow {
        blur_offsets: Vec<u32>,
        blur_size: Option<[u32; 2]>,
        glow_offset: u32,
    },
    Bevel {
        blur_offsets: Vec<u32>,
        blur_size: Option<[u32; 2]>,
        bevel_offset: u32,
    },
    ColorMatrix(u32),
    Convolution {
        offset: u32,
        kernel: BindGroup,
    },
    Gradient {
        blur_offsets: Vec<u32>,
        blur_size: Option<[u32; 2]>,
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
        needs_full_res_ping_pong: bool,
        is_complex_blend: bool,
        composite: RenderPipeline,
    },
}
pub(super) struct PreparedLayer {
    pipelines: PreparedPipelines,
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
                    let mut current_area = (bounds[2] as u64).saturating_mul(bounds[3] as u64);
                    for filter in filters {
                        match filter {
                            PreparedFilter::Blur { offsets, blur_size } => {
                                let blur_area = blur_size.map_or(current_area, |[w, h]| {
                                    u64::from(w).saturating_mul(u64::from(h))
                                });
                                let n = offsets.len() as u64;
                                passes = passes.saturating_add(n);
                                pixels = pixels.saturating_add(blur_area.saturating_mul(n));
                                if blur_size.is_some() {
                                    current_area = blur_area;
                                }
                            }
                            PreparedFilter::Glow {
                                blur_offsets,
                                blur_size,
                                ..
                            }
                            | PreparedFilter::Bevel {
                                blur_offsets,
                                blur_size,
                                ..
                            }
                            | PreparedFilter::Gradient {
                                blur_offsets,
                                blur_size,
                                ..
                            } => {
                                let blur_area = blur_size.map_or(current_area, |[w, h]| {
                                    u64::from(w).saturating_mul(u64::from(h))
                                });
                                let n = blur_offsets.len() as u64;
                                passes = passes.saturating_add(n + 1);
                                pixels = pixels
                                    .saturating_add(blur_area.saturating_mul(n))
                                    .saturating_add(current_area);
                            }
                            PreparedFilter::ColorMatrix(_) | PreparedFilter::Convolution { .. } => {
                                passes = passes.saturating_add(1);
                                pixels = pixels.saturating_add(current_area);
                            }
                        }
                    }
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

fn downsampled_blur_size(
    size: [f32; 2],
    blur_x: i32,
    blur_y: i32,
    quality: VabFilterQuality,
) -> Option<[u32; 2]> {
    if !quality.blur_scale.is_finite() || quality.blur_scale >= 0.999 {
        return None;
    }
    let scale = quality.blur_scale.clamp(0.125, 1.0);
    let min_dim = quality.min_dimension.max(1) as f32;
    let min_blur = if quality.min_blur_radius.is_finite() {
        quality.min_blur_radius.max(1.0)
    } else {
        return None;
    };
    let bx = (blur_x as f32 / 65536.0).clamp(1.0, 255.0);
    let by = (blur_y as f32 / 65536.0).clamp(1.0, 255.0);
    let sx = if size[0] >= min_dim && bx >= min_blur {
        scale
    } else {
        1.0
    };
    let sy = if size[1] >= min_dim && by >= min_blur {
        scale
    } else {
        1.0
    };
    if sx >= 0.999 && sy >= 0.999 {
        return None;
    }
    let full_w = size[0] as u32;
    let full_h = size[1] as u32;
    let bw = classify_texture_dimension(((size[0] * sx).ceil() as u32).max(1)).min(full_w);
    let bh = classify_texture_dimension(((size[1] * sy).ceil() as u32).max(1)).min(full_h);
    if bw >= full_w && bh >= full_h {
        return None;
    }
    Some([bw, bh])
}

fn prepare_blur_with_quality(
    filter: &AnimBlurFilter,
    size: [f32; 2],
    quality: VabFilterQuality,
    allow_downsample: bool,
    uniforms: &mut Uniforms,
) -> Result<(Vec<u32>, Option<[u32; 2]>)> {
    let blur_size = allow_downsample
        .then(|| downsampled_blur_size(size, filter.blur_x, filter.blur_y, quality))
        .flatten();
    let (target_size, scaled_filter) = if let Some([bw, bh]) = blur_size {
        let rx = bw as f32 / size[0].max(1.0);
        let ry = bh as f32 / size[1].max(1.0);
        let scale_axis = |blur: i32, ratio: f32| -> i32 {
            let full_size = (blur as f32 / 65536.0).clamp(1.0, 255.0);
            if full_size <= 1.0 {
                return blur;
            }
            let scaled = 1.0 + (full_size - 1.0) * ratio;
            (scaled * 65536.0).round() as i32
        };
        (
            [bw as f32, bh as f32],
            AnimBlurFilter {
                blur_x: scale_axis(filter.blur_x, rx),
                blur_y: scale_axis(filter.blur_y, ry),
                num_passes: filter.num_passes,
            },
        )
    } else {
        (size, filter.clone())
    };
    let offsets = prepare_blur(&scaled_filter, target_size, uniforms)?;
    let blur_size = if offsets.is_empty() { None } else { blur_size };
    Ok((offsets, blur_size))
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
    quality: VabFilterQuality,
    uniforms: &mut Uniforms,
) -> Result<PreparedFilter> {
    let (blur_offsets, blur_size) = prepare_blur_with_quality(
        &AnimBlurFilter {
            blur_x: filter.blur_x,
            blur_y: filter.blur_y,
            num_passes: filter.num_passes,
        },
        size,
        quality,
        true,
        uniforms,
    )?;
    let glow_offset = uniforms.push(&glow_uniform(filter))?;
    Ok(PreparedFilter::Glow {
        blur_offsets,
        blur_size,
        glow_offset,
    })
}

fn prepare_drop_shadow(
    filter: &AnimDropShadowFilter,
    size: [f32; 2],
    quality: VabFilterQuality,
    uniforms: &mut Uniforms,
) -> Result<PreparedFilter> {
    let (blur_offsets, blur_size) = prepare_blur_with_quality(
        &AnimBlurFilter {
            blur_x: filter.blur_x,
            blur_y: filter.blur_y,
            num_passes: filter.num_passes,
        },
        size,
        quality,
        true,
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
        blur_size,
        glow_offset,
    })
}

fn prepare_bevel(
    filter: &AnimBevelFilter,
    size: [f32; 2],
    quality: VabFilterQuality,
    uniforms: &mut Uniforms,
) -> Result<PreparedFilter> {
    let (blur_offsets, blur_size) = prepare_blur_with_quality(
        &AnimBlurFilter {
            blur_x: filter.blur_x,
            blur_y: filter.blur_y,
            num_passes: filter.num_passes,
        },
        size,
        quality,
        true,
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
        blur_size,
        bevel_offset,
    })
}

#[allow(clippy::too_many_arguments)]
fn prepare_gradient_filter(
    filter: &AnimGradientFilter,
    size: [f32; 2],
    bevel: bool,
    quality: VabFilterQuality,
    gpu: &mut FlashGpu,
    device: &RenderDevice,
    queue: &RenderQueue,
    uniforms: &mut Uniforms,
) -> Result<PreparedFilter> {
    let (blur_offsets, blur_size) = prepare_blur_with_quality(
        &AnimBlurFilter {
            blur_x: filter.blur_x,
            blur_y: filter.blur_y,
            num_passes: filter.num_passes,
        },
        size,
        quality,
        true,
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
        blur_size,
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
    quality: VabFilterQuality,
    gpu: &mut FlashGpu,
    cache: &PipelineCache,
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
                        gpu.image_bind_group(device, id, image)
                    }
                    None => gpu.white.clone(),
                };
                let m = draw.transform.matrix;
                let c = draw.transform.color_transform;
                let uv = draw.uv;
                let offset = uniforms.push(&DrawUniform {
                    row0: [m.a, m.c, m.tx, 0.0],
                    row1: [m.b, m.d, m.ty, 0.0],
                    uv0: [uv.x_axis.x, uv.y_axis.x, uv.z_axis.x, draw.ramp_v],
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
                    pipeline: ready_pipeline(
                        cache,
                        gpu.shape_pipeline(cache, mesh, samples, draw.blend)?,
                    )?,
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
                    mask, child, samples, quality, gpu, cache, device, queue, meshes, images,
                    uniforms,
                )?;
                let content = prepare_ops(
                    content, child, samples, quality, gpu, cache, device, queue, meshes, images,
                    uniforms,
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
                    ops, child, samples, quality, gpu, cache, device, queue, meshes, images,
                    uniforms,
                )?;
                let mut prepared_filters = Vec::new();
                let mut current_size = [child[2], child[3]];
                let mut needs_full_res_ping_pong = false;
                for (filter_index, filter) in filters.iter().enumerate() {
                    let prepared = match filter {
                        AnimFilter::BlurFilter(filter) => {
                            let tail_only_blurs = filters[filter_index + 1..]
                                .iter()
                                .all(|next| matches!(next, AnimFilter::BlurFilter(_)));
                            let (offsets, blur_size) = prepare_blur_with_quality(
                                filter,
                                current_size,
                                quality,
                                tail_only_blurs,
                                uniforms,
                            )?;
                            if let Some([bw, bh]) = blur_size {
                                current_size = [bw as f32, bh as f32];
                            } else if !offsets.is_empty() {
                                needs_full_res_ping_pong = true;
                            }
                            PreparedFilter::Blur { offsets, blur_size }
                        }
                        AnimFilter::GlowFilter(filter) => {
                            needs_full_res_ping_pong = true;
                            prepare_glow(filter, current_size, quality, uniforms)?
                        }
                        AnimFilter::ColorMatrixFilter(filter) => {
                            needs_full_res_ping_pong = true;
                            PreparedFilter::ColorMatrix(
                                uniforms.push(&color_matrix_uniform(filter))?,
                            )
                        }
                        AnimFilter::DropShadowFilter(filter) => {
                            needs_full_res_ping_pong = true;
                            prepare_drop_shadow(filter, current_size, quality, uniforms)?
                        }
                        AnimFilter::BevelFilter(filter) => {
                            needs_full_res_ping_pong = true;
                            prepare_bevel(filter, current_size, quality, uniforms)?
                        }
                        AnimFilter::ConvolutionFilter(filter) => {
                            needs_full_res_ping_pong = true;
                            prepare_convolution(filter, current_size, gpu, device, uniforms)?
                        }
                        AnimFilter::GradientGlowFilter(filter) => {
                            needs_full_res_ping_pong = true;
                            prepare_gradient_filter(
                                filter,
                                current_size,
                                false,
                                quality,
                                gpu,
                                device,
                                queue,
                                uniforms,
                            )?
                        }
                        AnimFilter::GradientBevelFilter(filter) => {
                            needs_full_res_ping_pong = true;
                            prepare_gradient_filter(
                                filter,
                                current_size,
                                true,
                                quality,
                                gpu,
                                device,
                                queue,
                                uniforms,
                            )?
                        }
                    };
                    prepared_filters.push(prepared);
                }
                let complex_mode = complex_blend_mode_id(*blend);
                let offset = uniforms.push(&CompositeUniform {
                    rect: child,
                    viewport: bounds,
                    mode: [
                        complex_mode.unwrap_or(0.0),
                        if *blend == VabBlendMode::Alpha {
                            1.0
                        } else {
                            0.0
                        },
                        0.0,
                        0.0,
                    ],
                    color: [0.0; 4],
                })?;
                let composite = if complex_mode.is_some() {
                    ready_pipeline(
                        cache,
                        gpu.complex_blend_pipeline(cache, TextureFormat::Rgba8Unorm, samples),
                    )?
                } else {
                    ready_pipeline(
                        cache,
                        gpu.layer_composite_pipeline(
                            cache,
                            TextureFormat::Rgba8Unorm,
                            samples,
                            *blend,
                        ),
                    )?
                };
                PreparedOp::Layer {
                    ops,
                    bounds: child,
                    offset,
                    filters: prepared_filters,
                    needs_full_res_ping_pong,
                    is_complex_blend: complex_mode.is_some(),
                    composite,
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
    quality: VabFilterQuality,
    gpu: &mut FlashGpu,
    cache: &PipelineCache,
    device: &RenderDevice,
    queue: &RenderQueue,
    meshes: &RenderAssets<RenderMesh>,
    images: &RenderAssets<GpuImage>,
    uniform_buffer: &mut ReusableUniformBuffer,
) -> Result<(PreparedLayer, bool)> {
    let pipelines = gpu.prepare_pipelines(ops, samples, cache, meshes, None)?;
    let mut uniforms = Uniforms {
        bytes: Vec::new(),
        alignment: (device.limits().min_uniform_buffer_offset_alignment as usize).max(128),
    };
    let ops = prepare_ops(
        ops,
        bounds,
        samples,
        quality,
        gpu,
        cache,
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
            pipelines,
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
    quality: Res<VabFilterQuality>,
    cache: Res<PipelineCache>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    meshes: Res<RenderAssets<RenderMesh>>,
    images: Res<RenderAssets<GpuImage>>,
    mut uniform_buffers: ResMut<OffscreenUniformBuffers>,
    mut diagnostics: ResMut<FlashRenderDiagnostics>,
) {
    diagnostics.pending_views = 0;
    diagnostics.errors.clear();
    gpu.retain_image_bind_groups(&images);
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
            let output_id =
                gpu.composite_pipeline(&cache, output.texture_descriptor.format, 1, false);
            let pipelines = gpu.prepare_pipelines(
                &frame.ops,
                msaa.samples(),
                &cache,
                &meshes,
                Some(output_id),
            )?;
            let mut uniforms = Uniforms {
                bytes: Vec::new(),
                alignment: (device.limits().min_uniform_buffer_offset_alignment as usize).max(128),
            };
            let ops = prepare_ops(
                &frame.ops,
                bounds,
                msaa.samples(),
                *quality,
                &mut gpu,
                &cache,
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
                    pipelines,
                    ops,
                    uniforms,
                    bounds,
                },
                output_offset,
                output: ready_pipeline(&cache, output_id)?,
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
            | TextureUsages::COPY_SRC
            | TextureUsages::COPY_DST,
        view_formats: &[],
    }
}

#[allow(clippy::too_many_arguments)]
fn render_single_texture_filter_pass(
    source_group: &BindGroup,
    destination: &TextureView,
    offset: u32,
    label: &'static str,
    prepared: &PreparedLayer,
    pipeline: &RenderPipeline,
    context: &mut RenderContext,
) {
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
    pass.set_bind_group(1, source_group, &[]);
    pass.draw(0..6, 0..1);
}

#[allow(clippy::too_many_arguments)]
fn render_convolution_pass(
    source_group: &BindGroup,
    destination: &TextureView,
    offset: u32,
    kernel: &BindGroup,
    prepared: &PreparedLayer,
    pipeline: &RenderPipeline,
    context: &mut RenderContext,
) {
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
    pass.set_bind_group(1, source_group, &[]);
    pass.set_bind_group(2, kernel, &[]);
    pass.draw(0..6, 0..1);
}

/// Run blur passes at a downsampled resolution `(bw, bh)`, starting from `source_group`.
/// Returns the pooled texture holding the final blurred image.
#[allow(clippy::too_many_arguments)]
fn render_downsampled_blur_chain(
    source_group: &BindGroup,
    blur_offsets: &[u32],
    blur_size: [u32; 2],
    bounds: [f32; 4],
    blur_label: &'static str,
    prepared: &PreparedLayer,
    gpu: &FlashGpu,
    device: &RenderDevice,
    pool: &mut FrameInternalTextureCache,
    context: &mut RenderContext,
    draw_calls: &mut u64,
) -> super::pool::Lease<super::texture_cache::PooledTexture> {
    let blur_pipeline = prepared.pipelines.get(FilterPipeline::Blur);
    let blur_a = pool.acquire(
        device,
        descriptor(blur_size[0], blur_size[1]),
        TextureAllocationSite {
            purpose: TexturePurpose::BlurDownsample,
            bounds,
        },
    );
    let blur_b = (blur_offsets.len() > 1).then(|| {
        pool.acquire(
            device,
            descriptor(blur_size[0], blur_size[1]),
            TextureAllocationSite {
                purpose: TexturePurpose::BlurDownsample,
                bounds,
            },
        )
    });
    let mut in_a = true;
    for (pass_index, blur_offset) in blur_offsets.iter().enumerate() {
        if pass_index == 0 {
            render_single_texture_filter_pass(
                source_group,
                blur_a.default_view(),
                *blur_offset,
                blur_label,
                prepared,
                &blur_pipeline,
                context,
            );
            in_a = true;
        } else {
            let blur_b = blur_b
                .as_ref()
                .expect("multi-pass blur allocates ping-pong");
            let (src_group, dst_view) = if in_a {
                (
                    blur_a.filter_bind_group(device, &gpu.texture_layout, &gpu.sampler),
                    blur_b.default_view(),
                )
            } else {
                (
                    blur_b.filter_bind_group(device, &gpu.texture_layout, &gpu.sampler),
                    blur_a.default_view(),
                )
            };
            render_single_texture_filter_pass(
                src_group,
                dst_view,
                *blur_offset,
                blur_label,
                prepared,
                &blur_pipeline,
                context,
            );
            in_a = !in_a;
        }
        *draw_calls += 1;
    }
    if in_a {
        blur_a
    } else {
        blur_b.expect("multi-pass blur allocates ping-pong")
    }
}

#[allow(clippy::too_many_arguments)]
fn render_blur_and_two_texture_filter(
    child: &mut super::texture_cache::TextureTarget,
    bounds: [f32; 4],
    blur_offsets: &[u32],
    blur_size: Option<[u32; 2]>,
    effect_offset: u32,
    blur_label: &'static str,
    effect_label: &'static str,
    effect_pipeline: &RenderPipeline,
    extra_group: Option<&BindGroup>,
    prepared: &PreparedLayer,
    gpu: &FlashGpu,
    device: &RenderDevice,
    pool: &mut FrameInternalTextureCache,
    context: &mut RenderContext,
    draw_calls: &mut u64,
) {
    // Fast path: run blur passes at reduced resolution while keeping the foreground
    // source and final composite at full resolution.
    if let Some(blur_size) = blur_size
        && !blur_offsets.is_empty()
    {
        let source_group = child
            .main_filter_bind_group(device, &gpu.texture_layout, &gpu.sampler)
            .clone();
        let blurred_tex = render_downsampled_blur_chain(
            &source_group,
            blur_offsets,
            blur_size,
            bounds,
            blur_label,
            prepared,
            gpu,
            device,
            pool,
            context,
            draw_calls,
        );
        let blurred_group =
            blurred_tex.filter_bind_group(device, &gpu.texture_layout, &gpu.sampler);
        let attachment = [Some(RenderPassColorAttachment {
            view: child.other_view(),
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
        pass.set_bind_group(2, blurred_group, &[]);
        if let Some(extra_group) = extra_group {
            pass.set_bind_group(3, extra_group, &[]);
        }
        pass.draw(0..6, 0..1);
        drop(pass);
        child.flip_main();
        *draw_calls += 1;
        return;
    }

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
    let blur_pipeline = prepared.pipelines.get(FilterPipeline::Blur);
    let mut blurred_in_other = false;
    for (pass_index, blur_offset) in blur_offsets.iter().enumerate() {
        let (source_group, destination) = if pass_index == 0 {
            (
                child
                    .main_filter_bind_group(device, &gpu.texture_layout, &gpu.sampler)
                    .clone(),
                child.other_view(),
            )
        } else if blurred_in_other {
            (
                child
                    .other_filter_bind_group(device, &gpu.texture_layout, &gpu.sampler)
                    .clone(),
                scratch.default_view(),
            )
        } else {
            (
                scratch
                    .filter_bind_group(device, &gpu.texture_layout, &gpu.sampler)
                    .clone(),
                child.other_view(),
            )
        };
        render_single_texture_filter_pass(
            &source_group,
            destination,
            *blur_offset,
            blur_label,
            prepared,
            &blur_pipeline,
            context,
        );
        blurred_in_other = pass_index == 0 || !blurred_in_other;
        *draw_calls += 1;
    }
    let source_group = child
        .main_filter_bind_group(device, &gpu.texture_layout, &gpu.sampler)
        .clone();
    let blurred_group = if blur_offsets.is_empty() {
        source_group.clone()
    } else if blurred_in_other {
        child
            .other_filter_bind_group(device, &gpu.texture_layout, &gpu.sampler)
            .clone()
    } else {
        scratch
            .filter_bind_group(device, &gpu.texture_layout, &gpu.sampler)
            .clone()
    };
    let result_in_other = !blurred_in_other || blur_offsets.is_empty();
    let destination = if result_in_other {
        child.other_view()
    } else {
        scratch.default_view()
    };
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
fn render_layer_into_target(
    children: &[PreparedOp],
    bounds: [f32; 4],
    filters: &[PreparedFilter],
    needs_full_res_ping_pong: bool,
    prepared: &PreparedLayer,
    gpu: &mut FlashGpu,
    device: &RenderDevice,
    pool: &mut FrameInternalTextureCache,
    msaa: &Msaa,
    meshes: &RenderAssets<RenderMesh>,
    allocator: &MeshAllocator,
    context: &mut RenderContext,
    draw_calls: &mut u64,
) -> super::texture_cache::TextureTarget {
    let mut child = pool.get(
        device,
        descriptor(bounds[2] as u32, bounds[3] as u32),
        msaa,
        TextureAllocationSite {
            purpose: TexturePurpose::Layer,
            bounds,
        },
    );
    render_ops(
        children,
        &child,
        Some(Srgba::NONE),
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
    child.release_msaa_attachment();
    if needs_full_res_ping_pong {
        child.enable_ping_pong(pool.acquire(
            device,
            descriptor(bounds[2] as u32, bounds[3] as u32),
            TextureAllocationSite {
                purpose: TexturePurpose::PingPong,
                bounds,
            },
        ));
    }
    for filter in filters {
        match filter {
            PreparedFilter::Blur { offsets, blur_size } => {
                if let Some(blur_size) = *blur_size
                    && !offsets.is_empty()
                {
                    let source_group = child
                        .main_filter_bind_group(device, &gpu.texture_layout, &gpu.sampler)
                        .clone();
                    let downsampled = render_downsampled_blur_chain(
                        &source_group,
                        offsets,
                        blur_size,
                        bounds,
                        "flash_blur",
                        prepared,
                        gpu,
                        device,
                        pool,
                        context,
                        draw_calls,
                    );
                    child.replace_with_single(downsampled);
                } else {
                    let pipeline = prepared.pipelines.get(FilterPipeline::Blur);
                    for blur_offset in offsets {
                        let (source, destination) = child.post_process_write_textures();
                        let source_group =
                            source.filter_bind_group(device, &gpu.texture_layout, &gpu.sampler);
                        render_single_texture_filter_pass(
                            source_group,
                            destination,
                            *blur_offset,
                            "flash_blur",
                            prepared,
                            &pipeline,
                            context,
                        );
                        *draw_calls += 1;
                    }
                }
            }
            PreparedFilter::Glow {
                blur_offsets,
                blur_size,
                glow_offset,
            } => {
                let glow_pipeline = prepared.pipelines.get(FilterPipeline::Glow);
                render_blur_and_two_texture_filter(
                    &mut child,
                    bounds,
                    blur_offsets,
                    *blur_size,
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
                blur_size,
                bevel_offset,
            } => {
                let bevel_pipeline = prepared.pipelines.get(FilterPipeline::Bevel);
                render_blur_and_two_texture_filter(
                    &mut child,
                    bounds,
                    blur_offsets,
                    *blur_size,
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
                let pipeline = prepared.pipelines.get(FilterPipeline::ColorMatrix);
                let (source, destination) = child.post_process_write_textures();
                let source_group =
                    source.filter_bind_group(device, &gpu.texture_layout, &gpu.sampler);
                render_single_texture_filter_pass(
                    source_group,
                    destination,
                    *offset,
                    "flash_color_matrix",
                    prepared,
                    &pipeline,
                    context,
                );
                *draw_calls += 1;
            }
            PreparedFilter::Convolution { offset, kernel } => {
                let pipeline = prepared.pipelines.get(FilterPipeline::Convolution);
                let (source, destination) = child.post_process_write_textures();
                let source_group =
                    source.filter_bind_group(device, &gpu.texture_layout, &gpu.sampler);
                render_convolution_pass(
                    source_group,
                    destination,
                    *offset,
                    kernel,
                    prepared,
                    &pipeline,
                    context,
                );
                *draw_calls += 1;
            }
            PreparedFilter::Gradient {
                blur_offsets,
                blur_size,
                effect_offset,
                ramp,
            } => {
                let pipeline = prepared.pipelines.get(FilterPipeline::Gradient);
                render_blur_and_two_texture_filter(
                    &mut child,
                    bounds,
                    blur_offsets,
                    *blur_size,
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
    child
}

#[allow(clippy::too_many_arguments)]
fn render_ops(
    ops: &[PreparedOp],
    target: &super::texture_cache::TextureTarget,
    initial_clear: Option<Srgba>,
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
    let mut pending_clear = initial_clear;
    if ops.is_empty() {
        if let Some(clear) = pending_clear.take() {
            clear_target(target, clear, context);
        }
        return;
    }
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
            render_ops(
                mask,
                &mask_target,
                Some(Srgba::NONE),
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
            render_ops(
                content,
                &content_target,
                Some(Srgba::NONE),
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
            let content_group =
                content_target.main_filter_bind_group(device, &gpu.texture_layout, &gpu.sampler);
            let mask_group =
                mask_target.main_filter_bind_group(device, &gpu.texture_layout, &gpu.sampler);
            let pipeline = prepared.pipelines.get(FilterPipeline::Mask);
            let attachment = [Some(match pending_clear.take() {
                Some(clear) => target.get_color_attachment(clear),
                None => target.load_color_attachment(),
            })];
            let mut pass = context.begin_tracked_render_pass(RenderPassDescriptor {
                label: Some("flash_alpha_mask"),
                color_attachments: &attachment,
                ..Default::default()
            });
            pass.set_render_pipeline(&pipeline);
            pass.set_bind_group(0, &prepared.uniforms, &[*offset]);
            pass.set_bind_group(1, content_group, &[]);
            pass.set_bind_group(2, mask_group, &[]);
            pass.draw(0..6, 0..1);
            *draw_calls += 1;
            index += 1;
        } else if let PreparedOp::Layer {
            ops: children,
            bounds,
            offset,
            filters,
            needs_full_res_ping_pong,
            is_complex_blend,
            composite,
        } = &ops[index]
        {
            let child = render_layer_into_target(
                children,
                *bounds,
                filters,
                *needs_full_res_ping_pong,
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
            let texture = child.main_filter_bind_group(device, &gpu.texture_layout, &gpu.sampler);
            if *is_complex_blend {
                if let Some(clear) = pending_clear.take() {
                    clear_target(target, clear, context);
                }
                let dst_size = target.main_size();
                let dst_copy = pool.acquire(
                    device,
                    descriptor(dst_size.width, dst_size.height),
                    TextureAllocationSite {
                        purpose: TexturePurpose::BlendDestination,
                        bounds: [0.0, 0.0, dst_size.width as f32, dst_size.height as f32],
                    },
                );
                context.command_encoder().copy_texture_to_texture(
                    target.main_texture().as_image_copy(),
                    dst_copy.cached.texture.as_image_copy(),
                    dst_size,
                );
                let dst_group =
                    dst_copy.filter_bind_group(device, &gpu.texture_layout, &gpu.sampler);
                let attachment = [Some(target.load_color_attachment())];
                let mut pass = context.begin_tracked_render_pass(RenderPassDescriptor {
                    label: Some("flash_complex_blend"),
                    color_attachments: &attachment,
                    ..Default::default()
                });
                pass.set_render_pipeline(composite);
                pass.set_bind_group(0, &prepared.uniforms, &[*offset]);
                pass.set_bind_group(1, texture, &[]);
                pass.set_bind_group(2, dst_group, &[]);
                pass.draw(0..6, 0..1);
                *draw_calls += 1;
            } else {
                let attachment = [Some(match pending_clear.take() {
                    Some(clear) => target.get_color_attachment(clear),
                    None => target.load_color_attachment(),
                })];
                let mut pass = context.begin_tracked_render_pass(RenderPassDescriptor {
                    label: Some("flash_layer_composite"),
                    color_attachments: &attachment,
                    ..Default::default()
                });
                pass.set_render_pipeline(composite);
                pass.set_bind_group(0, &prepared.uniforms, &[*offset]);
                pass.set_bind_group(1, texture, &[]);
                pass.draw(0..6, 0..1);
                *draw_calls += 1;
            }
            // Pass ends before child/dst_copy leases are returned; later siblings can reuse them.
            index += 1;
        } else {
            let attachment = [Some(match pending_clear.take() {
                Some(clear) => target.get_color_attachment(clear),
                None => target.load_color_attachment(),
            })];
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
    // Fast path: when a top-level isolated packet is a single Layer matching the
    // root bounds with transparent clear, render it directly without allocating a
    // redundant root surface or performing an identity composite copy pass.
    if clear == Srgba::NONE
        && let [
            PreparedOp::Layer {
                ops: children,
                bounds,
                filters,
                needs_full_res_ping_pong,
                is_complex_blend,
                ..
            },
        ] = prepared.ops.as_slice()
        && !*is_complex_blend
        && *bounds == prepared.bounds
    {
        return render_layer_into_target(
            children,
            *bounds,
            filters,
            *needs_full_res_ping_pong,
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
    }
    let mut surface = pool.get(
        device,
        descriptor(prepared.bounds[2] as u32, prepared.bounds[3] as u32),
        msaa,
        TextureAllocationSite {
            purpose: TexturePurpose::Root,
            bounds: prepared.bounds,
        },
    );
    render_ops(
        &prepared.ops,
        &surface,
        Some(clear),
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
    let texture = surface.main_filter_bind_group(&device, &gpu.texture_layout, &gpu.sampler);
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
        pass.set_bind_group(1, texture, &[]);
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
    fn downsampled_blur_respects_thresholds_and_axes() {
        assert_eq!(
            downsampled_blur_size(
                [128.0, 128.0],
                9 * 65536,
                9 * 65536,
                VabFilterQuality::full()
            ),
            None
        );
        assert_eq!(
            downsampled_blur_size(
                [128.0, 128.0],
                9 * 65536,
                9 * 65536,
                VabFilterQuality::half_blur()
            ),
            Some([64, 64])
        );
        // 1D horizontal blur only downsamples the blurred axis.
        assert_eq!(
            downsampled_blur_size(
                [128.0, 128.0],
                9 * 65536,
                65536,
                VabFilterQuality::half_blur()
            ),
            Some([64, 128])
        );
        // Small targets stay full resolution.
        assert_eq!(
            downsampled_blur_size(
                [16.0, 16.0],
                9 * 65536,
                9 * 65536,
                VabFilterQuality::half_blur()
            ),
            None
        );
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
