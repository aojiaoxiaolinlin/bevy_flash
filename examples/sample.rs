use bevy::{
    app::{App, Startup},
    asset::AssetServer,
    prelude::{Camera2dBundle, Commands, Msaa, Res},
    DefaultPlugins,
};
use bevy_flash::{bundle::SwfBundle, plugin::FlashPlugin};

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, FlashPlugin))
        .insert_resource(Msaa::Sample8)
        .add_systems(Startup, setup)
        .run();
}

fn setup(mut commands: Commands, assert_server: Res<AssetServer>) {
    commands.spawn(Camera2dBundle::default());
    commands.spawn(SwfBundle {
        // swf: assert_server.load("sprite.swf"),
        // swf: assert_server.load("scale.swf"),
        // swf: assert_server.load("rotate.swf"),
        // swf: assert_server.load("head_scale2.swf"),
        // swf: assert_server.load("head-animation.swf"),
        // swf: assert_server.load("head.swf"),
        swf: assert_server.load("spirit2158src.swf"),
        ..Default::default()
    });
}
