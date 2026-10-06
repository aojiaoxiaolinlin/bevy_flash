//! Animated exported UI. Space pauses; arrows change playback speed.
use bevy::prelude::*;
use bevy_flash::{
    FlashPlayerPlugin,
    vab_ui::{VabImageNode, VabUiPlayback, VabUiPlugin},
};
fn main() {
    App::new()
        .add_plugins((DefaultPlugins, FlashPlayerPlugin, VabUiPlugin))
        .add_systems(Startup, setup)
        .add_systems(Update, controls)
        .run();
}
fn setup(mut commands: Commands, assets: Res<AssetServer>) {
    commands.spawn(Camera2d);
    let graphic = assets.load("background551284.vab#sparkles");
    commands
        .spawn(Node {
            width: percent(100),
            height: percent(100),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            column_gap: px(32),
            ..default()
        })
        .with_children(|parent| {
            for width in [240.0, 240.0, 480.0] {
                parent.spawn((
                    VabImageNode::new(graphic.clone()),
                    Node {
                        width: px(width),
                        ..default()
                    },
                ));
            }
        });
}
fn controls(keys: Res<ButtonInput<KeyCode>>, mut players: Query<&mut VabUiPlayback>) {
    for mut player in &mut players {
        if keys.just_pressed(KeyCode::Space) {
            player.playing = !player.playing;
        }
        let speed = if keys.just_pressed(KeyCode::ArrowRight) {
            Some(player.speed() * 2.0)
        } else if keys.just_pressed(KeyCode::ArrowLeft) {
            Some(player.speed() * 0.5)
        } else {
            None
        };
        if let Some(speed) = speed {
            let _ = player.set_speed(speed);
        }
    }
}
