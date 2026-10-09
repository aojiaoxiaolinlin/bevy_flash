use super::pool::{Lease, Pool};
use bevy::{
    color::{LinearRgba, Srgba},
    ecs::entity::Entity,
    ecs::resource::Resource,
    platform::collections::HashMap,
    render::{
        extract_resource::ExtractResource,
        render_resource::{
            LoadOp, Operations, RenderPassColorAttachment, StoreOp, TextureDescriptor,
            TextureDimension, TextureUsages, TextureViewDescriptor,
        },
        renderer::RenderDevice,
        texture::CachedTexture,
        view::{Msaa, PostProcessWrite},
    },
};

const LARGE_TEXTURE_LOG_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug)]
pub(super) enum TexturePurpose {
    Root,
    Layer,
    Mask,
    MaskContent,
    PingPong,
    FilterScratch,
}

#[derive(Clone, Copy)]
pub(super) struct TextureAllocationSite {
    pub purpose: TexturePurpose,
    /// Unrounded target rectangle in output pixels.
    pub bounds: [f32; 4],
}

#[derive(Clone, Copy)]
pub(super) struct TextureAllocationOwner {
    pub render_frame: u64,
    pub app_frame: u32,
    pub entity: Entity,
    pub packet: usize,
    /// Unrounded bounds of the top-level isolated packet.
    pub packet_bounds: [f32; 4],
}

/// Controls textures retained by the shared transient render-target pool.
/// Live leases may temporarily exceed the budget; only idle textures are evictable.
#[derive(Resource, ExtractResource, Clone, Debug)]
#[extract_app(bevy::render::RenderApp)]
pub struct TransientTexturePoolSettings {
    pub max_resident_bytes: u64,
    pub max_unused_frames: u64,
    /// Emit details for newly created targets of at least 64 MiB.
    pub trace_large_allocations: bool,
}

impl Default for TransientTexturePoolSettings {
    fn default() -> Self {
        Self {
            max_resident_bytes: 64 * 1024 * 1024,
            max_unused_frames: 120,
            trace_large_allocations: false,
        }
    }
}

/// A local render operation owns its ping-pong state; it is never shared with another target.
pub struct TextureTarget {
    texture_a: Lease<CachedTexture>,
    texture_b: Option<Lease<CachedTexture>>,
    pub resolve_target: Option<Lease<CachedTexture>>,
    main_is_b: bool,
}

/// A pool texture kept across render frames by a bounded higher-level cache.
/// Dropping it immediately returns the texture to its descriptor bucket.
pub(super) struct PersistentTexture {
    _lease: Lease<CachedTexture>,
}

impl TextureTarget {
    /// Borrow the second single-sample surface only when post-processing
    /// starts, after nested geometry has finished using its own targets.
    pub(super) fn enable_ping_pong(&mut self, second: Lease<CachedTexture>) {
        assert!(self.texture_b.is_none() && !self.main_is_b);
        self.texture_b = Some(second);
    }

    pub fn main_view(&self) -> &bevy::render::render_resource::TextureView {
        &self.main().default_view
    }

    /// Rendering into this target has finished. The resolved single-sample
    /// image remains live for filters/compositing, while the multisample
    /// attachment can be borrowed by a later target in the same frame.
    pub(super) fn release_msaa_attachment(&mut self) {
        self.resolve_target.take();
    }

    pub(super) fn other_view(&self) -> &bevy::render::render_resource::TextureView {
        if self.main_is_b {
            &self.texture_a.default_view
        } else {
            &self
                .texture_b
                .as_ref()
                .expect("filtered target")
                .default_view
        }
    }

    pub(super) fn flip_main(&mut self) {
        self.main_is_b = !self.main_is_b;
    }

    pub(super) fn replace_main_with(&mut self, scratch: &mut Lease<CachedTexture>) {
        if self.main_is_b {
            std::mem::swap(self.texture_b.as_mut().expect("filtered target"), scratch);
        } else {
            std::mem::swap(&mut self.texture_a, scratch);
        }
    }

    pub fn load_color_attachment(&self) -> RenderPassColorAttachment<'_> {
        let mut attachment = self.get_color_attachment(Srgba::NONE);
        attachment.ops.load = LoadOp::Load;
        attachment
    }
    fn main(&self) -> &CachedTexture {
        if self.main_is_b {
            self.texture_b.as_ref().expect("filtered target")
        } else {
            &self.texture_a
        }
    }

    pub fn get_color_attachment(&self, clear: Srgba) -> RenderPassColorAttachment<'_> {
        let main = self.main();
        RenderPassColorAttachment {
            view: self
                .resolve_target
                .as_ref()
                .map_or(&main.default_view, |t| &t.default_view),
            depth_slice: None,
            resolve_target: self.resolve_target.as_ref().map(|_| &*main.default_view),
            ops: Operations {
                // Private Flash targets are Rgba8Unorm legacy working
                // surfaces, so clear values stay in encoded sRGB just like
                // Ruffle's framebuffer.
                load: LoadOp::Clear(
                    LinearRgba::new(
                        clear.red * clear.alpha,
                        clear.green * clear.alpha,
                        clear.blue * clear.alpha,
                        clear.alpha,
                    )
                    .into(),
                ),
                store: StoreOp::Store,
            },
        }
    }

    #[allow(dead_code)]
    pub fn post_process_write(&mut self) -> PostProcessWrite<'_> {
        let (source, destination) = if self.main_is_b {
            (
                self.texture_b.as_ref().expect("filtered target"),
                &self.texture_a,
            )
        } else {
            (
                &self.texture_a,
                self.texture_b.as_ref().expect("filtered target"),
            )
        };
        self.main_is_b = !self.main_is_b;
        PostProcessWrite {
            source: &source.default_view,
            source_texture: &source.texture,
            destination: &destination.default_view,
            destination_texture: &destination.texture,
        }
    }

    pub(super) fn into_persistent_main(self) -> PersistentTexture {
        let Self {
            texture_a,
            texture_b,
            resolve_target: _,
            main_is_b,
        } = self;
        PersistentTexture {
            _lease: if main_is_b {
                texture_b.expect("filtered target")
            } else {
                texture_a
            },
        }
    }
}

struct Bucket {
    pool: Pool<CachedTexture>,
    last_used: u64,
    total: usize,
}

#[derive(Resource, Default)]
pub struct FrameInternalTextureCache {
    textures: HashMap<TextureDescriptor<'static>, Bucket>,
    allocation_owner: Option<TextureAllocationOwner>,
    trace_large_allocations: bool,
    frame: u64,
    pub allocations: u64,
    pub reuses: u64,
    /// New texture for a descriptor that had no resident texture yet.
    pub first_in_bucket_allocations: u64,
    /// New texture because all resident textures of this descriptor were borrowed.
    pub exhausted_bucket_allocations: u64,
    /// Descriptor-derived payload of textures created during this frame.
    pub allocated_bytes: u64,
    pub live_textures: usize,
    pub peak_live_textures: usize,
    pub live_bytes: u64,
    pub peak_live_bytes: u64,
    /// Exact number of GPU textures still owned by all retained buckets.
    pub pooled_textures: usize,
    /// Logical texture payload including blocks, mips, layers and MSAA samples.
    /// wgpu does not expose backend heap padding or driver metadata.
    pub pooled_texture_bytes: u64,
    pub pooled_idle_textures: usize,
    pub pooled_idle_bytes: u64,
    pub bucket_count: usize,
    pub largest_bucket_textures: usize,
    pub largest_bucket_bytes: u64,
}

impl FrameInternalTextureCache {
    pub(super) fn set_allocation_owner(&mut self, owner: Option<TextureAllocationOwner>) {
        self.allocation_owner = owner;
    }

    pub fn begin_frame(&mut self, settings: &TransientTexturePoolSettings) {
        self.frame = self.frame.wrapping_add(1);
        self.trace_large_allocations = settings.trace_large_allocations;
        self.allocation_owner = None;
        self.textures.retain(|_, bucket| {
            let live = bucket.total.saturating_sub(bucket.pool.available_len());
            live > 0 || self.frame.wrapping_sub(bucket.last_used) < settings.max_unused_frames
        });
        self.enforce_resident_budget(settings.max_resident_bytes);
        self.allocations = 0;
        self.reuses = 0;
        self.first_in_bucket_allocations = 0;
        self.exhausted_bucket_allocations = 0;
        self.allocated_bytes = 0;
        self.update_live_metrics();
        self.peak_live_textures = self.live_textures;
        self.peak_live_bytes = self.live_bytes;
    }

    /// Release idle targets after render commands have released their leases.
    /// Live filter outputs remain protected even when they exceed the budget.
    pub fn end_frame(&mut self, settings: &TransientTexturePoolSettings) {
        self.enforce_resident_budget(settings.max_resident_bytes);
        self.update_live_metrics();
    }

    fn enforce_resident_budget(&mut self, max_bytes: u64) {
        let mut resident_bytes = self
            .textures
            .iter()
            .map(|(descriptor, bucket)| {
                texture_bytes(descriptor).saturating_mul(bucket.total as u64)
            })
            .fold(0u64, u64::saturating_add);
        if resident_bytes <= max_bytes {
            return;
        }
        let mut oldest = self
            .textures
            .iter()
            .map(|(descriptor, bucket)| (bucket.last_used, descriptor.clone()))
            .collect::<Vec<_>>();
        oldest.sort_unstable_by_key(|(last_used, _)| *last_used);
        for (_, descriptor) in oldest {
            let Some(bucket) = self.textures.get_mut(&descriptor) else {
                continue;
            };
            let removed = bucket.pool.clear_available();
            bucket.total = bucket.total.saturating_sub(removed);
            resident_bytes = resident_bytes
                .saturating_sub(texture_bytes(&descriptor).saturating_mul(removed as u64));
            if bucket.total == 0 {
                self.textures.remove(&descriptor);
            }
            if resident_bytes <= max_bytes {
                break;
            }
        }
    }

    pub(super) fn acquire(
        &mut self,
        device: &RenderDevice,
        mut descriptor: TextureDescriptor<'static>,
        site: TextureAllocationSite,
    ) -> Lease<CachedTexture> {
        descriptor.label = None;
        let bucket = self
            .textures
            .entry(descriptor.clone())
            .or_insert_with(|| Bucket {
                pool: Pool::default(),
                last_used: self.frame,
                total: 0,
            });
        bucket.last_used = self.frame;
        let (lease, reused) = bucket.pool.take(|| {
            let texture = device.create_texture(&descriptor);
            let default_view = texture.create_view(&TextureViewDescriptor::default());
            CachedTexture {
                texture,
                default_view,
            }
        });
        if reused {
            self.reuses += 1;
        } else {
            let first_in_bucket = bucket.total == 0;
            self.allocations += 1;
            if first_in_bucket {
                self.first_in_bucket_allocations += 1;
            } else {
                self.exhausted_bucket_allocations += 1;
            }
            let bytes = texture_bytes(&descriptor);
            self.allocated_bytes = self.allocated_bytes.saturating_add(bytes);
            if self.trace_large_allocations
                && bytes >= LARGE_TEXTURE_LOG_BYTES
                && let Some(owner) = self.allocation_owner
            {
                bevy::log::info!(
                    "VAB large texture allocation: render_frame={} app_frame={} entity={:?} packet={} purpose={:?} allocation={}x{} samples={} bytes={:.1} MiB bucket={} target_bounds={:?} packet_bounds={:?}",
                    owner.render_frame,
                    owner.app_frame,
                    owner.entity,
                    owner.packet,
                    site.purpose,
                    descriptor.size.width,
                    descriptor.size.height,
                    descriptor.sample_count,
                    bytes as f64 / 1024.0 / 1024.0,
                    if first_in_bucket {
                        "first"
                    } else {
                        "exhausted"
                    },
                    site.bounds,
                    owner.packet_bounds,
                );
            }
            bucket.total += 1;
        }
        self.update_live_metrics();
        lease
    }

    pub fn update_live_metrics(&mut self) {
        let mut live_textures = 0usize;
        let mut live_bytes = 0u64;
        let mut pooled_textures = 0usize;
        let mut pooled_bytes = 0u64;
        let mut idle_textures = 0usize;
        let mut idle_bytes = 0u64;
        let mut largest_bucket_textures = 0usize;
        let mut largest_bucket_bytes = 0u64;
        for (descriptor, bucket) in &self.textures {
            let idle = bucket.pool.available_len();
            let live = bucket.total.saturating_sub(idle);
            let bytes_per_texture = texture_bytes(descriptor);
            let bucket_bytes = bytes_per_texture.saturating_mul(bucket.total as u64);
            live_textures += live;
            live_bytes = live_bytes.saturating_add(bytes_per_texture.saturating_mul(live as u64));
            pooled_textures += bucket.total;
            pooled_bytes = pooled_bytes.saturating_add(bucket_bytes);
            idle_textures += idle;
            idle_bytes = idle_bytes.saturating_add(bytes_per_texture.saturating_mul(idle as u64));
            largest_bucket_textures = largest_bucket_textures.max(bucket.total);
            largest_bucket_bytes = largest_bucket_bytes.max(bucket_bytes);
        }
        self.live_textures = live_textures;
        self.live_bytes = live_bytes;
        self.peak_live_textures = self.peak_live_textures.max(live_textures);
        self.peak_live_bytes = self.peak_live_bytes.max(live_bytes);
        self.pooled_textures = pooled_textures;
        self.pooled_texture_bytes = pooled_bytes;
        self.pooled_idle_textures = idle_textures;
        self.pooled_idle_bytes = idle_bytes;
        self.bucket_count = self.textures.len();
        self.largest_bucket_textures = largest_bucket_textures;
        self.largest_bucket_bytes = largest_bucket_bytes;
    }

    pub fn get(
        &mut self,
        device: &RenderDevice,
        mut descriptor: TextureDescriptor<'static>,
        msaa: &Msaa,
        site: TextureAllocationSite,
    ) -> TextureTarget {
        descriptor.sample_count = 1;
        let texture_a = self.acquire(device, descriptor.clone(), site);
        let resolve_target = (msaa.samples() > 1).then(|| {
            self.acquire(
                device,
                TextureDescriptor {
                    sample_count: msaa.samples(),
                    usage: TextureUsages::RENDER_ATTACHMENT,
                    mip_level_count: 1,
                    ..descriptor
                },
                site,
            )
        });
        TextureTarget {
            texture_a,
            texture_b: None,
            resolve_target,
            main_is_b: false,
        }
    }
}

fn texture_bytes(descriptor: &TextureDescriptor<'_>) -> u64 {
    let (block_width, block_height) = descriptor.format.block_dimensions();
    let block_bytes = descriptor.format.block_copy_size(None).unwrap_or(4) as u64;
    let mut total = 0u64;
    for mip in 0..descriptor.mip_level_count {
        let width = (descriptor.size.width >> mip).max(1);
        let height = (descriptor.size.height >> mip).max(1);
        let depth = if descriptor.dimension == TextureDimension::D3 {
            (descriptor.size.depth_or_array_layers >> mip).max(1)
        } else {
            descriptor.size.depth_or_array_layers
        };
        let blocks_x = width.div_ceil(block_width);
        let blocks_y = height.div_ceil(block_height);
        total = total.saturating_add(
            u64::from(blocks_x)
                .saturating_mul(u64::from(blocks_y))
                .saturating_mul(u64::from(depth))
                .saturating_mul(block_bytes),
        );
    }
    total.saturating_mul(u64::from(descriptor.sample_count))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::render::render_resource::{Extent3d, TextureFormat};

    #[test]
    fn texture_byte_estimate_includes_mips_layers_and_samples() {
        let descriptor = TextureDescriptor {
            label: None,
            size: Extent3d {
                width: 4,
                height: 4,
                depth_or_array_layers: 2,
            },
            mip_level_count: 3,
            sample_count: 4,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        };
        assert_eq!(texture_bytes(&descriptor), (64 + 16 + 4) * 2 * 4);
    }
}
