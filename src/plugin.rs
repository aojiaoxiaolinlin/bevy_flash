use crate::assets::{SwfLoader, SwfMovie};
use crate::render::{FlashRenderPlugin, SWFComponent};
use crate::swf::display_object::TDisplayObject;
use bevy::app::App;
use bevy::asset::Handle;
use bevy::prelude::{Commands, Query, Resource, Transform};
use bevy::render::view::{check_visibility, VisibilitySystems};
use bevy::sprite::ColorMaterial;
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
            .add_systems(Update, enter_frame);
    }
}

fn enter_frame(
    query: Query<&Handle<SwfMovie>>,
    time: Res<Time>,
    mut timer: ResMut<PlayerTimer>,
    mut commands: Commands,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut swf_movie: ResMut<Assets<SwfMovie>>,
    mut query_entity: Query<(&SWFComponent, &mut Transform)>,
) {
    if timer.0.tick(time.delta()).just_finished() {
        for swf_handle in query.iter() {
            if let Some(swf_movie) = swf_movie.get_mut(swf_handle.id()) {
                swf_movie
                    .root_movie_clip
                    .enter_frame(&mut swf_movie.library);
            }

            // render_base(
            //     swf_movie.root_movie_clip.clone().into(),
            //     RuffleTransform::default(),
            // );
        }
    }
}
