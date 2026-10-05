pub mod material;
pub mod render;
pub mod sampling;
pub mod vab_asset;
pub mod vab_player;

use bevy::prelude::*;

use crate::{
    render::FlashRenderPlugin,
    vab_asset::{VabAsset, VabLoader},
    vab_player::advance_vab_animations,
};

pub struct FlashPlayerPlugin;

impl Plugin for FlashPlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(FlashRenderPlugin)
            .init_asset::<VabAsset>()
            .init_asset::<material::GradientMaterial>()
            .init_asset::<material::BitmapMaterial>()
            .init_asset_loader::<VabLoader>()
            .add_message::<vab_player::VabFrameEvent>()
            .add_message::<vab_player::VabCompleteEvent>()
            .add_systems(PostUpdate, advance_vab_animations);
    }
}
