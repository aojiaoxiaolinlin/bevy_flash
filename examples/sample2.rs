use bevy::prelude::*;
use bevy_flash::{
    FlashPlugin,
    player::{Flash, FlashPlayer},
};

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(FlashPlugin)
        .add_systems(Startup, setup)
        .run();
}

fn setup(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.spawn(Camera2d);
    commands.spawn((
        Flash(asset_server.load("983.swf")),
        FlashPlayer::from_looping(true),
        Transform::from_scale(Vec3::splat(2.0)),
    ));
}
