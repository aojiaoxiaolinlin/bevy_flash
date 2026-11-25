use bevy::platform::time::Instant;
use std::{
    collections::{HashSet, VecDeque},
    sync::{Arc, Mutex, atomic::Ordering},
};
use std::{hash::Hash, ops::Range};

use bevy::{
    asset::{AssetEvent, AssetEventSystems, AssetId, embedded_asset, load_embedded_asset},
    camera::{CompositingSpace, primitives::Aabb, visibility::VisibilitySystems},
    core_pipeline::core_2d::{CORE_2D_DEPTH_FORMAT, Transparent2d},
    diagnostic::FrameCount,
    ecs::system::{SystemParamItem, lifetimeless::SRes},
    math::{Affine3A, FloatOrd, Mat2, Mat4, Vec4},
    mesh::{Mesh, MeshVertexBufferLayoutRef},
    platform::collections::HashMap,
    prelude::*,
    render::{
        Extract, ExtractSchedule, Render, RenderApp, RenderStartup, RenderSystems,
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
        view::{
            ExtractedView, Msaa, RenderVisibleEntities, ResolvedCompositingSpace,
            RetainedViewEntity,
        },
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
    vab_asset::{ATTRIBUTE_VAB_COLOR_SRGB, ATTRIBUTE_VAB_POSITION, VabAsset, VabAssetHandle},
    vab_player::{VabPlayer, advance_vab_animations},
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
        embedded_asset!(app, "shaders/vab_instance.wesl");
        app.init_resource::<VabAssetRevisions>()
            .init_resource::<VabMaterialRevision>()
            .init_resource::<VabGpuSourceRevision>()
            .init_resource::<VabBoundsCache>()
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
            .add_systems(
                PostUpdate,
                sync_vab_aabbs
                    .after(track_vab_asset_revisions)
                    .after(advance_vab_animations)
                    .before(VisibilitySystems::CheckVisibility),
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

#[allow(clippy::unnecessary_fold)] // Consume every Shader event, including after the first change.
fn track_vab_gpu_source_revision(
    mut images: Option<MessageReader<AssetEvent<Image>>>,
    mut meshes: Option<MessageReader<AssetEvent<Mesh>>>,
    mut shaders: Option<MessageReader<AssetEvent<Shader>>>,
    mut revision: ResMut<VabGpuSourceRevision>,
) {
    let image_changed = images
        .as_mut()
        .is_some_and(|events| events.read().next().is_some());
    let mesh_changed = meshes
        .as_mut()
        .is_some_and(|events| events.read().next().is_some());
    let shader_changed = shaders.as_mut().is_some_and(|events| {
        events.read().fold(false, |dirty, event| {
            matches!(
                event,
                AssetEvent::Modified { .. } | AssetEvent::Removed { .. }
            ) || dirty
        })
    });
    if image_changed || mesh_changed || shader_changed {
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
#[extract_app(bevy::render::RenderApp)]
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
#[extract_app(bevy::render::RenderApp)]
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

/// Content-addressed key for reusable canonicalized filter symbol stamps.
/// Shared across packets within the same frame, across animation frames,
/// across entities, and across views with matching pixel scale/MSAA/quality.
#[derive(Clone, Copy, Hash, PartialEq, Eq, Debug)]
struct FilterStampKey {
    hash_a: u64,
    hash_b: u64,
    gpu_source_revision: u64,
    samples: u32,
    filter_quality: [u32; 4],
    size: [u32; 2],
    origin: [i32; 2],
    child_count: u16,
    filter_count: u16,
}

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
enum FilterCacheKey {
    Slot(FilterCacheSlot),
    Stamp(FilterStampKey),
}

struct FilterStampPlan {
    key: FilterStampKey,
    canonical_op: Op,
    stamp_pixel_bounds: [f32; 4],
    placement: vatf::animation::AnimMatrix,
}

#[derive(Resource, Default)]
struct VabBoundsCache {
    revisions: HashMap<AssetId<VabAsset>, u64>,
    clips: HashMap<(AssetId<VabAsset>, usize), Option<Aabb>>,
}

#[derive(Component, Clone, Copy, PartialEq, Eq)]
struct SyncedVabAabb {
    asset: AssetId<VabAsset>,
    revision: u64,
    clip: usize,
}

#[allow(clippy::type_complexity)]
fn sync_vab_aabbs(
    mut commands: Commands,
    assets: Res<Assets<VabAsset>>,
    revisions: Res<VabAssetRevisions>,
    mut cache: ResMut<VabBoundsCache>,
    query: Query<
        (
            Entity,
            &VabAssetHandle,
            &VabPlayer,
            Option<&SyncedVabAabb>,
            Option<&Aabb>,
        ),
        Without<super::RenderOffscreenTexture>,
    >,
) {
    cache.revisions.retain(|id, _| assets.contains(*id));
    cache.clips.retain(|(id, _), _| assets.contains(*id));
    for (entity, handle, player, synced, existing_aabb) in &query {
        let asset_id = handle.0.id();
        let revision = revisions.0.get(&asset_id).copied().unwrap_or(0);
        let clip = player.clip();
        let key = SyncedVabAabb {
            asset: asset_id,
            revision,
            clip,
        };
        if synced == Some(&key) {
            continue;
        }
        let Some(asset) = assets.get(&handle.0) else {
            continue;
        };
        if cache.revisions.get(&asset_id) != Some(&revision) {
            cache.clips.retain(|(id, _), _| *id != asset_id);
            cache.revisions.insert(asset_id, revision);
        }
        let aabb = *cache.clips.entry((asset_id, clip)).or_insert_with(|| {
            asset
                .clip_bounds(clip)
                .or_else(|| asset.content_bounds())
                .map(|[x, y, w, h]| {
                    // Flash Y-down maps to Bevy Y-up via VIEW_MATRIX.
                    // Keep a small guard margin for filter rounding and flat 2D planes.
                    Aabb::from_min_max(
                        Vec3::new(x - 2.0, -(y + h) - 2.0, -1.0),
                        Vec3::new(x + w + 2.0, -y + 2.0, 1.0),
                    )
                })
        });
        let mut entity_commands = commands.entity(entity);
        entity_commands.insert(key);
        match aabb {
            Some(aabb) if existing_aabb != Some(&aabb) => {
                entity_commands.insert(aabb);
            }
            None if existing_aabb.is_some() => {
                entity_commands.remove::<Aabb>();
            }
            _ => {}
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct FilterCacheContent {
    sample_generation: u64,
    gpu_source_revision: u64,
    scale: [u32; 2],
    samples: u32,
    size: [u32; 2],
    origin: [u32; 2],
    filter_quality: [u32; 4],
}

struct CachedFilterOutput {
    content: Option<FilterCacheContent>,
    _texture: super::texture_cache::PersistentTexture,
    bind_group: BindGroup,
    bytes: u64,
    last_used_frame: u64,
}

#[derive(Resource, Default)]
pub(super) struct VabFilterOutputCache {
    entries: HashMap<FilterCacheKey, CachedFilterOutput>,
    frame: u64,
    resident_bytes: u64,
}

impl VabFilterOutputCache {
    fn begin_frame(&mut self, settings: &VabFilterCacheSettings) {
        self.frame = self.frame.wrapping_add(1);
        let frame = self.frame;
        let slot_ttl = settings.max_unused_frames;
        let stamp_ttl = settings.max_unused_frames.saturating_mul(8);
        self.retain(|key, entry| {
            let ttl = match key {
                FilterCacheKey::Slot(_) => slot_ttl,
                FilterCacheKey::Stamp(_) => stamp_ttl,
            };
            frame.wrapping_sub(entry.last_used_frame) <= ttl
        });
        self.trim_to(settings.max_bytes, 0);
    }

    fn get_slot(
        &mut self,
        slot: FilterCacheSlot,
        content: FilterCacheContent,
    ) -> Option<BindGroup> {
        let key = FilterCacheKey::Slot(slot);
        if self
            .entries
            .get(&key)
            .is_some_and(|entry| entry.content != Some(content))
        {
            self.remove(key);
        }
        let entry = self.entries.get_mut(&key)?;
        entry.last_used_frame = self.frame;
        Some(entry.bind_group.clone())
    }

    fn get_stamp(&mut self, stamp: FilterStampKey) -> Option<BindGroup> {
        let key = FilterCacheKey::Stamp(stamp);
        let entry = self.entries.get_mut(&key)?;
        entry.last_used_frame = self.frame;
        Some(entry.bind_group.clone())
    }

    fn insert(
        &mut self,
        key: FilterCacheKey,
        content: Option<FilterCacheContent>,
        texture: super::texture_cache::PersistentTexture,
        bind_group: BindGroup,
        bytes: u64,
    ) {
        self.remove(key);
        self.resident_bytes = self.resident_bytes.saturating_add(bytes);
        self.entries.insert(
            key,
            CachedFilterOutput {
                content,
                _texture: texture,
                bind_group,
                bytes,
                last_used_frame: self.frame,
            },
        );
    }

    fn reserve(&mut self, key: FilterCacheKey, bytes: u64, max_bytes: u64) -> bool {
        if bytes > max_bytes {
            return false;
        }
        self.remove(key);
        self.trim_to(max_bytes, bytes);
        self.resident_bytes.saturating_add(bytes) <= max_bytes
    }

    fn trim_to(&mut self, max_bytes: u64, incoming: u64) {
        while self.resident_bytes.saturating_add(incoming) > max_bytes {
            let Some((&key, _)) = self
                .entries
                .iter()
                .filter(|(_, entry)| entry.last_used_frame != self.frame)
                .min_by_key(|(_, entry)| entry.last_used_frame)
            else {
                break;
            };
            self.remove(key);
        }
    }

    fn remove(&mut self, key: FilterCacheKey) {
        if let Some(entry) = self.entries.remove(&key) {
            self.resident_bytes = self.resident_bytes.saturating_sub(entry.bytes);
        }
    }

    fn retain(&mut self, mut keep: impl FnMut(&FilterCacheKey, &CachedFilterOutput) -> bool) {
        self.entries.retain(|key, entry| {
            let retain = keep(key, entry);
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
    packet_stamps: Vec<Option<FilterStampPlan>>,
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
    world_row0: [f32; 4],
    world_row1: [f32; 4],
    world_row2: [f32; 4],
    uv0: [f32; 4],
    uv1: [f32; 4],
    multiply: [f32; 4],
    add: [f32; 4],
    material: [f32; 4],
}

struct PreparedDrawPacket {
    mesh: AssetId<Mesh>,
    texture_id: Option<AssetId<Image>>,
    texture: BindGroup,
    blend: crate::vab_asset::VabBlendMode,
    instances: Range<u32>,
    pipeline: Option<CachedRenderPipelineId>,
}

struct PreparedLayerPacket {
    layer: Option<gpu::PreparedLayer>,
    target: Option<super::texture_cache::TextureTarget>,
    texture: Option<BindGroup>,
    stamp_id: Option<FilterStampKey>,
    pending_stamp: Option<usize>,
    cache_slot: FilterCacheSlot,
    cache_content: FilterCacheContent,
    pixel_bounds: [f32; 4],
    instances: Range<u32>,
    pipeline: Option<CachedRenderPipelineId>,
}

struct PreparedFilterStamp {
    key: FilterStampKey,
    layer: gpu::PreparedLayer,
    filter_msaa: Msaa,
    owner_entity: Entity,
    owner_packet: usize,
    pixel_bounds: [f32; 4],
    target: Option<super::texture_cache::TextureTarget>,
    texture: Option<BindGroup>,
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
    stamps: Vec<PreparedFilterStamp>,
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
struct VabFilterUniformBuffers {
    slots: HashMap<FilterCacheSlot, gpu::ReusableUniformBuffer>,
    stamps: Vec<gpu::ReusableUniformBuffer>,
}

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
        shader: load_embedded_asset!(asset_server.as_ref(), "shaders/vab_instance.wesl"),
        fallback_texture,
        sampler,
    });
}

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub(super) struct VabInstancePipelineKey {
    mesh: Mesh2dPipelineKey,
    blend: crate::vab_asset::VabBlendMode,
}

impl SpecializedMeshPipeline for VabInstancePipeline {
    type Key = VabInstancePipelineKey;

    fn specialize(
        &self,
        key: Self::Key,
        layout: &MeshVertexBufferLayoutRef,
    ) -> Result<RenderPipelineDescriptor, SpecializedMeshPipelineError> {
        let compact = layout.0.contains(ATTRIBUTE_VAB_POSITION);
        let vertex_layout = if compact {
            layout.0.get_layout(&[
                ATTRIBUTE_VAB_POSITION.at_shader_location(0),
                ATTRIBUTE_VAB_COLOR_SRGB.at_shader_location(1),
            ])?
        } else {
            layout.0.get_layout(&[
                Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
                Mesh::ATTRIBUTE_COLOR.at_shader_location(1),
            ])?
        };
        let mut vertex_defs = Vec::new();
        let mut fragment_defs = Vec::new();
        if compact {
            vertex_defs.push("COMPACT_VERTEX".into());
            fragment_defs.push("COMPACT_VERTEX".into());
        }
        if key.mesh.contains(Mesh2dPipelineKey::SRGB_COMPOSITING) {
            fragment_defs.push("SRGB_OUTPUT".into());
        }
        Ok(RenderPipelineDescriptor {
            label: Some("vab instance pipeline".into()),
            layout: vec![
                self.view_layout.clone(),
                self.draw_layout.clone(),
                self.texture_layout.clone(),
            ],
            vertex: VertexState {
                shader: self.shader.clone(),
                shader_defs: vertex_defs,
                entry_point: Some("vertex".into()),
                buffers: vec![vertex_layout],
                ..default()
            },
            fragment: Some(FragmentState {
                shader: self.shader.clone(),
                shader_defs: fragment_defs,
                targets: vec![Some(ColorTargetState {
                    format: key.mesh.target_format(),
                    blend: Some(gpu::fixed_blend_state(key.blend)),
                    write_mask: ColorWrites::ALL,
                })],
                ..default()
            }),
            primitive: PrimitiveState {
                topology: key.mesh.as_base_mesh_pipeline_key().primitive_topology(),
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
                count: key.mesh.msaa_samples(),
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
                    vec!["SRGB_OUTPUT".into()]
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
            let ops = super::extract::flatten_direct_fixed_blends(ops, true);
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
    count_unsupported_layers_inner(ops, false)
}

fn count_unsupported_layers_inner(ops: &[Op], nested: bool) -> usize {
    ops.iter()
        .map(|op| match op {
            Op::Draw(_) => 0,
            Op::Mask { mask, content, .. } => {
                count_unsupported_layers_inner(mask, true)
                    + count_unsupported_layers_inner(content, true)
            }
            Op::Layer {
                ops,
                filters,
                blend,
                ..
            } => {
                let unsupported_filter = filters.iter().any(|filter| {
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
                });
                // Nested layers inside an isolated TextureTarget support all 14
                // Flash blend modes (including complex destination-sampling modes).
                // Top-level Camera2d layers only have fixed-function hardware blending.
                let unsupported_blend = !nested
                    && !matches!(
                        blend,
                        crate::vab_asset::VabBlendMode::Normal
                            | crate::vab_asset::VabBlendMode::Layer
                            | crate::vab_asset::VabBlendMode::Add
                            | crate::vab_asset::VabBlendMode::Subtract
                            | crate::vab_asset::VabBlendMode::Screen
                            | crate::vab_asset::VabBlendMode::Lighten
                    );
                usize::from(unsupported_filter)
                    + usize::from(unsupported_blend)
                    + count_unsupported_layers_inner(ops, true)
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

const MAX_STAMP_DIMENSION: f32 = 1024.0;

struct StampHasher {
    a: u64,
    b: u64,
}

impl StampHasher {
    fn new() -> Self {
        Self {
            a: 0xcbf2_9ce4_8422_2325,
            b: 0x9e37_79b9_7f4a_7c15,
        }
    }

    fn mix(&mut self, value: u64) {
        self.a ^= value;
        self.a = self.a.wrapping_mul(0x100_0000_01b3);
        let mut z = self
            .b
            .wrapping_add(value.wrapping_add(0x9e37_79b9_7f4a_7c15));
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        self.b = z ^ (z >> 31);
    }

    fn write_u8(&mut self, v: u8) {
        self.mix(u64::from(v));
    }

    fn write_i16(&mut self, v: i16) {
        self.mix(v as u16 as u64);
    }

    fn write_u32(&mut self, v: u32) {
        self.mix(u64::from(v));
    }

    fn write_i32(&mut self, v: i32) {
        self.mix(v as u32 as u64);
    }

    fn write_q(&mut self, v: f32, scale: f32) {
        self.write_i32((v * scale).round() as i32);
    }
}

impl std::hash::Hasher for StampHasher {
    fn finish(&self) -> u64 {
        self.a ^ self.b
    }

    fn write(&mut self, bytes: &[u8]) {
        let (chunks, rem) = bytes.as_chunks::<8>();
        for chunk in chunks {
            self.mix(u64::from_le_bytes(*chunk));
        }
        if !rem.is_empty() {
            let mut tail = [0u8; 8];
            tail[..rem.len()].copy_from_slice(rem);
            self.mix(u64::from_le_bytes(tail));
        }
    }
}

fn is_isotropic_filter_chain(filters: &[vatf::animation::AnimFilter]) -> bool {
    use vatf::animation::AnimFilter;
    filters.iter().all(|filter| match filter {
        AnimFilter::BlurFilter(f) => (f.blur_x - f.blur_y).abs() <= 1,
        AnimFilter::GlowFilter(f) => (f.blur_x - f.blur_y).abs() <= 1,
        AnimFilter::ColorMatrixFilter(_) => true,
        AnimFilter::DropShadowFilter(_)
        | AnimFilter::BevelFilter(_)
        | AnimFilter::ConvolutionFilter(_)
        | AnimFilter::GradientGlowFilter(_)
        | AnimFilter::GradientBevelFilter(_) => false,
    })
}

fn anim_mat_mul(
    l: vatf::animation::AnimMatrix,
    r: vatf::animation::AnimMatrix,
) -> vatf::animation::AnimMatrix {
    vatf::animation::AnimMatrix {
        a: l.a * r.a + l.c * r.b,
        b: l.b * r.a + l.d * r.b,
        c: l.a * r.c + l.c * r.d,
        d: l.b * r.c + l.d * r.d,
        tx: l.a * r.tx + l.c * r.ty + l.tx,
        ty: l.b * r.tx + l.d * r.ty + l.ty,
    }
}

fn anim_mat_inv(m: vatf::animation::AnimMatrix) -> Option<vatf::animation::AnimMatrix> {
    let det = m.a * m.d - m.b * m.c;
    if !det.is_finite() || det.abs() < 1e-6 {
        return None;
    }
    let inv = 1.0 / det;
    let a = m.d * inv;
    let b = -m.b * inv;
    let c = -m.c * inv;
    let d = m.a * inv;
    let tx = -(a * m.tx + c * m.ty);
    let ty = -(b * m.tx + d * m.ty);
    Some(vatf::animation::AnimMatrix { a, b, c, d, tx, ty })
}

fn snap_canonical_matrix(
    m: vatf::animation::AnimMatrix,
    is_first: bool,
    isotropic: bool,
) -> vatf::animation::AnimMatrix {
    let snap = |v: f32, grid: f32| {
        let s = (v * grid).round() / grid;
        if s == 0.0 { 0.0 } else { s }
    };
    vatf::animation::AnimMatrix {
        a: snap(m.a, 1024.0),
        b: if is_first && isotropic {
            0.0
        } else {
            snap(m.b, 1024.0)
        },
        c: snap(m.c, 1024.0),
        d: snap(m.d, 1024.0),
        tx: if is_first { 0.0 } else { snap(m.tx, 64.0) },
        ty: if is_first { 0.0 } else { snap(m.ty, 64.0) },
    }
}

fn hash_anim_filter(h: &mut StampHasher, filter: &vatf::animation::AnimFilter) {
    use vatf::animation::AnimFilter;
    let q_blur = |fixed16: i32| (fixed16 + 128) >> 8;
    match filter {
        AnimFilter::BlurFilter(f) => {
            h.write_u8(1);
            h.write_i32(q_blur(f.blur_x));
            h.write_i32(q_blur(f.blur_y));
            h.write_u8(f.num_passes);
        }
        AnimFilter::GlowFilter(f) => {
            h.write_u8(2);
            h.write_u8(f.flags);
            h.write_u8(f.color_r);
            h.write_u8(f.color_g);
            h.write_u8(f.color_b);
            h.write_u8(f.color_a);
            h.write_i32(q_blur(f.blur_x));
            h.write_i32(q_blur(f.blur_y));
            h.write_i16(f.strength);
            h.write_u8(f.num_passes);
        }
        AnimFilter::DropShadowFilter(f) => {
            h.write_u8(3);
            h.write_u8(f.flags);
            h.write_u8(f.color_r);
            h.write_u8(f.color_g);
            h.write_u8(f.color_b);
            h.write_u8(f.color_a);
            h.write_i32(q_blur(f.blur_x));
            h.write_i32(q_blur(f.blur_y));
            h.write_i32(f.angle);
            h.write_i32(q_blur(f.distance));
            h.write_i16(f.strength);
            h.write_u8(f.num_passes);
        }
        AnimFilter::BevelFilter(f) => {
            h.write_u8(4);
            h.write_u8(f.flags);
            h.write_u8(f.shadow_color_r);
            h.write_u8(f.shadow_color_g);
            h.write_u8(f.shadow_color_b);
            h.write_u8(f.shadow_color_a);
            h.write_u8(f.highlight_color_r);
            h.write_u8(f.highlight_color_g);
            h.write_u8(f.highlight_color_b);
            h.write_u8(f.highlight_color_a);
            h.write_i32(q_blur(f.blur_x));
            h.write_i32(q_blur(f.blur_y));
            h.write_i32(f.angle);
            h.write_i32(q_blur(f.distance));
            h.write_i16(f.strength);
            h.write_u8(f.num_passes);
        }
        AnimFilter::ColorMatrixFilter(f) => {
            h.write_u8(5);
            for &v in &f.matrix {
                h.write_q(v, 1024.0);
            }
        }
        AnimFilter::ConvolutionFilter(f) => {
            h.write_u8(6);
            h.write_u8(f.num_matrix_rows);
            h.write_u8(f.num_matrix_cols);
            for &v in &f.matrix {
                h.write_q(v, 1024.0);
            }
            h.write_q(f.divisor, 1024.0);
            h.write_q(f.bias, 1024.0);
            h.write_u8(f.default_color_r);
            h.write_u8(f.default_color_g);
            h.write_u8(f.default_color_b);
            h.write_u8(f.default_color_a);
            h.write_u8(f.flags);
        }
        AnimFilter::GradientGlowFilter(f) | AnimFilter::GradientBevelFilter(f) => {
            h.write_u8(if matches!(filter, AnimFilter::GradientGlowFilter(_)) {
                7
            } else {
                8
            });
            h.write_u8(f.flags);
            h.write_u32(f.colors.len() as u32);
            for c in &f.colors {
                h.write_u8(c.ratio);
                h.write_u8(c.color_r);
                h.write_u8(c.color_g);
                h.write_u8(c.color_b);
                h.write_u8(c.color_a);
            }
            h.write_i32(q_blur(f.blur_x));
            h.write_i32(q_blur(f.blur_y));
            h.write_i32(f.angle);
            h.write_i32(q_blur(f.distance));
            h.write_i16(f.strength);
            h.write_u8(f.num_passes);
        }
    }
}

/// Canonicalize a top-level flat filtered symbol (1 or more `Op::Draw` children)
/// into a reusable stamp plan, factoring out translation, isotropic rotation,
/// and (when enabled via `VabFilterQuality::reuse_scaled_stamps`) octave-quantized
/// uniform particle scaling.
fn build_filter_stamp_plan(
    scaled_op: &Op,
    visible_bounds: Option<[f32; 4]>,
    gpu_source_revision: u64,
    filter_samples: u32,
    quality: gpu::VabFilterQuality,
) -> Option<FilterStampPlan> {
    use std::hash::Hash;

    let Op::Layer {
        ops,
        bounds,
        filters,
        ..
    } = scaled_op
    else {
        return None;
    };
    if filters.is_empty() || ops.is_empty() {
        return None;
    }
    if !ops.iter().all(|child| matches!(child, Op::Draw(_))) {
        return None;
    }

    // Never stamp layers that are clipped by the camera viewport edge or exceed
    // the stamp size cap — large/cropped layers keep the exact cropped per-slot path.
    let visible_px = packet_pixel_bounds(*bounds, visible_bounds)?;
    let full_px = packet_pixel_bounds(*bounds, None)?;
    if full_px != visible_px || full_px[2] > MAX_STAMP_DIMENSION || full_px[3] > MAX_STAMP_DIMENSION
    {
        return None;
    }

    let Op::Draw(first_draw) = &ops[0] else {
        unreachable!();
    };
    let m0 = first_draw.transform.matrix;
    if ![m0.a, m0.b, m0.c, m0.d, m0.tx, m0.ty]
        .iter()
        .all(|v| v.is_finite())
    {
        return None;
    }

    let isotropic = is_isotropic_filter_chain(filters);
    let sx = m0.a.hypot(m0.b);
    let sy = m0.c.hypot(m0.d);

    let placement = if !isotropic || sx < 1e-4 {
        // Tier 1: exact translation factorization (valid for all 8 Flash filters).
        vatf::animation::AnimMatrix {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            tx: m0.tx,
            ty: m0.ty,
        }
    } else {
        let cos = m0.a / sx;
        let sin = m0.b / sx;
        let dot = m0.a * m0.c + m0.b * m0.d;
        let uniform_similarity = quality.reuse_scaled_stamps
            && sx > 0.05
            && sy > 0.05
            && (sx - sy).abs() <= 0.02 * sx.max(sy)
            && dot.abs() <= 0.02 * sx * sy;
        if uniform_similarity {
            // Tier 3 (opt-in): octave-quantized uniform scale + rotation + translation.
            // Renders at the next power-of-two scale (always downscaling on screen) so
            // expanding/shrinking particle clouds share a tiny set of crisp stamps.
            let s_canon = 2.0f32.powi(sx.log2().ceil() as i32).clamp(0.5, 4.0);
            let s_ratio = sx / s_canon;
            vatf::animation::AnimMatrix {
                a: cos * s_ratio,
                b: sin * s_ratio,
                c: -sin * s_ratio,
                d: cos * s_ratio,
                tx: m0.tx,
                ty: m0.ty,
            }
        } else {
            // Tier 2 (default): exact rigid rotation + translation for isotropic filters.
            vatf::animation::AnimMatrix {
                a: cos,
                b: sin,
                c: -sin,
                d: cos,
                tx: m0.tx,
                ty: m0.ty,
            }
        }
    };

    let inv_placement = anim_mat_inv(placement)?;

    let mut canonical_ops = Vec::with_capacity(ops.len());
    for (index, child) in ops.iter().enumerate() {
        let Op::Draw(draw) = child else {
            unreachable!();
        };
        let mut draw = draw.clone();
        let relative = anim_mat_mul(inv_placement, draw.transform.matrix);
        draw.transform.matrix = snap_canonical_matrix(relative, index == 0, isotropic);
        canonical_ops.push(Op::Draw(draw));
    }

    let [cx, cy, cw, ch] = op_bounds(&canonical_ops);
    let (fx, fy, fw, fh) = vatf::animation::filter_dest_rect(cx, cy, cw, ch, filters);
    let stamp_pixel_bounds = gpu::pixel_bounds([fx, fy, fw, fh]).ok()?;
    if stamp_pixel_bounds[2] <= 0.0
        || stamp_pixel_bounds[3] <= 0.0
        || stamp_pixel_bounds[2] > MAX_STAMP_DIMENSION
        || stamp_pixel_bounds[3] > MAX_STAMP_DIMENSION
    {
        return None;
    }

    let mut hasher = StampHasher::new();
    for child in &canonical_ops {
        let Op::Draw(draw) = child else {
            unreachable!();
        };
        draw.mesh.hash(&mut hasher);
        draw.texture.hash(&mut hasher);
        hasher.write_u32(draw.kind);
        hasher.write_u8(draw.blend as u8);
        for g in draw.gradient {
            hasher.write_q(g, 256.0);
        }
        hasher.write_u32(draw.ramp_v.to_bits());
        for col in [draw.uv.x_axis, draw.uv.y_axis, draw.uv.z_axis] {
            hasher.write_q(col.x, 1024.0);
            hasher.write_q(col.y, 1024.0);
        }
        let m = draw.transform.matrix;
        hasher.write_q(m.a, 256.0);
        hasher.write_q(m.b, 256.0);
        hasher.write_q(m.c, 256.0);
        hasher.write_q(m.d, 256.0);
        hasher.write_q(m.tx, 16.0);
        hasher.write_q(m.ty, 16.0);
        let c = draw.transform.color_transform;
        for v in [
            c.r_multiply,
            c.g_multiply,
            c.b_multiply,
            c.a_multiply,
            c.r_add,
            c.g_add,
            c.b_add,
            c.a_add,
        ] {
            hasher.write_q(v, 255.0);
        }
    }
    for filter in filters {
        hash_anim_filter(&mut hasher, filter);
    }

    let key = FilterStampKey {
        hash_a: hasher.a,
        hash_b: hasher.b,
        gpu_source_revision,
        samples: filter_samples,
        filter_quality: quality.cache_key(),
        size: [stamp_pixel_bounds[2] as u32, stamp_pixel_bounds[3] as u32],
        origin: [stamp_pixel_bounds[0] as i32, stamp_pixel_bounds[1] as i32],
        child_count: u16::try_from(canonical_ops.len()).unwrap_or(u16::MAX),
        filter_count: u16::try_from(filters.len()).unwrap_or(u16::MAX),
    };

    let canonical_op = Op::Layer {
        ops: canonical_ops,
        bounds: [fx, fy, fw, fh],
        filters: filters.clone(),
        blend: crate::vab_asset::VabBlendMode::Normal,
    };

    Some(FilterStampPlan {
        key,
        canonical_op,
        stamp_pixel_bounds,
        placement,
    })
}

#[allow(clippy::too_many_arguments)]
fn queue_vab_instance_data(
    extracted: Res<ExtractedVabInstances>,
    mut resources: ResMut<VabInstanceResources>,
    views: Query<
        (
            &Msaa,
            &ExtractedView,
            &ResolvedCompositingSpace,
            &RenderVisibleEntities,
        ),
        With<Camera2d>,
    >,
    mut filter_cache: ResMut<VabFilterOutputCache>,
    filter_cache_settings: Res<VabFilterCacheSettings>,
    filter_msaa: Res<VabFilterMsaa>,
    filter_quality: Res<gpu::VabFilterQuality>,
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
        let resolved_filter_msaa = filter_msaa.resolve(*msaa);
        let filter_samples = resolved_filter_msaa.samples();
        for (entity, _) in visible.iter_visible() {
            let Some(instance) = extracted.0.get(entity) else {
                continue;
            };
            let has_isolated = instance
                .ops
                .iter()
                .any(|op| matches!(op, Op::Layer { .. } | Op::Mask { .. }));
            let (scale, visible_bounds, scaled_ops, packet_stamps) = if has_isolated {
                let scale = view_pixel_scale(view, instance.global);
                let clip_from_world = view.clip_from_world.unwrap_or_else(|| {
                    view.clip_from_view * view.world_from_view.to_matrix().inverse()
                });
                let visible_bounds = visible_pixel_bounds(
                    clip_from_world * Mat4::from(instance.global) * VIEW_MATRIX,
                    scale,
                );
                let scaled_ops = scale_ops(&instance.ops, scale);
                let packet_stamps = scaled_ops
                    .iter()
                    .map(|scaled_op| {
                        build_filter_stamp_plan(
                            scaled_op,
                            visible_bounds,
                            instance.gpu_source_revision,
                            filter_samples,
                            *filter_quality,
                        )
                    })
                    .collect();
                (scale, visible_bounds, scaled_ops, packet_stamps)
            } else {
                (Vec2::ONE, None, Vec::new(), Vec::new())
            };
            resources.instances.insert(
                (view.retained_view_entity, *entity),
                VabInstanceResource {
                    ops: instance.ops.clone(),
                    scaled_ops,
                    packet_stamps,
                    visible_bounds,
                    global: instance.global,
                    scale,
                    view_msaa: *msaa,
                    filter_msaa: resolved_filter_msaa,
                    target_format: view.target_format,
                    srgb_compositing: camera.0 == Some(CompositingSpace::Srgb),
                    sample_generation: instance.sample_generation,
                    gpu_source_revision: instance.gpu_source_revision,
                },
            );
        }
    }
    diagnostics.vab_queue_cpu_ns = elapsed_ns(started);
}

fn pack_world_rows(world_from_local: Mat4) -> ([f32; 4], [f32; 4], [f32; 4]) {
    let x = world_from_local.x_axis;
    let y = world_from_local.y_axis;
    let w = world_from_local.w_axis;
    (
        [x.x, y.x, w.x, 0.0],
        [x.y, y.y, w.y, 0.0],
        [x.z, y.z, w.z, 0.0],
    )
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
        let base_world = Mat4::from(instance.global) * VIEW_MATRIX;
        for (packet_index, op) in instance.ops.iter().enumerate() {
            let uniform = match op {
                Op::Draw(draw) => {
                    let local = draw.transform.matrix;
                    let local = Mat4::from_cols_array(&[
                        local.a, local.b, 0.0, 0.0, local.c, local.d, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0,
                        local.tx, local.ty, 0.0, 1.0,
                    ]);
                    let (world_row0, world_row1, world_row2) = pack_world_rows(base_world * local);
                    let uv = draw.uv;
                    let color = draw.transform.color_transform;
                    VabInstanceUniform {
                        world_row0,
                        world_row1,
                        world_row2,
                        uv0: [uv.x_axis.x, uv.y_axis.x, uv.z_axis.x, draw.ramp_v],
                        uv1: [uv.x_axis.y, uv.y_axis.y, uv.z_axis.y, 0.0],
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
                Op::Layer { .. } | Op::Mask { .. } => {
                    let (world_row0, world_row1, world_row2) =
                        if let Some(Some(plan)) = instance.packet_stamps.get(packet_index) {
                            let [ox, oy, w, h] = plan.stamp_pixel_bounds;
                            let p = plan.placement;
                            let inv_sx = 1.0 / instance.scale.x;
                            let inv_sy = 1.0 / instance.scale.y;
                            let quad = Mat4::from_cols_array(&[
                                p.a * w * inv_sx,
                                p.b * w * inv_sy,
                                0.0,
                                0.0,
                                p.c * h * inv_sx,
                                p.d * h * inv_sy,
                                0.0,
                                0.0,
                                0.0,
                                0.0,
                                1.0,
                                0.0,
                                (p.a * ox + p.c * oy + p.tx) * inv_sx,
                                (p.b * ox + p.d * oy + p.ty) * inv_sy,
                                0.0,
                                1.0,
                            ]);
                            pack_world_rows(base_world * quad)
                        } else {
                            let bounds = match instance.scaled_ops.get(packet_index) {
                                Some(Op::Layer { bounds, .. } | Op::Mask { bounds, .. }) => *bounds,
                                _ => unreachable!("scaled instance operation changed kind"),
                            };
                            let Some(pixel_bounds) =
                                packet_pixel_bounds(bounds, instance.visible_bounds)
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
                            pack_world_rows(base_world * rect)
                        };
                    VabInstanceUniform {
                        world_row0,
                        world_row1,
                        world_row2,
                        uv0: [1.0, 0.0, 0.0, 0.0],
                        uv1: [0.0, 1.0, 0.0, 0.0],
                        multiply: Vec4::ONE.to_array(),
                        add: Vec4::ZERO.to_array(),
                        material: [0.0, 0.0, 0.0, 3.0],
                    }
                }
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
    (mut flash_gpu, filter_quality): (ResMut<gpu::FlashGpu>, Res<gpu::VabFilterQuality>),
    mut texture_groups: ResMut<VabTextureBindGroups>,
    mut filter_uniform_buffers: ResMut<VabFilterUniformBuffers>,
    mut filter_cache: ResMut<VabFilterOutputCache>,
    mut pipelines: ResMut<SpecializedMeshPipelines<VabInstancePipeline>>,
    mut quad_pipelines: ResMut<VabQuadPipelines>,
    mut diagnostics: ResMut<FlashRenderDiagnostics>,
) {
    let started = diagnostics.enabled.then(Instant::now);
    prepared.instances.clear();
    prepared.stamps.clear();
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
    let mut pending_stamps = HashMap::<FilterStampKey, usize>::new();
    texture_groups.0.retain(|id, _| images.get(*id).is_some());
    flash_gpu.retain_image_bind_groups(&images);
    let filter_quality_key = filter_quality.cache_key();
    'instances: for ((view, entity), instance) in &resources.instances {
        let view_samples = instance.view_msaa.samples();
        let filter_samples = instance.filter_msaa.samples();
        let scale = instance.scale;
        let mut packets = Vec::new();
        for (packet_index, op) in instance.ops.iter().enumerate() {
            match op {
                Op::Draw(draw) => {
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
                        && packet.blend == draw.blend
                        && packet.instances.end == draw_instance
                    {
                        packet.instances.end += 1;
                    } else {
                        let packet_pipeline = meshes.get(draw.mesh).and_then(|mesh| {
                            let mesh_key = Mesh2dPipelineKey::from_msaa_samples(view_samples)
                                | Mesh2dPipelineKey::from_target_format(instance.target_format)
                                | Mesh2dPipelineKey::from(mesh.key_bits.bits());
                            let mesh_key = if instance.srgb_compositing {
                                mesh_key | Mesh2dPipelineKey::SRGB_COMPOSITING
                            } else {
                                mesh_key
                            };
                            pipelines
                                .specialize(
                                    &pipeline_cache,
                                    &pipeline,
                                    VabInstancePipelineKey {
                                        mesh: mesh_key,
                                        blend: draw.blend,
                                    },
                                    &mesh.layout,
                                )
                                .ok()
                        });
                        packets.push(PreparedVabPacket::Draw(PreparedDrawPacket {
                            mesh: draw.mesh,
                            texture_id: draw.texture,
                            texture,
                            blend: draw.blend,
                            instances: draw_instance..draw_instance + 1,
                            pipeline: packet_pipeline,
                        }));
                    }
                }
                Op::Layer { .. } | Op::Mask { .. } => {
                    let Some(&draw_instance) = buffers.indices.get(&(*view, *entity, packet_index))
                    else {
                        continue;
                    };
                    let blend = match op {
                        Op::Layer { blend, .. } => *blend,
                        Op::Mask { .. } => crate::vab_asset::VabBlendMode::Normal,
                        Op::Draw(_) => unreachable!(),
                    };
                    let packet_pipeline = Some(
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
                    );
                    let cache_slot = FilterCacheSlot {
                        view: *view,
                        entity: *entity,
                        packet: packet_index,
                    };
                    if let Some(Some(plan)) = instance.packet_stamps.get(packet_index) {
                        let (texture, pending_stamp) =
                            if let Some(texture) = filter_cache.get_stamp(plan.key) {
                                diagnostics.vab_filter_cache_hits += 1;
                                (Some(texture), None)
                            } else if let Some(&stamp_idx) = pending_stamps.get(&plan.key) {
                                // Deduplicate repeated identical symbols within the same frame.
                                diagnostics.vab_filter_cache_hits += 1;
                                (None, Some(stamp_idx))
                            } else {
                                let stamp_idx = prepared.stamps.len();
                                if stamp_idx >= filter_uniform_buffers.stamps.len() {
                                    filter_uniform_buffers
                                        .stamps
                                        .resize_with(stamp_idx + 1, Default::default);
                                }
                                let uniform_buffer = &mut filter_uniform_buffers.stamps[stamp_idx];
                                let result = gpu::prepare_layer(
                                    std::slice::from_ref(&plan.canonical_op),
                                    plan.stamp_pixel_bounds,
                                    filter_samples,
                                    *filter_quality,
                                    &mut flash_gpu,
                                    &pipeline_cache,
                                    &render_device,
                                    &render_queue,
                                    &meshes,
                                    &images,
                                    uniform_buffer,
                                );
                                let (layer, allocated) = match result {
                                    Ok(prepared_layer) => prepared_layer,
                                    Err(error) => {
                                        let message = error.to_string();
                                        if message.starts_with("pending") {
                                            diagnostics.pending_views += 1;
                                        } else {
                                            diagnostics.errors.push(message);
                                        }
                                        continue 'instances;
                                    }
                                };
                                if allocated {
                                    diagnostics.filter_uniform_buffer_allocations += 1;
                                } else {
                                    diagnostics.filter_uniform_buffer_reuses += 1;
                                }
                                diagnostics.vab_filter_cache_misses += 1;
                                prepared.stamps.push(PreparedFilterStamp {
                                    key: plan.key,
                                    layer,
                                    filter_msaa: instance.filter_msaa,
                                    owner_entity: *entity,
                                    owner_packet: packet_index,
                                    pixel_bounds: plan.stamp_pixel_bounds,
                                    target: None,
                                    texture: None,
                                });
                                pending_stamps.insert(plan.key, stamp_idx);
                                (None, Some(stamp_idx))
                            };
                        // Merge adjacent instances that share the same cached/pending
                        // filter stamp and blend pipeline into one instanced quad draw.
                        if let Some(PreparedVabPacket::Layer(prev)) = packets.last_mut()
                            && prev.stamp_id == Some(plan.key)
                            && prev.pipeline == packet_pipeline
                            && prev.instances.end == draw_instance
                        {
                            prev.instances.end += 1;
                        } else {
                            packets.push(PreparedVabPacket::Layer(Box::new(PreparedLayerPacket {
                                layer: None,
                                target: None,
                                texture,
                                stamp_id: Some(plan.key),
                                pending_stamp,
                                cache_slot,
                                cache_content: FilterCacheContent {
                                    sample_generation: instance.sample_generation,
                                    gpu_source_revision: instance.gpu_source_revision,
                                    scale: [scale.x.to_bits(), scale.y.to_bits()],
                                    samples: filter_samples,
                                    size: [
                                        plan.stamp_pixel_bounds[2] as u32,
                                        plan.stamp_pixel_bounds[3] as u32,
                                    ],
                                    origin: [
                                        plan.stamp_pixel_bounds[0].to_bits(),
                                        plan.stamp_pixel_bounds[1].to_bits(),
                                    ],
                                    filter_quality: filter_quality_key,
                                },
                                pixel_bounds: plan.stamp_pixel_bounds,
                                instances: draw_instance..draw_instance + 1,
                                pipeline: packet_pipeline,
                            })));
                        }
                        continue;
                    }
                    let scaled_op = instance
                        .scaled_ops
                        .get(packet_index)
                        .expect("scaled instance operation present for isolated layer");
                    let bounds = match scaled_op {
                        Op::Layer { bounds, .. } | Op::Mask { bounds, .. } => *bounds,
                        Op::Draw(_) => unreachable!("scaled instance operation changed kind"),
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
                    let Some(pixel_bounds) = packet_pixel_bounds(bounds, instance.visible_bounds)
                    else {
                        continue;
                    };
                    let cache_content = FilterCacheContent {
                        sample_generation: instance.sample_generation,
                        gpu_source_revision: instance.gpu_source_revision,
                        scale: [scale.x.to_bits(), scale.y.to_bits()],
                        samples: filter_samples,
                        size: [pixel_bounds[2] as u32, pixel_bounds[3] as u32],
                        origin: [pixel_bounds[0].to_bits(), pixel_bounds[1].to_bits()],
                        filter_quality: filter_quality_key,
                    };
                    live_uniform_slots.insert(cache_slot);
                    let (layer, texture) =
                        if let Some(texture) = filter_cache.get_slot(cache_slot, cache_content) {
                            diagnostics.vab_filter_cache_hits += 1;
                            (None, Some(texture))
                        } else {
                            let uniform_buffer =
                                filter_uniform_buffers.slots.entry(cache_slot).or_default();
                            let result = gpu::prepare_layer(
                                isolated_ops,
                                pixel_bounds,
                                filter_samples,
                                *filter_quality,
                                &mut flash_gpu,
                                &pipeline_cache,
                                &render_device,
                                &render_queue,
                                &meshes,
                                &images,
                                uniform_buffer,
                            );
                            let (layer, allocated) = match result {
                                Ok(prepared) => prepared,
                                Err(error) => {
                                    let message = error.to_string();
                                    if message.starts_with("pending") {
                                        diagnostics.pending_views += 1;
                                    } else {
                                        diagnostics.errors.push(message);
                                    }
                                    continue 'instances;
                                }
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
                        stamp_id: None,
                        pending_stamp: None,
                        cache_slot,
                        cache_content,
                        pixel_bounds,
                        instances: draw_instance..draw_instance + 1,
                        pipeline: packet_pipeline,
                    })));
                }
            }
        }
        // Preserve entity-level ordering while any screen pipeline compiles.
        if packets.iter().any(|packet| {
            let id = match packet {
                PreparedVabPacket::Draw(packet) => packet.pipeline,
                PreparedVabPacket::Layer(packet) => packet.pipeline,
            };
            id.and_then(|id| pipeline_cache.get_render_pipeline(id))
                .is_none()
        }) {
            continue;
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
        .slots
        .retain(|slot, _| live_uniform_slots.contains(slot));
    let used_stamps = prepared.stamps.len();
    if filter_uniform_buffers.stamps.len() > 256
        && filter_uniform_buffers.stamps.len() > used_stamps.saturating_mul(4)
    {
        filter_uniform_buffers.stamps.truncate(used_stamps.max(64));
    }
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
    let PreparedVabInstances {
        instances, stamps, ..
    } = &mut *prepared;
    let scene_texture_layout = pipeline_cache.get_bind_group_layout(&pipeline.texture_layout);
    for stamp in stamps.iter_mut() {
        let (passes, pixels) = if diagnostics.enabled {
            gpu::filter_workload(&stamp.layer)
        } else {
            (0, 0)
        };
        pool.set_allocation_owner(Some(TextureAllocationOwner {
            render_frame: frame,
            app_frame: app_frame.0,
            entity: stamp.owner_entity,
            packet: stamp.owner_packet,
            packet_bounds: stamp.pixel_bounds,
        }));
        let target = gpu::render_prepared_layer(
            &stamp.layer,
            Srgba::NONE,
            &mut flash_gpu,
            &render_device,
            &mut pool,
            &stamp.filter_msaa,
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
        let texture = target
            .main_scene_bind_group(&render_device, &scene_texture_layout, &pipeline.sampler)
            .clone();
        stamp.texture = Some(texture.clone());
        let bytes = target.main_bytes();
        let cache_key = FilterCacheKey::Stamp(stamp.key);
        if filter_cache.reserve(cache_key, bytes, filter_cache_settings.max_bytes) {
            let persistent = target.into_persistent_main();
            filter_cache.insert(cache_key, None, persistent, texture, bytes);
        } else {
            stamp.target = Some(target);
        }
    }
    for instance in instances.values_mut() {
        for packet in &mut instance.packets {
            let PreparedVabPacket::Layer(packet) = packet else {
                continue;
            };
            let count = packet.instances.len();
            diagnostics.vab_filter_layers += count;
            diagnostics.vab_filter_output_pixels =
                diagnostics.vab_filter_output_pixels.saturating_add(
                    (packet.pixel_bounds[2] as u64)
                        .saturating_mul(packet.pixel_bounds[3] as u64)
                        .saturating_mul(count as u64),
                );
            if let Some(stamp_idx) = packet.pending_stamp {
                packet.texture = stamps[stamp_idx].texture.clone();
                continue;
            }
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
            let texture = target
                .main_scene_bind_group(&render_device, &scene_texture_layout, &pipeline.sampler)
                .clone();
            packet.texture = Some(texture.clone());
            let bytes = target.main_bytes();
            let cache_key = FilterCacheKey::Slot(packet.cache_slot);
            if filter_cache.reserve(cache_key, bytes, filter_cache_settings.max_bytes) {
                let persistent = target.into_persistent_main();
                filter_cache.insert(
                    cache_key,
                    Some(packet.cache_content),
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
        &ResolvedCompositingSpace,
        &RenderVisibleEntities,
        &Msaa,
    )>,
    mut diagnostics: ResMut<FlashRenderDiagnostics>,
) {
    diagnostics.vab_instances_queued = 0;
    let draw_function = draw_functions.read().id::<DrawVabInstance>();
    for (view, camera, visible, msaa) in &views {
        let srgb_compositing = camera.0 == Some(CompositingSpace::Srgb);
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
                        | Mesh2dPipelineKey::from(mesh.key_bits.bits());
                    let mesh_key = if srgb_compositing {
                        mesh_key | Mesh2dPipelineKey::SRGB_COMPOSITING
                    } else {
                        mesh_key
                    };
                    pipelines
                        .specialize(
                            &pipeline_cache,
                            &pipeline,
                            VabInstancePipelineKey {
                                mesh: mesh_key,
                                blend: draw.blend,
                            },
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
    prepared.stamps.clear();
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

    fn symbol_layer(
        placement: vatf::animation::AnimMatrix,
        filters: Vec<vatf::animation::AnimFilter>,
    ) -> Op {
        use super::super::extract::Draw;
        use vatf::animation::{AnimColorTransform, AnimMatrix, AnimTransform};

        let m_base = placement;
        let m_accent = anim_mat_mul(
            placement,
            AnimMatrix {
                a: 1.0,
                b: 0.0,
                c: 0.0,
                d: 1.0,
                tx: 6.0,
                ty: -4.0,
            },
        );
        let draw = |mesh_index: u32, matrix: AnimMatrix| {
            Op::Draw(Draw {
                mesh: AssetId::Uuid {
                    uuid: bevy::asset::uuid::Uuid::from_u128(u128::from(mesh_index) + 1),
                },
                local_bounds: [-8.0, -8.0, 8.0, 8.0],
                texture: None,
                transform: AnimTransform {
                    matrix,
                    color_transform: AnimColorTransform::IDENTITY,
                },
                uv: Mat4::IDENTITY,
                kind: 0,
                gradient: [0.0; 3],
                ramp_v: 0.0,
                blend: crate::vab_asset::VabBlendMode::Normal,
            })
        };
        let ops = vec![draw(0, m_base), draw(1, m_accent)];
        let [x, y, w, h] = op_bounds(&ops);
        let (fx, fy, fw, fh) = vatf::animation::filter_dest_rect(x, y, w, h, &filters);
        Op::Layer {
            ops,
            bounds: [fx, fy, fw, fh],
            filters,
            blend: crate::vab_asset::VabBlendMode::Normal,
        }
    }

    #[test]
    fn stamp_canonicalizes_multi_shape_translation_and_isotropic_rotation() {
        use vatf::animation::{AnimFilter, AnimGlowFilter, AnimMatrix};

        let glow = vec![AnimFilter::GlowFilter(AnimGlowFilter {
            flags: 0x21,
            color_r: 0,
            color_g: 128,
            color_b: 255,
            color_a: 255,
            blur_x: 9 * 65536,
            blur_y: 9 * 65536,
            strength: 256,
            num_passes: 1,
        })];
        let a = symbol_layer(
            AnimMatrix {
                a: 1.0,
                b: 0.0,
                c: 0.0,
                d: 1.0,
                tx: 10.25,
                ty: 20.5,
            },
            glow.clone(),
        );
        let angle = 47.0f32.to_radians();
        let (sin, cos) = angle.sin_cos();
        let b = symbol_layer(
            AnimMatrix {
                a: cos,
                b: sin,
                c: -sin,
                d: cos,
                tx: 150.75,
                ty: -80.25,
            },
            glow,
        );
        let plan_a = build_filter_stamp_plan(&a, None, 1, 4, gpu::VabFilterQuality::full())
            .expect("stamp A");
        let plan_b = build_filter_stamp_plan(&b, None, 1, 4, gpu::VabFilterQuality::full())
            .expect("stamp B");
        assert_eq!(plan_a.key, plan_b.key);
        assert_eq!(plan_a.stamp_pixel_bounds, plan_b.stamp_pixel_bounds);
    }

    #[test]
    fn stamp_keeps_directional_drop_shadow_translation_only() {
        use vatf::animation::{AnimDropShadowFilter, AnimFilter, AnimMatrix};

        let shadow = vec![AnimFilter::DropShadowFilter(AnimDropShadowFilter {
            flags: 0x21,
            color_r: 0,
            color_g: 0,
            color_b: 0,
            color_a: 255,
            blur_x: 5 * 65536,
            blur_y: 5 * 65536,
            angle: 0,
            distance: 6 * 65536,
            strength: 256,
            num_passes: 1,
        })];
        let a = symbol_layer(
            AnimMatrix {
                tx: 10.0,
                ty: 20.0,
                ..AnimMatrix::IDENTITY
            },
            shadow.clone(),
        );
        let b_trans = symbol_layer(
            AnimMatrix {
                tx: 90.0,
                ty: -30.0,
                ..AnimMatrix::IDENTITY
            },
            shadow.clone(),
        );
        let b_rot = symbol_layer(
            AnimMatrix {
                a: 0.0,
                b: 1.0,
                c: -1.0,
                d: 0.0,
                tx: 90.0,
                ty: -30.0,
            },
            shadow,
        );
        let pa = build_filter_stamp_plan(&a, None, 1, 1, gpu::VabFilterQuality::full()).unwrap();
        let pt =
            build_filter_stamp_plan(&b_trans, None, 1, 1, gpu::VabFilterQuality::full()).unwrap();
        let pr =
            build_filter_stamp_plan(&b_rot, None, 1, 1, gpu::VabFilterQuality::full()).unwrap();
        assert_eq!(pa.key, pt.key, "pure translation must reuse shadow stamp");
        assert_ne!(
            pa.key, pr.key,
            "directional shadow must not rotate stamp key"
        );
    }

    #[test]
    fn stamp_octave_scale_reuse_only_when_enabled() {
        use vatf::animation::{AnimBlurFilter, AnimFilter, AnimMatrix};

        let blur = vec![AnimFilter::BlurFilter(AnimBlurFilter {
            blur_x: 9 * 65536,
            blur_y: 9 * 65536,
            num_passes: 1,
        })];
        let full_scale = symbol_layer(AnimMatrix::IDENTITY, blur.clone());
        let scaled = symbol_layer(
            AnimMatrix {
                a: 0.75,
                b: 0.0,
                c: 0.0,
                d: 0.75,
                tx: 40.0,
                ty: 10.0,
            },
            blur,
        );
        let exact_q = gpu::VabFilterQuality::full();
        let p0 = build_filter_stamp_plan(&full_scale, None, 1, 1, exact_q).unwrap();
        let p1 = build_filter_stamp_plan(&scaled, None, 1, 1, exact_q).unwrap();
        assert_ne!(
            p0.key, p1.key,
            "default exact quality must keep distinct scale stamps"
        );

        let particle_q = gpu::VabFilterQuality::full().with_scaled_stamps(true);
        let q0 = build_filter_stamp_plan(&full_scale, None, 1, 1, particle_q).unwrap();
        let q1 = build_filter_stamp_plan(&scaled, None, 1, 1, particle_q).unwrap();
        assert_eq!(
            q0.key, q1.key,
            "reuse_scaled_stamps should unify 0.75x and 1.0x into the 1.0x octave stamp"
        );
    }
}
