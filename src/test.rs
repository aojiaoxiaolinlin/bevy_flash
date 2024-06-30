use crate::{assets::FlashData, flash_bundle::FlashBundle, plugin::FlashPlugin};
use bevy::{
    app::{App, Startup},
    asset::{AssetServer, Assets},
    prelude::{default, Commands, PluginGroup, Res, ResMut},
    render::{settings::WgpuSettings, RenderPlugin},
    winit::WinitPlugin,
    DefaultPlugins,
};

pub fn test_app() -> App {
    let mut app = App::new();
    app.add_plugins((
        DefaultPlugins
            .set(RenderPlugin {
                render_creation: WgpuSettings {
                    backends: None,
                    ..default()
                }
                .into(),
                ..default()
            })
            .build()
            .disable::<WinitPlugin>(),
        FlashPlugin,
    ));
    app
}
fn setup(
    mut commands: Commands,
    mut flash_res: ResMut<Assets<FlashData>>,
    assert_server: Res<AssetServer>,
) {
    let flash_data = FlashData::new_from_binary_data(assert_server.load("head.swf"));
    let flash_handle = flash_res.add(flash_data);

    commands.spawn(FlashBundle {
        flash: flash_handle,
        ..Default::default()
    });
}

#[test]
fn test_load() {
    let mut app = test_app();

    app.add_systems(Startup, setup).run();
}


