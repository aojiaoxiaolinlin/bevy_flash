use crate::{bundle::SwfBundle, plugin::FlashPlugin};
use bevy::{
    app::{App, Startup},
    asset::AssetServer,
    prelude::{default, Commands, PluginGroup, Res},
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
fn setup(mut commands: Commands, assert_server: Res<AssetServer>) {
    commands.spawn(SwfBundle {
        swf: assert_server.load("head.swf"),
        ..Default::default()
    });
}

#[test]
fn test_load() {
    let mut app = test_app();

    app.add_systems(Startup, setup).run();
}
