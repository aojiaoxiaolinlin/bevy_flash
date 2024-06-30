use bevy::{app::Plugin, asset::AssetApp};

use crate::assets::{FlashData, SwfLoader, SwfMovie};

pub struct FlashPlugin;

impl Plugin for FlashPlugin {
    fn build(&self, app: &mut bevy::prelude::App) {
        app.init_asset::<FlashData>()
            .init_asset::<SwfMovie>()
            .init_asset_loader::<SwfLoader>();
    }
}
