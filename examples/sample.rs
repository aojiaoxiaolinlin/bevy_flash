use bevy::{
    DefaultPlugins,
    app::{App, Startup},
    asset::AssetServer,
    color::Color,
    dev_tools::fps_overlay::FpsOverlayPlugin,
    math::Vec3,
    prelude::{Camera2d, ClearColor, Commands, Msaa, PluginGroup, Res, Transform},
    window::{Window, WindowPlugin},
};
use bevy_flash::{FlashPlugin, bundle::FlashAnimation};

fn main() {
    App::new()
        .insert_resource(ClearColor(Color::srgb(
            102.0 / 255.0,
            102.0 / 255.0,
            102.0 / 255.0,
        )))
        .add_plugins((
            DefaultPlugins.set(WindowPlugin {
                primary_window: Some(Window {
                    present_mode: bevy::window::PresentMode::AutoNoVsync,
                    ..Default::default()
                }),
                ..Default::default()
            }),
            FlashPlugin,
            FpsOverlayPlugin::default(),
        ))
        .add_systems(Startup, setup)
        .run();
}

fn setup(mut commands: Commands, assert_server: Res<AssetServer>) {
    commands.spawn((Camera2d, Msaa::Sample8));
    commands.spawn((
        FlashAnimation {
            name: Some(String::from("mc")),
            swf_asset: assert_server.load("spirit2159src.swf"),
            ignore_root_swf_transform: true,
            ..Default::default()
        },
        Transform::from_translation(Vec3::new(0.0, 0.0, 0.0)).with_scale(Vec3::splat(2.0)),
    ));

    // commands.spawn((
    //     FlashAnimation {
    //         name: Some(String::from("m")),
    //         swf_asset: assert_server.load("131381-idle.swf"),
    //         ..Default::default()
    //     },
    //     Transform::from_translation(Vec3::new(0.0, 0.0, 0.0)).with_scale(Vec3::splat(8.0)),
    // ));

    commands.spawn((
        FlashAnimation {
            name: Some(String::from("m")),
            swf_asset: assert_server.load("leiyi2.swf"),
            ignore_root_swf_transform: false,
            ..Default::default()
        },
        Transform::from_translation(Vec3::new(300.0, -240.0, 0.0)).with_scale(Vec3::splat(2.0)),
    ));
}
