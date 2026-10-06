//! Cached vector images and native buttons for Bevy UI.
//!
//! Public components stay in this module. Button selection, layout measurement,
//! raster scheduling and cache ownership are separate internal responsibilities.
mod button;
mod cache;
mod layout;
mod raster;

use crate::{vab_graphic::VabGraphic, vab_player::VabPlayer};
use bevy::{prelude::*, ui::UiSystems};
pub use button::VabButtonNode;
use button::select_button_images;
use cache::{PendingRaster, RasterCache};
use layout::{IntrinsicSize, measure_graphics};
use raster::{PlaybackSource, update_images};

/// Displays a static or animated export at its intrinsic size unless constrained by Node.
/// Defaults to aspect-preserving, centered fitting including filter padding.
/// Layout/DPI changes rerasterize; UiTransform scaling only scales the cached image.
#[derive(Component, Clone)]
#[require(
    ImageNode::new(bevy::image::TRANSPARENT_IMAGE_HANDLE),
    IntrinsicSize,
    VabUiPlayback,
    PlaybackSource,
    PendingRaster
)]
pub struct VabImageNode {
    pub graphic: Handle<VabGraphic>,
    pub fit: VabImageFit,
    /// Antialiasing for this cached image, independent of the UI camera.
    pub msaa: Msaa,
}
impl VabImageNode {
    pub fn new(graphic: Handle<VabGraphic>) -> Self {
        Self {
            graphic,
            fit: VabImageFit::Contain,
            msaa: Msaa::Sample4,
        }
    }
}

/// Playback controls for exported UI. Defaults to looping; static graphics stay at frame 0.
/// Query this component to pause/resume, set speed, or inspect the current frame.
#[derive(Component, Debug, Default, Deref, DerefMut)]
pub struct VabUiPlayback(pub VabPlayer);
/// Ordering for systems that consume the published UI images.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VabUiSystems {
    SelectButton,
    Rasterize,
}

/// How the full visual bounds fit into the layout rectangle.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VabImageFit {
    /// Preserve aspect ratio, center, and leave transparent space as needed.
    #[default]
    Contain,
    /// Explicitly permit non-uniform scaling to fill both dimensions.
    Stretch,
}

/// Idle cache budget; images still used by UI nodes are never evicted.
#[derive(Resource)]
pub struct VabUiCacheSettings {
    pub idle_budget_bytes: u64,
    pub unused_frames: u64,
    /// Upper bound on either raster dimension (use <= the adapter texture limit).
    pub max_dimension: u32,
}
impl Default for VabUiCacheSettings {
    fn default() -> Self {
        Self {
            idle_budget_bytes: 32 * 1024 * 1024,
            unused_frames: 120,
            max_dimension: 2048,
        }
    }
}

pub struct VabUiPlugin;
impl Plugin for VabUiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<VabUiCacheSettings>()
            .init_resource::<RasterCache>()
            .add_systems(
                PostUpdate,
                select_button_images
                    .in_set(VabUiSystems::SelectButton)
                    .before(measure_graphics),
            )
            .add_systems(
                PostUpdate,
                measure_graphics
                    .after(UiSystems::Content)
                    .before(UiSystems::Layout),
            )
            .add_systems(
                PostUpdate,
                update_images
                    .in_set(VabUiSystems::Rasterize)
                    .after(UiSystems::Layout)
                    .after(bevy::camera::visibility::VisibilitySystems::VisibilityPropagate),
            );
    }
}
