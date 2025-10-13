use bevy::prelude::*;
use bevy_flash::{FlashPlugin, assets::SwfAssetLabel, shape::FlashShape};

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
        FlashShape(asset_server.load(SwfAssetLabel::Shape(6).from_asset("4.swf"))),
        Transform::from_scale(Vec3::splat(2.0)).with_translation(Vec3::new(200.0, -200.0, 0.0)),
    ));
}
