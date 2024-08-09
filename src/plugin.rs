use crate::assets::{SwfLoader, SwfMovie};
use crate::bundle::SwfSprite;
use crate::render::FlashRenderPlugin;
use crate::swf::display_object::TDisplayObject;
use bevy::app::{App, PostUpdate};
use bevy::asset::Handle;
use bevy::prelude::{IntoSystemConfigs, Query, Resource, With};
use bevy::render::view::{check_visibility, VisibilitySystems};
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
        app.add_plugins(FlashRenderPlugin)
            .init_asset::<SwfMovie>()
            .init_asset_loader::<SwfLoader>()
            .insert_resource(PlayerTimer(Timer::from_seconds(
                // TODO: 24fps
                24.0 / 1000.0,
                TimerMode::Repeating,
            )))
            .add_systems(Update, enter_frame)
            .add_systems(
                PostUpdate,
                check_visibility::<With<SwfSprite>>.in_set(VisibilitySystems::CheckVisibility),
            );
    }
}

fn enter_frame(
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

fn render_base(query: Query<&Handle<SwfMovie>>, mut swf_movie: ResMut<Assets<SwfMovie>>) {
    for swf_handle in query.iter() {
        if let Some(swf_movie) = swf_movie.get_mut(swf_handle.id()) {
            let root_movie_clip = swf_movie.root_movie_clip.clone();
            root_movie_clip.render_self();
        }
    }
}
