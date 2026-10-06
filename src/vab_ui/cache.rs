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
pub(super) struct RasterEntry {
    pub(super) image: Handle<Image>,
    pub(super) job: Option<Entity>,
    pub(super) done: RasterOnce,
    pub(super) last_used: u64,
    pub(super) bytes: u64,
}
#[derive(Resource, Default)]
pub(super) struct RasterCache {
    pub(super) entries: HashMap<RasterKey, RasterEntry>,
    pub(super) frame: u64,
    used: HashSet<RasterKey>,
    idle: Vec<(RasterKey, u64, u64)>,
}

impl RasterCache {
    pub(super) fn begin_frame(&mut self, commands: &mut Commands, invalidated: bool) {
        self.frame += 1;
        self.used.clear();
        if invalidated {
            for (_, entry) in self.entries.drain() {
                entry.cancel_job(commands);
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
}

impl UiAssetChanges<'_, '_> {
    #[allow(clippy::unnecessary_fold)]
    pub(super) fn invalidated(&mut self) -> bool {
        // Build the array before any(): short-circuiting the readers would leave
        // stale events to trigger unnecessary invalidations in subsequent frames.
        [
            changed(&mut self.graphics),
            changed(&mut self.assets),
            changed(&mut self.meshes),
            changed(&mut self.gradients),
            changed(&mut self.bitmaps),
            self.images.read().fold(false, |dirty, e| {
                matches!(e, AssetEvent::Modified { .. }) || dirty
            }),
        ]
        .into_iter()
        .any(|dirty| dirty)
    }
}

#[allow(clippy::unnecessary_fold)]
fn changed<A: Asset>(events: &mut MessageReader<AssetEvent<A>>) -> bool {
    events.read().fold(false, |dirty, event| {
        matches!(
            event,
            AssetEvent::Modified { .. } | AssetEvent::Removed { .. }
        ) || dirty
    })
}
