//! Minimal VAB playback example. For performance diagnostics, run spirit_diagnostics.
use bevy::prelude::*;
use bevy_flash_remake::{
    FlashPlayerPlugin,
    render::{TransientTexturePoolSettings, VabFilterMsaa},
    vab_asset::{VabAsset, VabAssetHandle},
    vab_player::{VabFrameEvent, VabPlayer},
};

#[derive(Component)]
struct Spirit;

#[derive(Component)]
struct StartWhenLoaded;

fn main() {
    App::new()
        .insert_resource(ClearColor(Color::srgb_u8(102, 102, 102)))
        .insert_resource(VabFilterMsaa::Off)
        // Retain recurring filter targets for this heavy animation at 2x scale.
        .insert_resource(TransientTexturePoolSettings {
            max_resident_bytes: 256 * 1024 * 1024,
            max_unused_frames: 1000,
            ..default()
        })
        .add_plugins((DefaultPlugins, FlashPlayerPlugin))
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (start_when_loaded, keyboard_control, handle_frame_events),
        )
        .run();
}

fn setup(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.spawn((Camera2d, CompositingSpace::Srgb, Msaa::Sample4));
    commands.spawn((
        Name::new("123620-2x"),
        Spirit,
        StartWhenLoaded,
        VabAssetHandle(asset_server.load("123620.vab")),
        VabPlayer::default(),
        Transform::from_scale(Vec3::splat(2.0)),
    ));
    info!("controls: Space = ATT, R = WAI, P = pause/resume");
}

fn start_when_loaded(
    mut commands: Commands,
    assets: Res<Assets<VabAsset>>,
    mut spirits: Query<(Entity, &VabAssetHandle, &mut VabPlayer), With<StartWhenLoaded>>,
) {
    for (entity, handle, mut player) in &mut spirits {
        let Some(asset) = assets.get(&handle.0) else {
            continue;
        };
        if let Err(error) = player.set_fallback_loop(asset, "ATTACK") {
            warn!("ATTACK is unavailable: {error}");
        } else if let Err(error) = player.play_loop(asset, "ATTACK") {
            warn!("ATTACK is unavailable: {error}");
        }
        commands.entity(entity).remove::<StartWhenLoaded>();
    }
}

fn keyboard_control(
    keyboard: Res<ButtonInput<KeyCode>>,
    assets: Res<Assets<VabAsset>>,
    mut spirits: Query<(&VabAssetHandle, &mut VabPlayer), With<Spirit>>,
) {
    for (handle, mut player) in &mut spirits {
        let Some(asset) = assets.get(&handle.0) else {
            continue;
        };
        if keyboard.just_pressed(KeyCode::Space)
            && let Err(error) = player.play_once(asset, "ATT")
        {
            warn!("ATT is unavailable: {error}");
        }
        if keyboard.just_pressed(KeyCode::KeyR)
            && let Err(error) = player.play_loop(asset, "WAI")
        {
            warn!("WAI is unavailable: {error}");
        }
        if keyboard.just_pressed(KeyCode::KeyP) {
            if player.playing {
                player.pause();
            } else {
                player.resume();
            }
        }
    }
}

fn handle_frame_events(mut events: MessageReader<VabFrameEvent>) {
    for event in events.read() {
        debug!(
            "entity={} animation={} frame={} event={}",
            event.entity, event.animation, event.frame, event.name
        );
    }
}
