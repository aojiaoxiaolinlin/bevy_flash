use bevy::{dev_tools::fps_overlay::FpsOverlayPlugin, prelude::*};
use bevy_flash::{
    FlashPlugin,
    player::{Flash, FlashPlayer},
};
fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins.set(WindowPlugin {
                primary_window: Some(Window {
                    present_mode: bevy::window::PresentMode::AutoNoVsync,
                    ..Default::default()
                }),
                ..Default::default()
            }),
            FpsOverlayPlugin::default(),
        ))
        .add_plugins(FlashPlugin)
        .add_systems(Startup, setup)
        .run();
}

fn setup(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.spawn(Camera2d);
    commands.spawn((
        Flash(asset_server.load("spirit2159src.swf")),
        FlashPlayer::from_animation_name("WAI").with_looping(true),
        Transform::from_scale(Vec3::splat(2.)),
    ));
}
