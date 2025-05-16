use bevy::{
    DefaultPlugins,
    app::{App, Startup, Update},
    asset::{AssetEvent, AssetServer, Assets},
    color::Color,
    dev_tools::fps_overlay::FpsOverlayPlugin,
    ecs::{error::Result, event::EventReader, system::ResMut},
    math::Vec3,
    prelude::{Camera2d, ClearColor, Commands, Msaa, PluginGroup, Res, Transform},
    window::{Window, WindowPlugin},
};
use bevy_flash::{FlashPlugin, assets::FlashAnimationSwfData, bundle::FlashAnimation};

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
        .add_systems(Update, flash_animation)
        .run();
}

fn setup(mut commands: Commands, assert_server: Res<AssetServer>) {
    commands.spawn((Camera2d, Msaa::Sample8));
    commands.spawn((
        FlashAnimation {
            swf: assert_server.load("spirit2159src.swf"),
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

    // commands.spawn((
    //     FlashAnimation {
    //         name: Some(String::from("m")),
    //         swf: assert_server.load("leiyi.swf"),
    //         ..Default::default()
    //     },
    //     Transform::from_translation(Vec3::new(300.0, -240.0, 0.0)).with_scale(Vec3::splat(2.0)),
    // ));
}

fn flash_animation(
    mut flashes: ResMut<Assets<FlashAnimationSwfData>>,
    mut flash_swf_data_events: EventReader<AssetEvent<FlashAnimationSwfData>>,
) -> Result {
    for event in flash_swf_data_events.read() {
        if let AssetEvent::LoadedWithDependencies { id } = event {
            let flash = flashes.get_mut(*id).unwrap();
            flash.player.set_play_animation("WAI", true, None)?;
        }
    }
    Ok(())
}
