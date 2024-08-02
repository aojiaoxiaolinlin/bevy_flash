use crate::assets::{FlashData, SwfLoader, SwfMovie};
use crate::swf::display_object::TDisplayObject;
use bevy::app::App;
use bevy::asset::Handle;
use bevy::core_pipeline::core_2d::Transparent2d;
use bevy::prelude::{
    Commands, GlobalTransform, InheritedVisibility, Query, Resource, ViewVisibility,
};
use bevy::render::render_phase::AddRenderCommand;
use bevy::render::view::RenderLayers;
use bevy::render::{Extract, ExtractSchedule, RenderApp};
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
        app.init_asset::<FlashData>()
            .init_asset::<SwfMovie>()
            .init_asset_loader::<SwfLoader>()
            .insert_resource(PlayerTimer(Timer::from_seconds(
                24.0 / 1000.0,
                TimerMode::Repeating,
            )))
            .add_systems(Update, flash_enter_frame);
    }

    fn finish(&self, app: &mut App) {
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            todo!();
        }
    }
}

fn flash_enter_frame(
    time: Res<Time>,
    mut timer: ResMut<PlayerTimer>,
    flash_data: Res<Assets<FlashData>>,
    mut swf_movie: ResMut<Assets<SwfMovie>>,
) {
    if timer.0.tick(time.delta()).just_finished() {
        flash_data.iter().for_each(|(_id, flash_data)| {
            if let Some(swf_movie) = swf_movie.get_mut(flash_data.swf_movie.id()) {
                swf_movie
                    .root_movie_clip
                    .enter_frame(&mut swf_movie.library);
            }
        });
    }
}

fn extract_flash(
    mut command: Commands,
    query_flash: Extract<
        Query<(
            &Handle<SwfMovie>,
            &GlobalTransform,
            Option<&RenderLayers>,
            &ViewVisibility,
            &InheritedVisibility,
        )>,
    >,
    swf_movie: Extract<Res<Assets<SwfMovie>>>,
) {
    todo!();
}
