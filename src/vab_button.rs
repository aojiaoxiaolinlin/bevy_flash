//! Native SWF button states, exported as `ui.vab#export_name`.
use crate::vab_graphic::VabGraphic;
use bevy::prelude::*;

/// Static vector states share a fixed layout rectangle and registration point.
/// ActionScript and SWF button sounds are intentionally not executed.
#[derive(Asset, TypePath)]
pub struct VabButton {
    #[dependency]
    pub up: Handle<VabGraphic>,
    #[dependency]
    pub over: Handle<VabGraphic>,
    #[dependency]
    pub down: Handle<VabGraphic>,
    /// Preserved hit geometry, never displayed. UI currently uses Bevy's node rectangle.
    #[dependency]
    pub hit_test: Option<Handle<VabGraphic>>,
}
