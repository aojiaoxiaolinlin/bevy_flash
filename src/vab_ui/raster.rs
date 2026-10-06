//! Advance visible UI playback and schedule one-shot raster work.
use super::cache::{
    PendingImage, PendingRaster, RasterCache, RasterEntry, RasterKey, UiAssetChanges,
};
use super::{VabGraphic, VabImageFit, VabImageNode, VabUiCacheSettings, VabUiPlayback};
use crate::{
    render::{OffscreenViewTarget, RasterOnce},
    vab_asset::{VabAsset, VabAssetHandle},
    vab_player::VabPlayer,
};
use bevy::{asset::AssetId, prelude::*};

#[derive(Component, Default)]
pub(super) struct PlaybackSource(pub(super) Option<AssetId<VabGraphic>>);

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn update_images(
    mut commands: Commands,
    mut cache: ResMut<RasterCache>,
    settings: Res<VabUiCacheSettings>,
    graphics: Res<Assets<VabGraphic>>,
    assets: Res<Assets<VabAsset>>,
    mut images: ResMut<Assets<Image>>,
    time: Res<Time>,
    mut nodes: Query<(
        &VabImageNode,
        &ComputedNode,
        &mut ImageNode,
        &mut VabUiPlayback,
        Option<&InheritedVisibility>,
        &mut PlaybackSource,
        &mut PendingRaster,
    )>,
    mut changes: UiAssetChanges,
) {
    let invalid = changes.invalidated();
    cache.begin_frame(&mut commands, invalid);
    let frame = cache.frame;
    for (node, layout, mut image_node, mut playback, visibility, mut source, mut pending) in
        &mut nodes
    {
        if invalid {
            pending.0 = None;
        }
        if visibility.is_some_and(|v| !v.get()) {
            continue;
        }
        let visual_size = match image_node.visual_box {
            bevy::ui::VisualBox::ContentBox => layout.content_box().size(),
            bevy::ui::VisualBox::PaddingBox => layout.padding_box().size(),
            bevy::ui::VisualBox::BorderBox => layout.size,
        };
        if !visual_size.is_finite() || visual_size.min_element() <= 0.0 {
            continue;
        }
        let Some(graphic) = graphics.get(&node.graphic) else {
            image_node.image = bevy::image::TRANSPARENT_IMAGE_HANDLE;
            continue;
        };
        let Some(asset) = assets.get(&graphic.render_asset) else {
            continue;
        };
        if source.0 != Some(node.graphic.id()) {
            playback.current_frame = 0;
            playback.timer = 0.0;
            source.0 = Some(node.graphic.id());
            pending.0 = None;
        }
        if graphic.frame_count > 1 {
            playback.advance(asset, time.delta_secs_f64());
        } else {
            playback.current_frame = 0;
        }
        let selected_frame = playback.current_frame.min(graphic.frame_count - 1);
        let limit = settings.max_dimension.clamp(1, 8192) as f32;
        let size = (visual_size * (limit / visual_size.max_element()).min(1.0))
            .ceil()
            .max(Vec2::ONE)
            .as_uvec2();
        let key = RasterKey {
            graphic: node.graphic.id(),
            size,
            samples: node.msaa.samples(),
            fit: node.fit,
            frame: selected_frame,
        };
        if let Some(waiting) = &pending.0
            && (!waiting.key.same_target(&key) || !cache.entries.contains_key(&waiting.key))
        {
            pending.0 = None;
        }
        if let Some(waiting) = &pending.0 {
            if waiting.done.complete() {
                image_node.image = waiting.image.clone();
                pending.0 = None;
            } else {
                // Finish the in-flight frame even when playback has moved ahead.
                // Keep its lease protected and the previously published picture visible.
                cache.mark_used(waiting.key);
                continue;
            }
        }
        if let std::collections::hash_map::Entry::Vacant(slot) = cache.entries.entry(key) {
            let unit = graphic.visual_bounds;
            let ratios = size.as_vec2() / Vec2::new(unit[2], unit[3]);
            let scale = match node.fit {
                VabImageFit::Contain => Vec2::splat(ratios.min_element()),
                VabImageFit::Stretch => ratios,
            };
            let bounds = [
                unit[0] * scale.x,
                unit[1] * scale.y,
                unit[2] * scale.x,
                unit[3] * scale.y,
            ];
            let raster_size = size;
            let image = images.add(OffscreenViewTarget::create_image(raster_size));
            let mut target = OffscreenViewTarget::new(image.clone(), raster_size);
            target.scale = scale;
            // Center the visual bounds inside the complete node-sized transparent image.
            target.origin = Vec2::new(bounds[0], bounds[1])
                - (size.as_vec2() - Vec2::new(bounds[2], bounds[3])) * 0.5;
            let mut player = VabPlayer::default();
            player.pause();
            player.current_frame = selected_frame;
            let done = RasterOnce::default();
            let job = commands
                .spawn((
                    VabAssetHandle(graphic.render_asset.clone()),
                    player,
                    target,
                    node.msaa,
                    done.clone(),
                ))
                .id();
            slot.insert(RasterEntry {
                image,
                job: Some(job),
                done,
                last_used: frame,
                bytes: raster_size.x as u64 * raster_size.y as u64 * 4,
            });
        }
        let entry = cache.entries.get_mut(&key).unwrap();
        entry.last_used = frame;
        if entry.done.complete() {
            if image_node.image != entry.image {
                image_node.image = entry.image.clone();
            }
        } else {
            pending.0 = Some(PendingImage {
                key,
                image: entry.image.clone(),
                done: entry.done.clone(),
            });
        }
        cache.mark_used(key);
    }
    cache.finish_frame(&mut commands, &settings);
}
