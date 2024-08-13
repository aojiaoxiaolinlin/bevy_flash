use bevy::{
    app::{App, Startup},
    asset::AssetServer,
    prelude::{Camera2dBundle, Commands, Res},
    DefaultPlugins,
};
use bevy_flash::{bundle::SwfBundle, plugin::FlashPlugin};

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, FlashPlugin))
        .add_systems(Startup, setup)
        .run();
}

fn setup(mut commands: Commands, assert_server: Res<AssetServer>) {
    commands.spawn(Camera2dBundle::default());
    commands.spawn(SwfBundle {
        swf: assert_server.load("head.swf"),
        // swf: assert_server.load("spirit2471src.swf"),
        ..Default::default()
    });
}
