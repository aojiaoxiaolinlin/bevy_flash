use bevy::{
    app::{App, Startup},
    asset::{AssetServer, Assets},
    prelude::{Commands, Res, ResMut},
    DefaultPlugins,
};
use bevy_flash::{assets::FlashData, flash_bundle::FlashBundle, plugin::FlashPlugin};

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, FlashPlugin))
        .add_systems(Startup, setup)
        .run();
}

fn setup(
    mut commands: Commands,
    mut flash_res: ResMut<Assets<FlashData>>,
    assert_server: Res<AssetServer>,
) {
    // let flash_data = FlashData::new_from_binary_data(assert_server.load("head.swf"));
    let flash_data = FlashData::new_from_binary_data(assert_server.load("head-animation.swf"));
    // let flash_data = FlashData::new_from_binary_data(assert_server.load("spirit2471src.swf"));
    let flash_handle = flash_res.add(flash_data);

    commands.spawn(FlashBundle {
        flash: flash_handle,
        ..Default::default()
    });
}
