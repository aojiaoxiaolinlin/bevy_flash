use bevy::{
    DefaultPlugins,
    app::{App, Startup},
    asset::AssetServer,
    color::Color,
    prelude::{Camera2d, ClearColor, Commands, Res},
};
use bevy_flash::{FlashPlugin, bundle::FlashAnimation};

fn main() {
    App::new()
        .insert_resource(ClearColor(Color::srgb(
            102.0 / 255.0,
            102.0 / 255.0,
            102.0 / 255.0,
        )))
        .add_plugins((DefaultPlugins, FlashPlugin))
        .add_systems(Startup, setup)
        .run();
}

fn setup(mut commands: Commands, assert_server: Res<AssetServer>) {
    commands.spawn(Camera2d);
    commands.spawn(FlashAnimation {
        swf_movie: assert_server.load("glow.swf"),
        ..Default::default()
    });
}
