//! Raster cache keys, ownership, completion and idle eviction.
use super::{VabGraphic, VabImageFit, VabUiCacheSettings};
use crate::render::RasterOnce;
use crate::{
    material::{BitmapMaterial, GradientMaterial},
    vab_asset::VabAsset,
};
use bevy::{asset::AssetId, ecs::system::SystemParam, prelude::*};
use std::collections::{HashMap, HashSet};

#[derive(Component, Default)]
pub(super) struct PendingRaster(pub(super) Option<PendingImage>);

/// The last requested frame can finish while playback advances independently.
pub(super) struct PendingImage {
    pub(super) key: RasterKey,
    pub(super) image: Handle<Image>,
    pub(super) done: RasterOnce,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct RasterKey {
    pub(super) graphic: AssetId<VabGraphic>,
    pub(super) size: UVec2,
    pub(super) samples: u32,
    pub(super) fit: VabImageFit,
    pub(super) frame: usize,
}
impl RasterKey {
    /// Frame changes may finish in flight; target/source changes replace the request.
    pub(super) fn same_target(&self, other: &Self) -> bool {
        self.graphic == other.graphic
            && self.size == other.size
            && self.samples == other.samples
            && self.fit == other.fit
    }
}
pub(super) struct RasterDependencies {
    pub(super) asset: AssetId<VabAsset>,
    pub(super) meshes: Vec<AssetId<Mesh>>,
    pub(super) gradients: Vec<AssetId<GradientMaterial>>,
    pub(super) bitmaps: Vec<AssetId<BitmapMaterial>>,
    pub(super) images: Vec<AssetId<Image>>,
}

impl RasterDependencies {
    pub(super) fn collect(
        asset_id: AssetId<VabAsset>,
        asset: &VabAsset,
        gradients: &Assets<GradientMaterial>,
        bitmaps: &Assets<BitmapMaterial>,
    ) -> Self {
        let mut mesh_ids = Vec::new();
        let mut gradient_ids = Vec::new();
        let mut bitmap_ids = Vec::new();
        let mut image_ids = Vec::new();
        for group in &asset.render_meshes {
            let mesh_id = group.mesh.id();
            if !mesh_ids.contains(&mesh_id) {
                mesh_ids.push(mesh_id);
            }
            match &group.material {
                crate::vab_asset::MeshMaterial::Color => {}
                crate::vab_asset::MeshMaterial::Gradient(handle) => {
                    let id = handle.id();
                    if !gradient_ids.contains(&id) {
                        gradient_ids.push(id);
                    }
                    if let Some(mat) = gradients.get(id) {
                        let img = mat.texture.id();
                        if !image_ids.contains(&img) {
                            image_ids.push(img);
                        }
                    }
                }
                crate::vab_asset::MeshMaterial::Bitmap(handle) => {
                    let id = handle.id();
                    if !bitmap_ids.contains(&id) {
                        bitmap_ids.push(id);
                    }
                    if let Some(mat) = bitmaps.get(id) {
                        let img = mat.texture.id();
                        if !image_ids.contains(&img) {
                            image_ids.push(img);
                        }
                    }
                }
            }
        }
        Self {
            asset: asset_id,
            meshes: mesh_ids,
            gradients: gradient_ids,
            bitmaps: bitmap_ids,
            images: image_ids,
        }
    }
}

pub(super) struct RasterEntry {
    pub(super) image: Handle<Image>,
    pub(super) job: Option<Entity>,
    pub(super) done: RasterOnce,
    pub(super) last_used: u64,
    pub(super) bytes: u64,
    pub(super) deps: RasterDependencies,
}

#[derive(Default)]
pub(super) struct UiAssetInvalidation {
    pub(super) all: bool,
    pub(super) graphics: HashSet<AssetId<VabGraphic>>,
    pub(super) assets: HashSet<AssetId<VabAsset>>,
    pub(super) meshes: HashSet<AssetId<Mesh>>,
    pub(super) gradients: HashSet<AssetId<GradientMaterial>>,
    pub(super) bitmaps: HashSet<AssetId<BitmapMaterial>>,
    pub(super) images: HashSet<AssetId<Image>>,
}

impl UiAssetInvalidation {
    fn is_empty(&self) -> bool {
        !self.all
            && self.graphics.is_empty()
            && self.assets.is_empty()
            && self.meshes.is_empty()
            && self.gradients.is_empty()
            && self.bitmaps.is_empty()
            && self.images.is_empty()
    }

    fn matches(&self, key: &RasterKey, entry: &RasterEntry) -> bool {
        self.all
            || self.graphics.contains(&key.graphic)
            || self.assets.contains(&entry.deps.asset)
            || self.images.contains(&entry.image.id())
            || entry.deps.meshes.iter().any(|id| self.meshes.contains(id))
            || entry
                .deps
                .gradients
                .iter()
                .any(|id| self.gradients.contains(id))
            || entry
                .deps
                .bitmaps
                .iter()
                .any(|id| self.bitmaps.contains(id))
            || entry.deps.images.iter().any(|id| self.images.contains(id))
    }
}

#[derive(Resource, Default)]
pub(super) struct RasterCache {
    pub(super) entries: HashMap<RasterKey, RasterEntry>,
    pub(super) frame: u64,
    used: HashSet<RasterKey>,
    idle: Vec<(RasterKey, u64, u64)>,
}

impl RasterCache {
    pub(super) fn begin_frame(
        &mut self,
        commands: &mut Commands,
        invalidation: &UiAssetInvalidation,
    ) {
        self.frame += 1;
        self.used.clear();
        if invalidation.all {
            for (_, entry) in self.entries.drain() {
                entry.cancel_job(commands);
            }
        } else if !invalidation.is_empty() {
            let stale: Vec<RasterKey> = self
                .entries
                .iter()
                .filter(|(key, entry)| invalidation.matches(key, entry))
                .map(|(key, _)| *key)
                .collect();
            for key in stale {
                if let Some(entry) = self.entries.remove(&key) {
                    entry.cancel_job(commands);
                }
            }
        }
    }

    pub(super) fn mark_used(&mut self, key: RasterKey) {
        self.used.insert(key);
    }

    pub(super) fn finish_frame(&mut self, commands: &mut Commands, settings: &VabUiCacheSettings) {
        for entry in self.entries.values_mut() {
            if entry.done.complete()
                && let Some(job) = entry.job.take()
            {
                commands.entity(job).despawn();
            }
        }
        // Retain scratch allocations across frames; only cache entries own image handles.
        self.idle.clear();
        self.idle.extend(
            self.entries
                .iter()
                .filter(|(key, _)| !self.used.contains(key))
                .map(|(key, entry)| (*key, entry.last_used, entry.bytes)),
        );
        self.idle.sort_by_key(|(_, last, _)| *last);
        let mut idle_bytes: u64 = self.idle.iter().map(|(_, _, bytes)| bytes).sum();
        for (key, last, bytes) in &self.idle {
            if self.frame.saturating_sub(*last) > settings.unused_frames
                || idle_bytes > settings.idle_budget_bytes
            {
                if let Some(entry) = self.entries.remove(key) {
                    entry.cancel_job(commands);
                }
                idle_bytes -= bytes;
            }
        }
    }
}

impl RasterEntry {
    fn cancel_job(self, commands: &mut Commands) {
        if let Some(job) = self.job {
            commands.entity(job).despawn();
        }
    }
}

/// Read every event stream once per update. Images removed by cache eviction do
/// not invalidate remaining outputs, or cache collection would invalidate itself.
#[derive(SystemParam)]
pub(super) struct UiAssetChanges<'w, 's> {
    graphics: MessageReader<'w, 's, AssetEvent<VabGraphic>>,
    assets: MessageReader<'w, 's, AssetEvent<VabAsset>>,
    meshes: MessageReader<'w, 's, AssetEvent<Mesh>>,
    gradients: MessageReader<'w, 's, AssetEvent<GradientMaterial>>,
    bitmaps: MessageReader<'w, 's, AssetEvent<BitmapMaterial>>,
    images: MessageReader<'w, 's, AssetEvent<Image>>,
    shaders: MessageReader<'w, 's, AssetEvent<Shader>>,
}

impl UiAssetChanges<'_, '_> {
    pub(super) fn collect(&mut self) -> UiAssetInvalidation {
        let mut inv = UiAssetInvalidation::default();
        collect_changed(&mut self.graphics, &mut inv.graphics);
        collect_changed(&mut self.assets, &mut inv.assets);
        collect_changed(&mut self.meshes, &mut inv.meshes);
        collect_changed(&mut self.gradients, &mut inv.gradients);
        collect_changed(&mut self.bitmaps, &mut inv.bitmaps);
        for event in self.shaders.read() {
            if matches!(
                event,
                AssetEvent::Modified { .. } | AssetEvent::Removed { .. }
            ) {
                inv.all = true;
            }
        }
        for event in self.images.read() {
            if let AssetEvent::Modified { id } = event {
                inv.images.insert(*id);
            }
        }
        inv
    }
}

fn collect_changed<A: Asset>(
    events: &mut MessageReader<AssetEvent<A>>,
    out: &mut HashSet<AssetId<A>>,
) {
    for event in events.read() {
        if let AssetEvent::Modified { id } | AssetEvent::Removed { id } = event {
            out.insert(*id);
        }
    }
}
