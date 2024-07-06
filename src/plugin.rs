use crate::assets::{FlashData, SwfLoader, SwfMovie};
use crate::flash_utils::display_object::TDisplayObject;
use bevy::{
    app::{Plugin, Update},
    asset::{AssetApp, Assets},
    prelude::{Res, ResMut},
};

pub struct FlashPlugin;

impl Plugin for FlashPlugin {
    fn build(&self, app: &mut bevy::prelude::App) {
        app.init_asset::<FlashData>()
            .init_asset::<SwfMovie>()
            .init_asset_loader::<SwfLoader>()
            .add_systems(Update, flash_enter_frame);
    }
}

fn flash_enter_frame(flash_data: Res<Assets<FlashData>>, mut swf_movie: ResMut<Assets<SwfMovie>>) {
    flash_data.iter().for_each(|(_id, flash_data)| {
        if let Some(swf_movie) = swf_movie.get_mut(flash_data.swf_movie.id()) {
            println!("swf_movie: {:?}", swf_movie.library.characters().len());
            swf_movie
                .root_movie_clip
                .enter_frame(&mut swf_movie.library);
        }
    })
}
