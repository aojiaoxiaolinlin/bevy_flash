use crate::assets::{SwfLoader, SwfMovie};
use crate::swf::display_object::TDisplayObject;
use bevy::app::App;
use bevy::asset::Handle;
use bevy::ecs::query;
use bevy::prelude::{Query, Resource};
use bevy::render::RenderApp;
use bevy::time::{Time, Timer, TimerMode};
use bevy::{
    app::{Plugin, Update},
    asset::{AssetApp, Assets},
    prelude::{Res, ResMut},
};

#[derive(Resource)]
struct PlayerTimer(Timer);

pub struct FlashPlugin;

impl Plugin for FlashPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<SwfMovie>()
            .init_asset_loader::<SwfLoader>()
            .insert_resource(PlayerTimer(Timer::from_seconds(
                // TODO: 24fps
                24.0 / 1000.0,
                TimerMode::Repeating,
            )))
            .add_systems(Update, flash_enter_frame);
    }

    fn finish(&self, app: &mut App) {
        if let Some(_render_app) = app.get_sub_app_mut(RenderApp) {}
    }
}

fn flash_enter_frame(
    query: Query<&Handle<SwfMovie>>,
    _time: Res<Time>,
    mut _timer: ResMut<PlayerTimer>,
    mut swf_movie: ResMut<Assets<SwfMovie>>,
) {
    for swf_handle in query.iter() {
        if let Some(swf_movie) = swf_movie.get_mut(swf_handle.id()) {
            swf_movie
                .root_movie_clip
                .enter_frame(&mut swf_movie.library);
        }
    }
}
