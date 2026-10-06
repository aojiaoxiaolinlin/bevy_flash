//! Intrinsic layout measures are independent of cached image resolution.
use super::{VabGraphic, VabImageNode};
use crate::vab_asset::VabAsset;
use bevy::{
    prelude::*,
    ui::{ComputedUiRenderTargetInfo, ContentSize, NodeMeasure, widget::ImageMeasure},
};

#[derive(Component, Default)]
pub(super) struct IntrinsicSize(pub(super) Vec2);

// Override ImageNode's texture-based measurement after Bevy's Content systems.
// Raster resolution must never become intrinsic layout size (DPI feedback loop).
#[allow(clippy::type_complexity)]
pub(super) fn measure_graphics(
    graphics: Res<Assets<VabGraphic>>,
    assets: Res<Assets<VabAsset>>,
    meshes: Res<Assets<Mesh>>,
    mut nodes: Query<(
        Ref<VabImageNode>,
        &mut IntrinsicSize,
        &mut ContentSize,
        &mut ImageNode,
        Ref<ComputedUiRenderTargetInfo>,
    )>,
) {
    for (node, mut intrinsic, mut content, mut image, target) in &mut nodes {
        if node.is_changed() || graphics.is_changed() || assets.is_changed() || meshes.is_changed()
        {
            let size = graphics
                .get(&node.graphic)
                .map(|g| Vec2::new(g.visual_bounds[2], g.visual_bounds[3]))
                .unwrap_or(Vec2::ZERO);
            if intrinsic.0 != size {
                intrinsic.0 = size;
            }
        }
        if image.image_mode != NodeImageMode::Stretch {
            image.image_mode = NodeImageMode::Stretch;
        }
        // Bevy clears Stretch image measurements in UiSystems::Content.
        // Restore vector intrinsic sizing even when the published image is unchanged.
        if intrinsic.is_changed()
            || target.is_changed()
            || image.is_changed()
            || content.is_changed()
        {
            content.set(NodeMeasure::Image(ImageMeasure {
                size: intrinsic.0 * target.scale_factor(),
                visual_box: image.visual_box,
            }));
        }
    }
}
