use bevy::platform::time::Instant;
use std::{
    collections::{HashSet, VecDeque},
    sync::{Arc, Mutex, atomic::Ordering},
};
use std::{hash::Hash, ops::Range};

use bevy::{
    asset::{AssetEvent, AssetEventSystems, AssetId, embedded_asset, load_embedded_asset},
    camera::CompositingSpace,
    core_pipeline::core_2d::{CORE_2D_DEPTH_FORMAT, Transparent2d},
    diagnostic::FrameCount,
    ecs::system::{SystemParamItem, lifetimeless::SRes},
    math::{Affine3A, FloatOrd, Mat2, Mat4, Vec4},
    mesh::{Mesh, MeshVertexBufferLayoutRef},
    platform::collections::HashMap,
    prelude::*,
    render::{
        Extract, ExtractSchedule, Render, RenderApp, RenderStartup, RenderSystems,
        camera::ExtractedCamera,
        diagnostic::RecordDiagnostics,
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        mesh::{RenderMesh, RenderMeshBufferInfo, allocator::MeshAllocator},
        render_asset::RenderAssets,
        render_phase::{
            AddRenderCommand, DrawFunctions, PhaseItem, PhaseItemExtraIndex, RenderCommand,
            RenderCommandResult, TrackedRenderPass, ViewSortedRenderPhases,
        },
        render_resource::*,
        renderer::{RenderContext, RenderDevice, RenderQueue},
        sync_world::RenderEntity,
        texture::GpuImage,
        view::{ExtractedView, Msaa, RenderVisibleEntities, RetainedViewEntity},
    },
    sprite_render::{
        Mesh2dPipeline, Mesh2dPipelineKey, SetMesh2dViewBindGroup, init_mesh_2d_pipeline,
    },
};
use bytemuck::{Pod, Zeroable};

use crate::{
    material::{BitmapMaterial, GradientMaterial},
    render::{
        FlashRenderDiagnostics,
        extract::{Op, op_bounds},
        gpu,
        texture_cache::TextureAllocationOwner,
    },
    sampling::VabSkin,
    vab_asset::{VabAsset, VabAssetHandle},
    vab_player::VabPlayer,
};

const VIEW_MATRIX: Mat4 = Mat4::from_cols_array(&[
    1.0, 0.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
]);

pub(super) struct VabInstanceRenderPlugin;

/// Number of reused diagnostic paths for matching asynchronous GPU timings to
/// the render frame that submitted them.
pub const VAB_FILTER_DIAGNOSTIC_SLOTS: u64 = 256;

/// Latest completed scene-filter workload, readable from the main world.
/// Installed only by [`super::VabDiagnosticsPlugin`].
/// The render world publishes it after pre-rendering, so a consumer may see
/// the previous render frame while the current one is in flight.
#[derive(Clone, Copy, Debug, Default)]
pub struct VabFilterWorkloadSample {
    pub frame: u64,
    /// Main-world frame count carried into this render frame by Bevy extraction.
    pub app_frame: u32,
    pub layers: usize,
    pub cache_hits: usize,
    pub cache_misses: usize,
    /// Actual Blur/Glow/ColorMatrix/etc. shader passes, excluding cached layers.
    pub passes: u64,
    /// Sum of render-target pixels across those shader passes.
    pub processed_pixels: u64,
    pub extract_cpu_ns: u64,
    pub queue_cpu_ns: u64,
    pub prepare_buffers_cpu_ns: u64,
    pub prepare_bind_groups_cpu_ns: u64,
    pub prepass_cpu_ns: u64,
    pub filter_cache_bytes: u64,
    /// Newly created transient GPU textures during this render frame.
    pub texture_allocations: u64,
    pub texture_reuses: u64,
    pub texture_first_in_bucket_allocations: u64,
    pub texture_exhausted_bucket_allocations: u64,
    pub texture_allocated_bytes: u64,
    pub pooled_texture_bytes: u64,
    pub peak_live_transient_bytes: u64,
    /// Resident pool payload after render cleanup and idle-budget trimming.
    pub pool_end_bytes: Option<u64>,
    pub pool_end_live_bytes: Option<u64>,
}

#[derive(Resource, Clone, Default)]
pub struct VabFilterWorkload(Arc<Mutex<VabFilterWorkloadState>>);

#[derive(Default)]
struct VabFilterWorkloadState {
    latest: VabFilterWorkloadSample,
    recent: VecDeque<(VabFilterWorkloadSample, Instant)>,
    pending: Option<VabFilterWorkloadSample>,
}

impl VabFilterWorkload {
    pub fn latest(&self) -> VabFilterWorkloadSample {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .latest
    }

    /// Recent render frames and their publication times, for pairing with
    /// asynchronously delivered Bevy GPU diagnostics.
    pub fn recent_since(&self, frame: u64) -> Vec<(VabFilterWorkloadSample, Instant)> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .recent
            .iter()
            .filter(|(sample, _)| sample.frame > frame)
            .copied()
            .collect()
    }

    fn stage(&self, sample: VabFilterWorkloadSample) {
        let mut state = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.pending = Some(sample);
    }

    fn record_pool_end(&self, total: u64, live: u64) {
        let mut state = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(mut sample) = state.pending.take() {
            sample.pool_end_bytes = Some(total);
            sample.pool_end_live_bytes = Some(live);
            state.latest = sample;
            state.recent.push_back((sample, Instant::now()));
            if state.recent.len() > VAB_FILTER_DIAGNOSTIC_SLOTS as usize {
                state.recent.pop_front();
            }
        }
    }
}

impl Plugin for VabInstanceRenderPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/vab_instance.wgsl");
        app.init_resource::<VabAssetRevisions>()
            .init_resource::<VabMaterialRevision>()
            .init_resource::<VabGpuSourceRevision>()
            .init_resource::<VabFilterCacheSettings>()
            .init_resource::<VabFilterMsaa>()
            .add_systems(
                PostUpdate,
                (
                    track_vab_asset_revisions,
                    track_vab_material_revision,
                    track_vab_gpu_source_revision,
                )
                    .after(AssetEventSystems),
            )
            .add_plugins((
                bevy::render::extract_component::ExtractComponentPlugin::<VabAssetHandle>::default(
                ),
                ExtractResourcePlugin::<VabFilterCacheSettings>::default(),
                ExtractResourcePlugin::<VabFilterMsaa>::default(),
            ));
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<ExtractedVabInstances>()
            .init_resource::<VabSampleCache>()
            .init_resource::<VabFilterOutputCache>()
            .init_resource::<VabInstanceResources>()
            .init_resource::<VabInstanceBuffers>()
            .init_resource::<VabFilterUniformBuffers>()
            .init_resource::<PreparedVabInstances>()
            .init_resource::<VabTextureBindGroups>()
            .init_resource::<VabQuadPipelines>()
            .init_resource::<SpecializedMeshPipelines<VabInstancePipeline>>()
            .add_render_command::<Transparent2d, DrawVabInstance>()
            .add_systems(
                RenderStartup,
                init_vab_instance_pipeline.after(init_mesh_2d_pipeline),
            )
            .add_systems(ExtractSchedule, extract_vab_instances)
            .add_systems(
                Render,
                (
                    queue_vab_instance_data.in_set(RenderSystems::QueueMeshes),
                    queue_vab_instances
                        .in_set(RenderSystems::QueueMeshes)
                        .after(queue_vab_instance_data),
                    prepare_vab_instance_buffers.in_set(RenderSystems::PrepareResources),
                    prepare_vab_instance_bind_groups.in_set(RenderSystems::PrepareBindGroups),
                    clear_prepared_vab_instances
                        .in_set(RenderSystems::Cleanup)
                        .after(RenderSystems::Render),
                    trim_idle_texture_pool
                        .in_set(RenderSystems::Cleanup)
                        .after(clear_prepared_vab_instances),
                ),
            );
    }
}

#[derive(Resource, Default)]
struct VabAssetRevisions(HashMap<AssetId<VabAsset>, u64>);

#[derive(Resource, Default)]
struct VabMaterialRevision(u64);

#[derive(Resource, Default)]
struct VabGpuSourceRevision(u64);

fn track_vab_asset_revisions(
    mut events: MessageReader<AssetEvent<VabAsset>>,
    mut revisions: ResMut<VabAssetRevisions>,
) {
    for event in events.read() {
        let id = match event {
            AssetEvent::Added { id }
            | AssetEvent::Modified { id }
            | AssetEvent::Removed { id }
            | AssetEvent::Unused { id }
            | AssetEvent::LoadedWithDependencies { id } => *id,
        };
        let revision = revisions.0.entry(id).or_default();
        *revision = revision.wrapping_add(1);
    }
}

fn track_vab_material_revision(
    mut bitmaps: MessageReader<AssetEvent<BitmapMaterial>>,
    mut gradients: MessageReader<AssetEvent<GradientMaterial>>,
    mut revision: ResMut<VabMaterialRevision>,
) {
    let bitmap_changed = bitmaps.read().next().is_some();
    let gradient_changed = gradients.read().next().is_some();
    if bitmap_changed || gradient_changed {
        revision.0 = revision.0.wrapping_add(1);
    }
}

fn track_vab_gpu_source_revision(
    mut images: Option<MessageReader<AssetEvent<Image>>>,
    mut meshes: Option<MessageReader<AssetEvent<Mesh>>>,
    mut revision: ResMut<VabGpuSourceRevision>,
) {
    let image_changed = images
        .as_mut()
        .is_some_and(|events| events.read().next().is_some());
    let mesh_changed = meshes
        .as_mut()
        .is_some_and(|events| events.read().next().is_some());
    if image_changed || mesh_changed {
        revision.0 = revision.0.wrapping_add(1);
    }
}

#[derive(Clone, PartialEq, Eq)]
struct VabSampleKey {
    asset: AssetId<VabAsset>,
    asset_revision: u64,
    material_revision: u64,
    clip: usize,
    frame: usize,
    skin: VabSkin,
}

struct CachedVabSample {
    key: VabSampleKey,
    ops: Arc<[Op]>,
    generation: u64,
}

#[derive(Resource, Default)]
struct VabSampleCache {
    entries: HashMap<Entity, CachedVabSample>,
    next_generation: u64,
}

/// Bounds persistent filtered outputs without retaining every animation frame.
#[derive(Resource, ExtractResource, Clone)]
pub struct VabFilterCacheSettings {
    /// Hard limit for persistent single-sampled filter outputs. The transient
    /// textures used while producing a cache miss are accounted separately.
    pub max_bytes: u64,
    /// Evict an output after this many render frames without a cache hit.
    pub max_unused_frames: u64,
}

/// Sample count for VAB filter isolation targets in the scene renderer.
/// The final `Transparent2d` draw always uses the camera's own MSAA setting.
#[derive(Resource, ExtractResource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VabFilterMsaa {
    /// Match each camera's MSAA setting, preserving the existing edge quality.
    #[default]
    FollowCamera,
    Sample2,
    Sample4,
    /// Draw filtered vector geometry without MSAA to reduce transient memory.
    Off,
}

impl VabFilterMsaa {
    fn resolve(self, camera: Msaa) -> Msaa {
        match self {
            Self::FollowCamera => camera,
            Self::Sample2 => Msaa::Sample2,
            Self::Sample4 => Msaa::Sample4,
            Self::Off => Msaa::Off,
        }
    }
}

impl Default for VabFilterCacheSettings {
    fn default() -> Self {
        Self {
            max_bytes: 64 * 1024 * 1024,
            max_unused_frames: 8,
        }
    }
}

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
struct FilterCacheSlot {
    view: RetainedViewEntity,
    entity: Entity,
    packet: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct FilterCacheContent {
    sample_generation: u64,
    gpu_source_revision: u64,
    scale: [u32; 2],
    samples: u32,
    size: [u32; 2],
    origin: [u32; 2],
}

struct CachedFilterOutput {
    content: FilterCacheContent,
    _texture: super::texture_cache::PersistentTexture,
    bind_group: BindGroup,
    bytes: u64,
    last_used_frame: u64,
}

#[derive(Resource, Default)]
pub(super) struct VabFilterOutputCache {
    entries: HashMap<FilterCacheSlot, CachedFilterOutput>,
    frame: u64,
    resident_bytes: u64,
}

impl VabFilterOutputCache {
    fn begin_frame(&mut self, settings: &VabFilterCacheSettings) {
        self.frame = self.frame.wrapping_add(1);
        let frame = self.frame;
        self.retain(|entry| {
            frame.wrapping_sub(entry.last_used_frame) <= settings.max_unused_frames
        });
        self.trim_to(settings.max_bytes, 0);
    }

    fn get(&mut self, slot: FilterCacheSlot, content: FilterCacheContent) -> Option<BindGroup> {
        if self
            .entries
            .get(&slot)
            .is_some_and(|entry| entry.content != content)
        {
            self.remove(slot);
        }
        let entry = self.entries.get_mut(&slot)?;
        entry.last_used_frame = self.frame;
        Some(entry.bind_group.clone())
    }

    fn insert(
        &mut self,
        slot: FilterCacheSlot,
        content: FilterCacheContent,
        texture: super::texture_cache::PersistentTexture,
        bind_group: BindGroup,
        bytes: u64,
    ) {
        self.resident_bytes = self.resident_bytes.saturating_add(bytes);
        self.entries.insert(
            slot,
            CachedFilterOutput {
                content,
                _texture: texture,
                bind_group,
                bytes,
                last_used_frame: self.frame,
            },
        );
    }

    fn reserve(&mut self, slot: FilterCacheSlot, bytes: u64, max_bytes: u64) -> bool {
        if bytes > max_bytes {
            return false;
        }
        self.remove(slot);
        self.trim_to(max_bytes, bytes);
        self.resident_bytes.saturating_add(bytes) <= max_bytes
    }

    fn trim_to(&mut self, max_bytes: u64, incoming: u64) {
        while self.resident_bytes.saturating_add(incoming) > max_bytes {
            let Some((&slot, _)) = self
                .entries
                .iter()
                .filter(|(_, entry)| entry.last_used_frame != self.frame)
                .min_by_key(|(_, entry)| entry.last_used_frame)
            else {
                break;
            };
            self.remove(slot);
        }
    }

    fn remove(&mut self, slot: FilterCacheSlot) {
        if let Some(entry) = self.entries.remove(&slot) {
            self.resident_bytes = self.resident_bytes.saturating_sub(entry.bytes);
        }
    }

    fn retain(&mut self, mut keep: impl FnMut(&CachedFilterOutput) -> bool) {
        self.entries.retain(|_, entry| {
            let retain = keep(entry);
            if !retain {
                self.resident_bytes = self.resident_bytes.saturating_sub(entry.bytes);
            }
            retain
        });
    }
}

#[derive(Default)]
struct ExtractedVabInstance {
    ops: Arc<[Op]>,
    global: Affine3A,
    unsupported_layers: usize,
    sample_generation: u64,
    gpu_source_revision: u64,
}

#[derive(Resource, Default)]
struct ExtractedVabInstances(HashMap<Entity, ExtractedVabInstance>);

struct VabInstanceResource {
    ops: Arc<[Op]>,
    scaled_ops: Vec<Op>,
    visible_bounds: Option<[f32; 4]>,
    global: Affine3A,
    scale: Vec2,
    view_msaa: Msaa,
    filter_msaa: Msaa,
    target_format: TextureFormat,
    srgb_compositing: bool,
    sample_generation: u64,
    gpu_source_revision: u64,
}

#[derive(Resource, Default)]
struct VabInstanceResources {
    instances: HashMap<(RetainedViewEntity, Entity), VabInstanceResource>,
}

#[derive(Resource)]
struct VabInstanceBuffers {
    indices: HashMap<(RetainedViewEntity, Entity, usize), u32>,
    values: RawBufferVec<VabInstanceUniform>,
    bind_group: Option<BindGroup>,
}

impl Default for VabInstanceBuffers {
    fn default() -> Self {
        let mut values = RawBufferVec::new(BufferUsages::STORAGE);
        values.set_label(Some("vab instance storage"));
        Self {
            indices: default(),
            values,
            bind_group: None,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct VabInstanceUniform {
    world_from_local: [[f32; 4]; 4],
    uv: [[f32; 4]; 4],
    multiply: [f32; 4],
    add: [f32; 4],
    material: [f32; 4],
}

struct PreparedDrawPacket {
    mesh: AssetId<Mesh>,
    texture_id: Option<AssetId<Image>>,
    texture: BindGroup,
    instances: Range<u32>,
    pipeline: Option<CachedRenderPipelineId>,
}

struct PreparedLayerPacket {
    layer: Option<gpu::PreparedLayer>,
    target: Option<super::texture_cache::TextureTarget>,
    texture: Option<BindGroup>,
    cache_slot: FilterCacheSlot,
    cache_content: FilterCacheContent,
    pixel_bounds: [f32; 4],
    instances: Range<u32>,
    pipeline: Option<CachedRenderPipelineId>,
}

enum PreparedVabPacket {
    Draw(PreparedDrawPacket),
    Layer(Box<PreparedLayerPacket>),
}

struct PreparedVabInstance {
    packets: Vec<PreparedVabPacket>,
    filter_msaa: Msaa,
}

#[derive(Resource, Default)]
pub(super) struct PreparedVabInstances {
    instances: HashMap<(RetainedViewEntity, Entity), PreparedVabInstance>,
    draw_bind_group: Option<BindGroup>,
}

struct CachedTextureBindGroup {
    texture_view: TextureViewId,
    sampler: SamplerId,
    bind_group: BindGroup,
}

#[derive(Resource, Default)]
struct VabTextureBindGroups(HashMap<AssetId<Image>, CachedTextureBindGroup>);

#[derive(Resource, Default)]
struct VabFilterUniformBuffers(HashMap<FilterCacheSlot, gpu::ReusableUniformBuffer>);

#[derive(Resource, Default)]
struct VabQuadPipelines(
    HashMap<(TextureFormat, u32, crate::vab_asset::VabBlendMode, bool), CachedRenderPipelineId>,
);

#[derive(Resource, Clone)]
pub(super) struct VabInstancePipeline {
    view_layout: BindGroupLayoutDescriptor,
    draw_layout: BindGroupLayoutDescriptor,
    texture_layout: BindGroupLayoutDescriptor,
    shader: Handle<Shader>,
    fallback_texture: BindGroup,
    sampler: Sampler,
}

fn init_vab_instance_pipeline(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    pipeline_cache: Res<PipelineCache>,
    mesh_pipeline: Res<Mesh2dPipeline>,
    asset_server: Res<AssetServer>,
) {
    let draw_layout = BindGroupLayoutDescriptor::new(
        "vab_instance_draw_layout",
        &[BindGroupLayoutEntry {
            binding: 0,
            visibility: ShaderStages::VERTEX_FRAGMENT,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    );
    let texture_layout = BindGroupLayoutDescriptor::new(
        "vab_instance_texture_layout",
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
    let texture = render_device.create_texture_with_data(
        &render_queue,
        &TextureDescriptor {
            label: Some("vab white texture"),
            size: Extent3d::default(),
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
    let sampler = render_device.create_sampler(&SamplerDescriptor::default());
    let fallback_texture = render_device.create_bind_group(
        "vab white texture bind group",
        &pipeline_cache.get_bind_group_layout(&texture_layout),
        &[
            BindGroupEntry {
                binding: 0,
                resource: BindingResource::TextureView(&texture.create_view(&default())),
            },
            BindGroupEntry {
                binding: 1,
                resource: BindingResource::Sampler(&sampler),
            },
        ],
    );
    commands.insert_resource(VabInstancePipeline {
        view_layout: mesh_pipeline.view_layout.clone(),
        draw_layout,
        texture_layout,
        shader: load_embedded_asset!(asset_server.as_ref(), "shaders/vab_instance.wgsl"),
        fallback_texture,
        sampler,
    });
}

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub(super) struct VabInstancePipelineKey(Mesh2dPipelineKey);

impl SpecializedMeshPipeline for VabInstancePipeline {
    type Key = VabInstancePipelineKey;

    fn specialize(
        &self,
        key: Self::Key,
        layout: &MeshVertexBufferLayoutRef,
    ) -> Result<RenderPipelineDescriptor, SpecializedMeshPipelineError> {
        let vertex_layout = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_COLOR.at_shader_location(1),
        ])?;
        let shader_defs = if key.0.contains(Mesh2dPipelineKey::SRGB_COMPOSITING) {
            vec!["SRGB_COMPOSITING".into()]
        } else {
            Vec::new()
        };
        Ok(RenderPipelineDescriptor {
            label: Some("vab instance pipeline".into()),
            layout: vec![
                self.view_layout.clone(),
                self.draw_layout.clone(),
                self.texture_layout.clone(),
            ],
            vertex: VertexState {
                shader: self.shader.clone(),
                entry_point: Some("vertex".into()),
                buffers: vec![vertex_layout],
                ..default()
            },
            fragment: Some(FragmentState {
                shader: self.shader.clone(),
                shader_defs,
                targets: vec![Some(ColorTargetState {
                    format: key.0.target_format(),
                    blend: Some(BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: ColorWrites::ALL,
                })],
                ..default()
            }),
            primitive: PrimitiveState {
                topology: key.0.primitive_topology(),
                cull_mode: None,
                ..default()
            },
            depth_stencil: Some(DepthStencilState {
                format: CORE_2D_DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(CompareFunction::GreaterEqual),
                stencil: StencilState::default(),
                bias: DepthBiasState::default(),
            }),
            multisample: MultisampleState {
                count: key.0.msaa_samples(),
                ..default()
            },
            ..default()
        })
    }
}

impl VabInstancePipeline {
    fn quad_descriptor(
        &self,
        format: TextureFormat,
        samples: u32,
        blend: crate::vab_asset::VabBlendMode,
        srgb_compositing: bool,
    ) -> RenderPipelineDescriptor {
        RenderPipelineDescriptor {
            label: Some("vab filtered layer pipeline".into()),
            layout: vec![
                self.view_layout.clone(),
                self.draw_layout.clone(),
                self.texture_layout.clone(),
            ],
            vertex: VertexState {
                shader: self.shader.clone(),
                entry_point: Some("vertex_quad".into()),
                buffers: vec![],
                ..default()
            },
            fragment: Some(FragmentState {
                shader: self.shader.clone(),
                shader_defs: if srgb_compositing {
                    vec!["SRGB_COMPOSITING".into()]
                } else {
                    Vec::new()
                },
                targets: vec![Some(ColorTargetState {
                    format,
                    blend: Some(gpu::fixed_blend_state(blend)),
                    write_mask: ColorWrites::ALL,
                })],
                ..default()
            }),
            primitive: PrimitiveState {
                topology: PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..default()
            },
            depth_stencil: Some(DepthStencilState {
                format: CORE_2D_DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(CompareFunction::GreaterEqual),
                stencil: StencilState::default(),
                bias: DepthBiasState::default(),
            }),
            multisample: MultisampleState {
                count: samples,
                ..default()
            },
            ..default()
        }
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn extract_vab_instances(
    query: Extract<
        Query<
            (
                Entity,
                RenderEntity,
                &GlobalTransform,
                &VabAssetHandle,
                &VabPlayer,
                Option<&VabSkin>,
                &ViewVisibility,
            ),
            Without<super::RenderOffscreenTexture>,
        >,
    >,
    assets: Extract<Res<Assets<VabAsset>>>,
    bitmaps: Extract<Res<Assets<crate::material::BitmapMaterial>>>,
    gradients: Extract<Res<Assets<crate::material::GradientMaterial>>>,
    revisions: Extract<Res<VabAssetRevisions>>,
    material_revision: Extract<Res<VabMaterialRevision>>,
    gpu_source_revision: Extract<Res<VabGpuSourceRevision>>,
    mut extracted: ResMut<ExtractedVabInstances>,
    mut sample_cache: ResMut<VabSampleCache>,
    mut diagnostics: ResMut<FlashRenderDiagnostics>,
) {
    let started = diagnostics.enabled.then(Instant::now);
    extracted.0.clear();
    diagnostics.vab_instances_extracted = 0;
    diagnostics.unsupported_vab_layers = 0;
    diagnostics.vab_mask_layers = 0;
    diagnostics.vab_sample_cache_hits = 0;
    diagnostics.vab_sample_cache_misses = 0;
    let default_skin = VabSkin::default();
    let mut live = HashSet::<Entity>::new();
    for (_, render_entity, global, handle, player, skin, visibility) in &query {
        if !visibility.get() {
            continue;
        }
        live.insert(render_entity);
        let Some(asset) = assets.get(&handle.0) else {
            continue;
        };
        let skin = skin.unwrap_or(&default_skin);
        let asset_id = handle.0.id();
        let key = VabSampleKey {
            asset: asset_id,
            asset_revision: revisions.0.get(&asset_id).copied().unwrap_or(0),
            material_revision: material_revision.0,
            clip: player.clip(),
            frame: player.current_frame,
            skin: skin.clone(),
        };
        let (ops, sample_generation) = if let Some(cached) =
            sample_cache.entries.get(&render_entity)
            && cached.key == key
        {
            diagnostics.vab_sample_cache_hits += 1;
            (cached.ops.clone(), cached.generation)
        } else {
            let Ok(commands) = asset.sample(player.clip(), player.current_frame, skin, Vec3::ONE)
            else {
                sample_cache.entries.remove(&render_entity);
                continue;
            };
            let Ok(ops) = super::extract::resolve(asset, &commands, &bitmaps, &gradients) else {
                sample_cache.entries.remove(&render_entity);
                continue;
            };
            diagnostics.vab_sample_cache_misses += 1;
            let ops: Arc<[Op]> = ops.into();
            sample_cache.next_generation = sample_cache.next_generation.wrapping_add(1).max(1);
            let generation = sample_cache.next_generation;
            sample_cache.entries.insert(
                render_entity,
                CachedVabSample {
                    key,
                    ops: ops.clone(),
                    generation,
                },
            );
            (ops, generation)
        };
        if !ops.is_empty() {
            let unsupported_layers = if diagnostics.enabled {
                count_unsupported_layers(&ops)
            } else {
                0
            };
            if diagnostics.enabled {
                diagnostics.vab_mask_layers += count_masks(&ops);
            }
            let instance = ExtractedVabInstance {
                ops,
                global: global.affine(),
                unsupported_layers,
                sample_generation,
                gpu_source_revision: gpu_source_revision.0,
            };
            diagnostics.vab_instances_extracted += 1;
            diagnostics.unsupported_vab_layers += instance.unsupported_layers;
            extracted.0.insert(render_entity, instance);
        }
    }
    sample_cache
        .entries
        .retain(|entity, _| live.contains(entity));
    diagnostics.vab_extract_cpu_ns = elapsed_ns(started);
}

fn elapsed_ns(started: Option<Instant>) -> u64 {
    started.map_or(0, |started| {
        u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX)
    })
}

fn count_masks(ops: &[Op]) -> usize {
    ops.iter()
        .map(|op| match op {
            Op::Draw(_) => 0,
            Op::Mask { mask, content, .. } => 1 + count_masks(mask) + count_masks(content),
            Op::Layer { ops, .. } => count_masks(ops),
        })
        .sum()
}

fn count_unsupported_layers(ops: &[Op]) -> usize {
    ops.iter()
        .map(|op| match op {
            Op::Draw(_) => 0,
            Op::Mask { mask, content, .. } => {
                count_unsupported_layers(mask) + count_unsupported_layers(content)
            }
            Op::Layer {
                ops,
                filters,
                blend,
                ..
            } => {
                usize::from(filters.iter().any(|filter| {
                    !matches!(
                        filter,
                        vatf::animation::AnimFilter::BlurFilter(_)
                            | vatf::animation::AnimFilter::GlowFilter(_)
                            | vatf::animation::AnimFilter::ColorMatrixFilter(_)
                            | vatf::animation::AnimFilter::DropShadowFilter(_)
                            | vatf::animation::AnimFilter::BevelFilter(_)
                            | vatf::animation::AnimFilter::ConvolutionFilter(_)
                            | vatf::animation::AnimFilter::GradientGlowFilter(_)
                            | vatf::animation::AnimFilter::GradientBevelFilter(_)
                    )
                })) + usize::from(!matches!(
                    blend,
                    crate::vab_asset::VabBlendMode::Normal
                        | crate::vab_asset::VabBlendMode::Layer
                        | crate::vab_asset::VabBlendMode::Add
                        | crate::vab_asset::VabBlendMode::Subtract
                        | crate::vab_asset::VabBlendMode::Screen
                        | crate::vab_asset::VabBlendMode::Lighten
                )) + count_unsupported_layers(ops)
            }
        })
        .sum()
}

fn view_pixel_scale(view: &ExtractedView, global: Affine3A) -> Vec2 {
    let view_from_world = view.world_from_view.to_matrix().inverse();
    let clip_from_world = view
        .clip_from_world
        .unwrap_or(view.clip_from_view * view_from_world);
    let clip_from_local = clip_from_world * Mat4::from(global) * VIEW_MATRIX;
    let project = |point: Vec2| {
        let clip = clip_from_local * point.extend(0.0).extend(1.0);
        clip.xy() / clip.w
    };
    let origin = project(Vec2::ZERO);
    let half_viewport = view.viewport.zw().as_vec2() * 0.5;
    let x = ((project(Vec2::X) - origin) * half_viewport).length();
    let y = ((project(Vec2::Y) - origin) * half_viewport).length();
    Vec2::new(x, y).clamp(Vec2::splat(1.0 / 1024.0), Vec2::splat(1024.0))
}

/// Invert the animation plane's affine projection, rather than assuming an
/// axis-aligned entity or a camera at the origin. Perspective/degenerate planes
/// keep the full target. Bounds are in the same scaled Flash space as scale_ops.
fn visible_pixel_bounds(clip_from_local: Mat4, scale: Vec2) -> Option<[f32; 4]> {
    if clip_from_local.x_axis.w != 0.0 || clip_from_local.y_axis.w != 0.0 {
        return None;
    }
    let w = clip_from_local.w_axis.w;
    let origin = clip_from_local.w_axis.xy() / w;
    let axes = Mat2::from_cols(
        clip_from_local.x_axis.xy() / (w * scale.x),
        clip_from_local.y_axis.xy() / (w * scale.y),
    );
    if !axes.is_finite() || axes.determinant().abs() <= f32::MIN_POSITIVE {
        return None;
    }
    let inverse = axes.inverse();
    let mut min = Vec2::splat(f32::INFINITY);
    let mut max = Vec2::splat(f32::NEG_INFINITY);
    for corner in [
        Vec2::NEG_ONE,
        Vec2::new(-1.0, 1.0),
        Vec2::ONE,
        Vec2::new(1.0, -1.0),
    ] {
        let point = inverse * (corner - origin);
        min = min.min(point);
        max = max.max(point);
    }
    // Guard bilinear sampling and sample coverage at the final quad's edge.
    min = (min - Vec2::splat(2.0)).floor();
    max = (max + Vec2::splat(2.0)).ceil();
    (min.is_finite() && max.is_finite()).then_some([min.x, min.y, max.x - min.x, max.y - min.y])
}

fn packet_pixel_bounds(bounds: [f32; 4], visible: Option<[f32; 4]>) -> Option<[f32; 4]> {
    let mut bounds = bounds;
    if let Some(visible) = visible {
        let x = bounds[0].max(visible[0]);
        let y = bounds[1].max(visible[1]);
        let right = (bounds[0] + bounds[2]).min(visible[0] + visible[2]);
        let bottom = (bounds[1] + bounds[3]).min(visible[1] + visible[3]);
        bounds = [x, y, right - x, bottom - y];
    }
    if bounds[2] <= 0.0 || bounds[3] <= 0.0 {
        return None;
    }
    gpu::pixel_bounds(bounds).ok()
}

fn scale_ops(ops: &[Op], scale: Vec2) -> Vec<Op> {
    ops.iter()
        .map(|op| match op {
            Op::Draw(draw) => {
                let mut draw = draw.clone();
                let matrix = &mut draw.transform.matrix;
                matrix.a *= scale.x;
                matrix.c *= scale.x;
                matrix.tx *= scale.x;
                matrix.b *= scale.y;
                matrix.d *= scale.y;
                matrix.ty *= scale.y;
                Op::Draw(draw)
            }
            Op::Mask { mask, content, .. } => {
                let mask = scale_ops(mask, scale);
                let content = scale_ops(content, scale);
                let a = op_bounds(&mask);
                let b = op_bounds(&content);
                let x = a[0].max(b[0]);
                let y = a[1].max(b[1]);
                let right = (a[0] + a[2]).min(b[0] + b[2]);
                let bottom = (a[1] + a[3]).min(b[1] + b[3]);
                Op::Mask {
                    mask,
                    content,
                    bounds: [x, y, (right - x).max(0.0), (bottom - y).max(0.0)],
                }
            }
            Op::Layer {
                ops,
                filters,
                blend,
                ..
            } => {
                let ops = scale_ops(ops, scale);
                let mut filters = filters.clone();
                for filter in &mut filters {
                    filter.scale(scale.x, scale.y);
                }
                let [x, y, width, height] = op_bounds(&ops);
                let bounds = if filters.is_empty() {
                    [x, y, width, height]
                } else {
                    let (x, y, width, height) =
                        vatf::animation::filter_dest_rect(x, y, width, height, &filters);
                    [x, y, width, height]
                };
                Op::Layer {
                    ops,
                    bounds,
                    filters,
                    blend: *blend,
                }
            }
        })
        .collect()
}

fn queue_vab_instance_data(
    extracted: Res<ExtractedVabInstances>,
    mut resources: ResMut<VabInstanceResources>,
    views: Query<
        (
            &Msaa,
            &ExtractedView,
            &ExtractedCamera,
            &RenderVisibleEntities,
        ),
        With<Camera2d>,
    >,
    mut filter_cache: ResMut<VabFilterOutputCache>,
    filter_cache_settings: Res<VabFilterCacheSettings>,
    filter_msaa: Res<VabFilterMsaa>,
    mut diagnostics: ResMut<FlashRenderDiagnostics>,
) {
    let started = diagnostics.enabled.then(Instant::now);
    resources.instances.clear();
    diagnostics.vab_filter_cache_hits = 0;
    diagnostics.vab_filter_cache_misses = 0;
    filter_cache.begin_frame(&filter_cache_settings);
    diagnostics.vab_filter_cache_entries = filter_cache.entries.len();
    diagnostics.vab_filter_cache_bytes = filter_cache.resident_bytes;
    for (msaa, view, camera, visible) in &views {
        let Some(visible) = visible.get::<VabAssetHandle>() else {
            continue;
        };
        for (entity, _) in visible.iter_visible() {
            let Some(instance) = extracted.0.get(entity) else {
                continue;
            };
            let scale = view_pixel_scale(view, instance.global);
            let clip_from_world = view.clip_from_world.unwrap_or_else(|| {
                view.clip_from_view * view.world_from_view.to_matrix().inverse()
            });
            let visible_bounds = visible_pixel_bounds(
                clip_from_world * Mat4::from(instance.global) * VIEW_MATRIX,
                scale,
            );
            resources.instances.insert(
                (view.retained_view_entity, *entity),
                VabInstanceResource {
                    ops: instance.ops.clone(),
                    scaled_ops: scale_ops(&instance.ops, scale),
                    visible_bounds,
                    global: instance.global,
                    scale,
                    view_msaa: *msaa,
                    filter_msaa: filter_msaa.resolve(*msaa),
                    target_format: view.target_format,
                    srgb_compositing: camera.compositing_space == Some(CompositingSpace::Srgb),
                    sample_generation: instance.sample_generation,
                    gpu_source_revision: instance.gpu_source_revision,
                },
            );
        }
    }
    diagnostics.vab_queue_cpu_ns = elapsed_ns(started);
}

fn prepare_vab_instance_buffers(
    resources: Res<VabInstanceResources>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    mut buffers: ResMut<VabInstanceBuffers>,
    mut diagnostics: ResMut<FlashRenderDiagnostics>,
) {
    let started = diagnostics.enabled.then(Instant::now);
    buffers.indices.clear();
    buffers.values.clear();
    for ((view, entity), instance) in &resources.instances {
        for (packet_index, (op, scaled_op)) in
            instance.ops.iter().zip(&instance.scaled_ops).enumerate()
        {
            let uniform = match (op, scaled_op) {
                (Op::Draw(draw), _) => {
                    let local = draw.transform.matrix;
                    let local = Mat4::from_cols_array(&[
                        local.a, local.b, 0.0, 0.0, local.c, local.d, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0,
                        local.tx, local.ty, 0.0, 1.0,
                    ]);
                    let color = draw.transform.color_transform;
                    VabInstanceUniform {
                        world_from_local: (Mat4::from(instance.global) * VIEW_MATRIX * local)
                            .to_cols_array_2d(),
                        uv: draw.uv.to_cols_array_2d(),
                        multiply: [
                            color.r_multiply,
                            color.g_multiply,
                            color.b_multiply,
                            color.a_multiply,
                        ],
                        add: [color.r_add, color.g_add, color.b_add, color.a_add],
                        material: [
                            draw.gradient[0],
                            draw.gradient[1],
                            draw.gradient[2],
                            draw.kind as f32,
                        ],
                    }
                }
                (
                    Op::Layer { .. } | Op::Mask { .. },
                    Op::Layer { bounds, .. } | Op::Mask { bounds, .. },
                ) => {
                    let Some(pixel_bounds) = packet_pixel_bounds(*bounds, instance.visible_bounds)
                    else {
                        continue;
                    };
                    let logical_bounds = [
                        pixel_bounds[0] / instance.scale.x,
                        pixel_bounds[1] / instance.scale.y,
                        pixel_bounds[2] / instance.scale.x,
                        pixel_bounds[3] / instance.scale.y,
                    ];
                    let rect = Mat4::from_scale_rotation_translation(
                        Vec3::new(logical_bounds[2], logical_bounds[3], 1.0),
                        Quat::IDENTITY,
                        Vec3::new(logical_bounds[0], logical_bounds[1], 0.0),
                    );
                    VabInstanceUniform {
                        world_from_local: (Mat4::from(instance.global) * VIEW_MATRIX * rect)
                            .to_cols_array_2d(),
                        uv: Mat4::IDENTITY.to_cols_array_2d(),
                        multiply: Vec4::ONE.to_array(),
                        add: Vec4::ZERO.to_array(),
                        material: [0.0, 0.0, 0.0, 3.0],
                    }
                }
                _ => unreachable!("scaled instance operation changed kind"),
            };
            let index = u32::try_from(buffers.values.len()).expect("too many VAB draw instances");
            buffers
                .indices
                .insert((*view, *entity, packet_index), index);
            buffers.values.push(uniform);
        }
    }
    if !buffers.values.is_empty() {
        let required = buffers.values.len();
        if required > buffers.values.capacity() {
            buffers
                .values
                .reserve(required.next_power_of_two(), &render_device);
            buffers.bind_group = None;
            diagnostics.instance_buffer_allocations += 1;
        } else {
            diagnostics.instance_buffer_reuses += 1;
        }
        buffers.values.write_buffer(&render_device, &render_queue);
    }
    diagnostics.vab_prepare_buffers_cpu_ns = elapsed_ns(started);
}

#[allow(clippy::too_many_arguments)]
fn prepare_vab_instance_bind_groups(
    resources: Res<VabInstanceResources>,
    mut buffers: ResMut<VabInstanceBuffers>,
    mut prepared: ResMut<PreparedVabInstances>,
    pipeline: Res<VabInstancePipeline>,
    pipeline_cache: Res<PipelineCache>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    meshes: Res<RenderAssets<RenderMesh>>,
    images: Res<RenderAssets<GpuImage>>,
    mut flash_gpu: ResMut<gpu::FlashGpu>,
    mut texture_groups: ResMut<VabTextureBindGroups>,
    mut filter_uniform_buffers: ResMut<VabFilterUniformBuffers>,
    mut filter_cache: ResMut<VabFilterOutputCache>,
    mut pipelines: ResMut<SpecializedMeshPipelines<VabInstancePipeline>>,
    mut quad_pipelines: ResMut<VabQuadPipelines>,
    mut diagnostics: ResMut<FlashRenderDiagnostics>,
) {
    let started = diagnostics.enabled.then(Instant::now);
    prepared.instances.clear();
    if !buffers.values.is_empty() && buffers.bind_group.is_none() {
        buffers.bind_group = buffers.values.buffer().map(|buffer| {
            render_device.create_bind_group(
                "vab instance instances",
                &pipeline_cache.get_bind_group_layout(&pipeline.draw_layout),
                &[BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                }],
            )
        });
        diagnostics.instance_bind_group_creations += 1;
    } else if !buffers.values.is_empty() {
        diagnostics.instance_bind_group_reuses += 1;
    }
    prepared.draw_bind_group = (!buffers.values.is_empty())
        .then(|| buffers.bind_group.clone())
        .flatten();
    diagnostics.vab_instances_prepared = 0;
    diagnostics.vab_draw_instances_prepared = 0;
    diagnostics.vab_draw_packets = 0;
    let mut prepared_entities = HashSet::<Entity>::new();
    let mut live_uniform_slots = HashSet::new();
    texture_groups.0.retain(|id, _| images.get(*id).is_some());
    for ((view, entity), instance) in &resources.instances {
        let view_samples = instance.view_msaa.samples();
        let filter_samples = instance.filter_msaa.samples();
        let scale = instance.scale;
        let mut packets = Vec::new();
        for (packet_index, (op, scaled_op)) in
            instance.ops.iter().zip(&instance.scaled_ops).enumerate()
        {
            match (op, scaled_op) {
                (Op::Draw(draw), _) => {
                    let Some(&draw_instance) = buffers.indices.get(&(*view, *entity, packet_index))
                    else {
                        continue;
                    };
                    let texture = match draw.texture {
                        None => pipeline.fallback_texture.clone(),
                        Some(id) => {
                            let Some(image) = images.get(id) else {
                                continue;
                            };
                            let texture_view = image.texture_view.id();
                            let sampler = image.sampler.id();
                            let rebuild = texture_groups.0.get(&id).is_none_or(|cached| {
                                cached.texture_view != texture_view || cached.sampler != sampler
                            });
                            if rebuild {
                                diagnostics.material_bind_group_creations += 1;
                                texture_groups.0.insert(
                                    id,
                                    CachedTextureBindGroup {
                                        texture_view,
                                        sampler,
                                        bind_group: create_texture_bind_group(
                                            &render_device,
                                            &pipeline_cache,
                                            &pipeline,
                                            image,
                                        ),
                                    },
                                );
                            } else {
                                diagnostics.material_bind_group_reuses += 1;
                            }
                            texture_groups.0[&id].bind_group.clone()
                        }
                    };
                    if let Some(PreparedVabPacket::Draw(packet)) = packets.last_mut()
                        && packet.mesh == draw.mesh
                        && packet.texture_id == draw.texture
                        && packet.instances.end == draw_instance
                    {
                        packet.instances.end += 1;
                    } else {
                        let packet_pipeline = meshes.get(draw.mesh).and_then(|mesh| {
                            let mesh_key = Mesh2dPipelineKey::from_msaa_samples(view_samples)
                                | Mesh2dPipelineKey::from_target_format(instance.target_format)
                                | Mesh2dPipelineKey::from_primitive_topology_and_strip_index(
                                    mesh.primitive_topology(),
                                    mesh.index_format(),
                                );
                            let mesh_key = if instance.srgb_compositing {
                                mesh_key | Mesh2dPipelineKey::SRGB_COMPOSITING
                            } else {
                                mesh_key
                            };
                            pipelines
                                .specialize(
                                    &pipeline_cache,
                                    &pipeline,
                                    VabInstancePipelineKey(mesh_key),
                                    &mesh.layout,
                                )
                                .ok()
                        });
                        packets.push(PreparedVabPacket::Draw(PreparedDrawPacket {
                            mesh: draw.mesh,
                            texture_id: draw.texture,
                            texture,
                            instances: draw_instance..draw_instance + 1,
                            pipeline: packet_pipeline,
                        }));
                    }
                }
                (
                    Op::Layer { .. } | Op::Mask { .. },
                    Op::Layer { bounds, .. } | Op::Mask { bounds, .. },
                ) => {
                    let Some(&draw_instance) = buffers.indices.get(&(*view, *entity, packet_index))
                    else {
                        continue;
                    };
                    let blend = match op {
                        Op::Layer { blend, .. } => *blend,
                        Op::Mask { .. } => crate::vab_asset::VabBlendMode::Normal,
                        Op::Draw(_) => unreachable!(),
                    };
                    let mut isolated = scaled_op.clone();
                    if let Op::Layer { blend, .. } = &mut isolated {
                        *blend = crate::vab_asset::VabBlendMode::Normal;
                    }
                    // The packet's root target already isolates the result.
                    // An unfiltered outer Layer with Normal internal blending
                    // would allocate an identical child target and composite
                    // it back over transparent pixels for no visual benefit.
                    let isolated_ops = match &isolated {
                        Op::Layer { ops, filters, .. } if filters.is_empty() => ops.as_slice(),
                        _ => std::slice::from_ref(&isolated),
                    };
                    // Only crop the final root surface. Nested filters/masks
                    // retain their full inputs and halos before compositing.
                    let Some(pixel_bounds) = packet_pixel_bounds(*bounds, instance.visible_bounds)
                    else {
                        continue;
                    };
                    let cache_slot = FilterCacheSlot {
                        view: *view,
                        entity: *entity,
                        packet: packet_index,
                    };
                    let cache_content = FilterCacheContent {
                        sample_generation: instance.sample_generation,
                        gpu_source_revision: instance.gpu_source_revision,
                        scale: [scale.x.to_bits(), scale.y.to_bits()],
                        samples: filter_samples,
                        size: [pixel_bounds[2] as u32, pixel_bounds[3] as u32],
                        origin: [pixel_bounds[0].to_bits(), pixel_bounds[1].to_bits()],
                    };
                    live_uniform_slots.insert(cache_slot);
                    let (layer, texture) =
                        if let Some(texture) = filter_cache.get(cache_slot, cache_content) {
                            diagnostics.vab_filter_cache_hits += 1;
                            (None, Some(texture))
                        } else {
                            let uniform_buffer =
                                filter_uniform_buffers.0.entry(cache_slot).or_default();
                            let Ok((layer, allocated)) = gpu::prepare_layer(
                                isolated_ops,
                                pixel_bounds,
                                filter_samples,
                                &mut flash_gpu,
                                &render_device,
                                &render_queue,
                                &meshes,
                                &images,
                                uniform_buffer,
                            ) else {
                                continue;
                            };
                            if allocated {
                                diagnostics.filter_uniform_buffer_allocations += 1;
                            } else {
                                diagnostics.filter_uniform_buffer_reuses += 1;
                            }
                            diagnostics.vab_filter_cache_misses += 1;
                            (Some(layer), None)
                        };
                    packets.push(PreparedVabPacket::Layer(Box::new(PreparedLayerPacket {
                        layer,
                        target: None,
                        texture,
                        cache_slot,
                        cache_content,
                        pixel_bounds,
                        instances: draw_instance..draw_instance + 1,
                        pipeline: Some(
                            *quad_pipelines
                                .0
                                .entry((
                                    instance.target_format,
                                    view_samples,
                                    blend,
                                    instance.srgb_compositing,
                                ))
                                .or_insert_with(|| {
                                    pipeline_cache.queue_render_pipeline(pipeline.quad_descriptor(
                                        instance.target_format,
                                        view_samples,
                                        blend,
                                        instance.srgb_compositing,
                                    ))
                                }),
                        ),
                    })));
                }
                _ => unreachable!("scaled instance operation changed kind"),
            }
        }
        if packets.is_empty() {
            continue;
        }
        diagnostics.vab_draw_packets += packets.len();
        prepared.instances.insert(
            (*view, *entity),
            PreparedVabInstance {
                packets,
                filter_msaa: instance.filter_msaa,
            },
        );
        if diagnostics.enabled {
            prepared_entities.insert(*entity);
        }
    }
    diagnostics.vab_instances_prepared = prepared_entities.len();
    diagnostics.vab_draw_instances_prepared = buffers.indices.len();
    filter_uniform_buffers
        .0
        .retain(|slot, _| live_uniform_slots.contains(slot));
    diagnostics.vab_prepare_bind_groups_cpu_ns = elapsed_ns(started);
}

fn create_texture_bind_group(
    render_device: &RenderDevice,
    pipeline_cache: &PipelineCache,
    pipeline: &VabInstancePipeline,
    image: &GpuImage,
) -> BindGroup {
    render_device.create_bind_group(
        "vab texture bind group",
        &pipeline_cache.get_bind_group_layout(&pipeline.texture_layout),
        &[
            BindGroupEntry {
                binding: 0,
                resource: BindingResource::TextureView(&image.texture_view),
            },
            BindGroupEntry {
                binding: 1,
                resource: BindingResource::Sampler(&image.sampler),
            },
        ],
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_vab_instance_filters(
    mut prepared: ResMut<PreparedVabInstances>,
    pipeline: Res<VabInstancePipeline>,
    pipeline_cache: Res<PipelineCache>,
    mut flash_gpu: ResMut<gpu::FlashGpu>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    locals: (Local<Option<Buffer>>, Local<u64>),
    mut pool: ResMut<super::texture_cache::FrameInternalTextureCache>,
    mut filter_cache: ResMut<VabFilterOutputCache>,
    filter_cache_settings: Res<VabFilterCacheSettings>,
    meshes: Res<RenderAssets<RenderMesh>>,
    allocator: Res<MeshAllocator>,
    mut context: RenderContext,
    mut diagnostics: ResMut<FlashRenderDiagnostics>,
    workload: Option<Res<VabFilterWorkload>>,
    app_frame: Res<FrameCount>,
) {
    let started = diagnostics.enabled.then(Instant::now);
    let (mut diagnostic_frame_buffer, mut render_frame) = locals;
    *render_frame = render_frame.wrapping_add(1);
    let frame = *render_frame;
    diagnostics.vab_filter_layers = 0;
    diagnostics.vab_filter_output_pixels = 0;
    diagnostics.vab_filter_passes = 0;
    diagnostics.vab_filter_processed_pixels = 0;
    let recorder = diagnostics
        .gpu_timing
        .then(|| context.diagnostic_recorder())
        .flatten();
    if let Some(recorder) = &recorder {
        // Bevy publishes these values and the timestamps in the same diagnostic
        // batch. Consumers can identify the generation of a reused span slot.
        let buffer = diagnostic_frame_buffer.get_or_insert_with(|| {
            render_device.create_buffer(&BufferDescriptor {
                label: Some("vab diagnostic frame"),
                size: 8,
                usage: BufferUsages::COPY_SRC | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        });
        render_queue.write_buffer(buffer, 0, &frame.to_le_bytes());
        recorder.record_u32(
            context.command_encoder(),
            &buffer.slice(0..4),
            "vab_filter_frame_low",
        );
        recorder.record_u32(
            context.command_encoder(),
            &buffer.slice(4..8),
            "vab_filter_frame_high",
        );
    }
    let gpu_span = recorder.as_ref().map(|recorder| {
        recorder.time_span(
            context.command_encoder(),
            format!(
                "vab_filter_prepass_{:03}",
                frame % VAB_FILTER_DIAGNOSTIC_SLOTS
            ),
        )
    });
    for instance in prepared.instances.values_mut() {
        for packet in &mut instance.packets {
            let PreparedVabPacket::Layer(packet) = packet else {
                continue;
            };
            diagnostics.vab_filter_layers += 1;
            diagnostics.vab_filter_output_pixels =
                diagnostics.vab_filter_output_pixels.saturating_add(
                    (packet.pixel_bounds[2] as u64).saturating_mul(packet.pixel_bounds[3] as u64),
                );
            let Some(layer) = packet.layer.as_ref() else {
                continue;
            };
            let (passes, pixels) = if diagnostics.enabled {
                gpu::filter_workload(layer)
            } else {
                (0, 0)
            };
            pool.set_allocation_owner(Some(TextureAllocationOwner {
                render_frame: frame,
                app_frame: app_frame.0,
                entity: packet.cache_slot.entity,
                packet: packet.cache_slot.packet,
                packet_bounds: packet.pixel_bounds,
            }));
            let target = gpu::render_prepared_layer(
                layer,
                Srgba::NONE,
                &mut flash_gpu,
                &render_device,
                &mut pool,
                &instance.filter_msaa,
                &meshes,
                &allocator,
                &mut context,
                &mut diagnostics.draw_calls,
            );
            pool.set_allocation_owner(None);
            diagnostics.vab_filter_passes = diagnostics.vab_filter_passes.saturating_add(passes);
            diagnostics.vab_filter_processed_pixels = diagnostics
                .vab_filter_processed_pixels
                .saturating_add(pixels);
            let texture = render_device.create_bind_group(
                "vab filtered layer texture",
                &pipeline_cache.get_bind_group_layout(&pipeline.texture_layout),
                &[
                    BindGroupEntry {
                        binding: 0,
                        resource: BindingResource::TextureView(target.main_view()),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: BindingResource::Sampler(&pipeline.sampler),
                    },
                ],
            );
            packet.texture = Some(texture.clone());
            let bytes = u64::from(packet.cache_content.size[0])
                .saturating_mul(u64::from(packet.cache_content.size[1]))
                .saturating_mul(4);
            if filter_cache.reserve(packet.cache_slot, bytes, filter_cache_settings.max_bytes) {
                let persistent = target.into_persistent_main();
                filter_cache.insert(
                    packet.cache_slot,
                    packet.cache_content,
                    persistent,
                    texture,
                    bytes,
                );
            } else {
                packet.target = Some(target);
            }
        }
    }
    if let Some(span) = gpu_span {
        span.end(context.command_encoder());
    }
    if !diagnostics.enabled {
        return;
    }
    pool.update_live_metrics();
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
    diagnostics.vab_filter_cache_entries = filter_cache.entries.len();
    diagnostics.vab_filter_cache_bytes = filter_cache.resident_bytes;
    if let Some(workload) = workload {
        workload.stage(VabFilterWorkloadSample {
            frame,
            app_frame: app_frame.0,
            layers: diagnostics.vab_filter_layers,
            cache_hits: diagnostics.vab_filter_cache_hits,
            cache_misses: diagnostics.vab_filter_cache_misses,
            passes: diagnostics.vab_filter_passes,
            processed_pixels: diagnostics.vab_filter_processed_pixels,
            extract_cpu_ns: diagnostics.vab_extract_cpu_ns,
            queue_cpu_ns: diagnostics.vab_queue_cpu_ns,
            prepare_buffers_cpu_ns: diagnostics.vab_prepare_buffers_cpu_ns,
            prepare_bind_groups_cpu_ns: diagnostics.vab_prepare_bind_groups_cpu_ns,
            prepass_cpu_ns: elapsed_ns(started),
            filter_cache_bytes: filter_cache.resident_bytes,
            texture_allocations: pool.allocations,
            texture_reuses: pool.reuses,
            texture_first_in_bucket_allocations: pool.first_in_bucket_allocations,
            texture_exhausted_bucket_allocations: pool.exhausted_bucket_allocations,
            texture_allocated_bytes: pool.allocated_bytes,
            pooled_texture_bytes: pool.pooled_texture_bytes,
            peak_live_transient_bytes: pool.peak_live_bytes,
            pool_end_bytes: None,
            pool_end_live_bytes: None,
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn queue_vab_instances(
    draw_functions: Res<DrawFunctions<Transparent2d>>,
    pipeline: Res<VabInstancePipeline>,
    mut pipelines: ResMut<SpecializedMeshPipelines<VabInstancePipeline>>,
    pipeline_cache: Res<PipelineCache>,
    meshes: Res<RenderAssets<RenderMesh>>,
    resources: Res<VabInstanceResources>,
    mut quad_pipelines: ResMut<VabQuadPipelines>,
    mut phases: ResMut<ViewSortedRenderPhases<Transparent2d>>,
    views: Query<(
        &ExtractedView,
        &ExtractedCamera,
        &RenderVisibleEntities,
        &Msaa,
    )>,
    mut diagnostics: ResMut<FlashRenderDiagnostics>,
) {
    diagnostics.vab_instances_queued = 0;
    let draw_function = draw_functions.read().id::<DrawVabInstance>();
    for (view, camera, visible, msaa) in &views {
        let srgb_compositing = camera.compositing_space == Some(CompositingSpace::Srgb);
        let Some(phase) = phases.get_mut(&view.retained_view_entity) else {
            continue;
        };
        let Some(visible) = visible.get::<VabAssetHandle>() else {
            continue;
        };
        for (render_entity, main_entity) in visible.iter_visible() {
            let Some(instance) = resources
                .instances
                .get(&(view.retained_view_entity, *render_entity))
            else {
                continue;
            };
            let Some((pipeline_id, indexed)) = instance.ops.iter().find_map(|op| match op {
                Op::Draw(draw) => {
                    let mesh = meshes.get(draw.mesh)?;
                    let mesh_key = Mesh2dPipelineKey::from_msaa_samples(msaa.samples())
                        | Mesh2dPipelineKey::from_target_format(view.target_format)
                        | Mesh2dPipelineKey::from_primitive_topology_and_strip_index(
                            mesh.primitive_topology(),
                            mesh.index_format(),
                        );
                    let mesh_key = if srgb_compositing {
                        mesh_key | Mesh2dPipelineKey::SRGB_COMPOSITING
                    } else {
                        mesh_key
                    };
                    pipelines
                        .specialize(
                            &pipeline_cache,
                            &pipeline,
                            VabInstancePipelineKey(mesh_key),
                            &mesh.layout,
                        )
                        .ok()
                        .map(|id| (id, mesh.indexed()))
                }
                Op::Layer { blend, .. } => {
                    let key = (view.target_format, msaa.samples(), *blend, srgb_compositing);
                    Some((
                        *quad_pipelines.0.entry(key).or_insert_with(|| {
                            pipeline_cache.queue_render_pipeline(pipeline.quad_descriptor(
                                view.target_format,
                                msaa.samples(),
                                *blend,
                                srgb_compositing,
                            ))
                        }),
                        false,
                    ))
                }
                Op::Mask { .. } => {
                    let blend = crate::vab_asset::VabBlendMode::Normal;
                    let key = (view.target_format, msaa.samples(), blend, srgb_compositing);
                    Some((
                        *quad_pipelines.0.entry(key).or_insert_with(|| {
                            pipeline_cache.queue_render_pipeline(pipeline.quad_descriptor(
                                view.target_format,
                                msaa.samples(),
                                blend,
                                srgb_compositing,
                            ))
                        }),
                        false,
                    ))
                }
            }) else {
                continue;
            };
            phase.add_retained(Transparent2d {
                sort_key: FloatOrd(Mat4::from(instance.global).w_axis.z),
                entity: (*render_entity, *main_entity),
                pipeline: pipeline_id,
                draw_function,
                batch_range: 0..1,
                extra_index: PhaseItemExtraIndex::None,
                extracted_index: usize::MAX,
                indexed,
            });
            diagnostics.vab_instances_queued += 1;
        }
    }
}

type DrawVabInstance = (SetMesh2dViewBindGroup<0>, DrawVabInstanceCommands);

struct DrawVabInstanceCommands;

impl RenderCommand<Transparent2d> for DrawVabInstanceCommands {
    type Param = (
        SRes<PreparedVabInstances>,
        SRes<RenderAssets<RenderMesh>>,
        SRes<MeshAllocator>,
        SRes<PipelineCache>,
        SRes<FlashRenderDiagnostics>,
    );
    type ViewQuery = &'static ExtractedView;
    type ItemQuery = ();

    fn render<'w>(
        item: &Transparent2d,
        view: &'w ExtractedView,
        _entity: Option<()>,
        (prepared, meshes, allocator, pipeline_cache, diagnostics): SystemParamItem<
            'w,
            '_,
            Self::Param,
        >,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let diagnostics = diagnostics.into_inner();
        let prepared = prepared.into_inner();
        let Some(instance) = prepared
            .instances
            .get(&(view.retained_view_entity, item.entity()))
        else {
            return RenderCommandResult::Skip;
        };
        let Some(draw_bind_group) = &prepared.draw_bind_group else {
            return RenderCommandResult::Skip;
        };
        let pipeline_cache = pipeline_cache.into_inner();
        let meshes = meshes.into_inner();
        let allocator = allocator.into_inner();
        pass.set_bind_group(1, draw_bind_group, &[]);
        for packet in &instance.packets {
            match packet {
                PreparedVabPacket::Draw(packet) => {
                    let Some(pipeline) = packet
                        .pipeline
                        .and_then(|id| pipeline_cache.get_render_pipeline(id))
                    else {
                        continue;
                    };
                    let Some(mesh) = meshes.get(packet.mesh) else {
                        continue;
                    };
                    let Some(vertices) = allocator.mesh_vertex_slice(&packet.mesh) else {
                        continue;
                    };
                    pass.set_render_pipeline(pipeline);
                    pass.set_bind_group(2, &packet.texture, &[]);
                    pass.set_vertex_buffer(0, vertices.buffer.slice(..));
                    match &mesh.buffer_info {
                        RenderMeshBufferInfo::Indexed {
                            count,
                            index_format,
                        } => {
                            let Some(indices) = allocator.mesh_index_slice(&packet.mesh) else {
                                continue;
                            };
                            pass.set_index_buffer(indices.buffer.slice(..), *index_format);
                            pass.draw_indexed(
                                indices.range.start..indices.range.start + count,
                                vertices.range.start as i32,
                                packet.instances.clone(),
                            );
                        }
                        RenderMeshBufferInfo::NonIndexed => {
                            pass.draw(vertices.range, packet.instances.clone());
                        }
                    }
                    if diagnostics.enabled {
                        diagnostics.vab_draw_calls.fetch_add(1, Ordering::Relaxed);
                    }
                }
                PreparedVabPacket::Layer(packet) => {
                    let Some(pipeline) = packet
                        .pipeline
                        .and_then(|id| pipeline_cache.get_render_pipeline(id))
                    else {
                        continue;
                    };
                    let Some(texture) = &packet.texture else {
                        continue;
                    };
                    pass.set_render_pipeline(pipeline);
                    pass.set_bind_group(2, texture, &[]);
                    pass.draw(0..6, packet.instances.clone());
                    if diagnostics.enabled {
                        diagnostics.vab_draw_calls.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        }
        RenderCommandResult::Success
    }
}

fn clear_prepared_vab_instances(mut prepared: ResMut<PreparedVabInstances>) {
    prepared.instances.clear();
    prepared.draw_bind_group = None;
}

fn trim_idle_texture_pool(
    mut pool: ResMut<super::texture_cache::FrameInternalTextureCache>,
    settings: Res<super::texture_cache::TransientTexturePoolSettings>,
    mut diagnostics: ResMut<FlashRenderDiagnostics>,
    workload: Option<Res<VabFilterWorkload>>,
) {
    pool.end_frame(&settings);
    if !diagnostics.enabled {
        return;
    }
    diagnostics.live_transient_textures = pool.live_textures;
    diagnostics.live_transient_bytes = pool.live_bytes;
    diagnostics.pooled_textures = pool.pooled_textures;
    diagnostics.pooled_texture_bytes = pool.pooled_texture_bytes;
    diagnostics.pooled_idle_textures = pool.pooled_idle_textures;
    diagnostics.pooled_idle_bytes = pool.pooled_idle_bytes;
    diagnostics.texture_pool_buckets = pool.bucket_count;
    diagnostics.largest_texture_pool_bucket = pool.largest_bucket_textures;
    diagnostics.largest_texture_pool_bucket_bytes = pool.largest_bucket_bytes;
    if let Some(workload) = workload {
        workload.record_pool_end(pool.pooled_texture_bytes, pool.live_bytes);
    }
}

#[cfg(test)]
mod clipping_tests {
    use super::*;

    #[test]
    fn visible_bounds_follow_translated_rotated_and_scaled_planes() {
        let matrix = Mat4::from_scale_rotation_translation(
            Vec3::new(0.02, -0.04, 1.0),
            Quat::from_rotation_z(0.7),
            Vec3::new(0.3, -0.5, 0.0),
        );
        let scale = Vec2::new(2.0, 4.0);
        let bounds = visible_pixel_bounds(matrix, scale).unwrap();
        let inverse = matrix.inverse();
        for corner in [
            Vec2::NEG_ONE,
            Vec2::new(-1.0, 1.0),
            Vec2::ONE,
            Vec2::new(1.0, -1.0),
        ] {
            let point = inverse.transform_point3(corner.extend(0.0)).truncate() * scale;
            assert!(point.x >= bounds[0] + 1.0 && point.x <= bounds[0] + bounds[2] - 1.0);
            assert!(point.y >= bounds[1] + 1.0 && point.y <= bounds[1] + bounds[3] - 1.0);
        }
        assert!(visible_pixel_bounds(Mat4::ZERO, Vec2::ONE).is_none());
        let mut perspective = matrix;
        perspective.x_axis.w = 0.1;
        assert!(visible_pixel_bounds(perspective, scale).is_none());
    }

    #[test]
    fn root_crop_rejects_invisible_packets_and_keeps_sampling_margin() {
        let visible = visible_pixel_bounds(
            Mat4::from_scale(Vec3::new(1.0 / 32.0, -1.0 / 32.0, 1.0)),
            Vec2::ONE,
        );
        assert_eq!(visible, Some([-34.0, -34.0, 68.0, 68.0]));
        let cropped = packet_pixel_bounds([-3104.0, -3104.0, 6208.0, 6208.0], visible).unwrap();
        assert_eq!(cropped, [-34.0, -34.0, 80.0, 80.0]);
        assert!(packet_pixel_bounds([100.0, 100.0, 20.0, 20.0], visible).is_none());
    }
}
