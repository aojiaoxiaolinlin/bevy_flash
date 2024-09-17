use bevy::{
    app::{App, Startup},
    asset::AssetServer,
    prelude::{Camera2dBundle, Commands, Msaa, Res, SpatialBundle, Transform},
    DefaultPlugins,
};
use bevy_flash::{bundle::SwfBundle, plugin::FlashPlugin};
use glam::Vec3;

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
        // swf_handle: assert_server.load("sprite.swf"),
        // swf_handle: assert_server.load("scale.swf"),
        // swf_handle: assert_server.load("rotate.swf"),
        // swf_handle: assert_server.load("head_scale2.swf"),
        // swf_handle: assert_server.load("head-animation.swf"),
        // swf_handle: assert_server.load("head.swf"),
        swf_handle: assert_server.load("spirit2158src.swf"),
        // swf_handle: assert_server.load("frames.swf"),
        // swf_handle: assert_server.load("bitmap_test.swf"),
        // swf_handle: assert_server.load("miaomiao.swf"),
        // swf_handle: assert_server.load("tou.swf"),
        // swf_handle: assert_server.load("123680-idle.swf"),
        // swf_handle: assert_server.load("frame_animation.swf"),
        // swf_handle: assert_server.load("gradient.swf"),
        // swf_handle: assert_server.load("weiba.swf"),
        // swf_handle: assert_server.load("spirit1src.swf"),
        // swf_handle: assert_server.load("32.swf"),
        spatial: SpatialBundle {
            transform: Transform::from_scale(Vec3::splat(1.0)),
            ..Default::default()
        },
        ..Default::default()
    });
}
