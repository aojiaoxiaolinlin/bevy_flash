use bevy_flash::{
    vab_asset::VabAsset,
    vab_player::{PlaybackEvent, VabPlayer},
};
use vatf::baked::{BakedClip, BakedMovie, FrameEvent};

fn asset() -> VabAsset {
    VabAsset {
        shape_map: Default::default(),
        morph_map: Default::default(),
        render_meshes: vec![],
        baked: BakedMovie {
            frame_rate: 10.0,
            skins: vec![],
            clips: vec![
                BakedClip {
                    name: "attack".into(),
                    start_frame: 0,
                    frames: vec![vec![]; 4],
                    events: vec![
                        FrameEvent {
                            frame: 0,
                            name: "start".into(),
                        },
                        FrameEvent {
                            frame: 2,
                            name: "hit".into(),
                        },
                        FrameEvent {
                            frame: 3,
                            name: "hit".into(),
                        },
                    ],
                },
                BakedClip {
                    name: "recover".into(),
                    start_frame: 4,
                    frames: vec![vec![]; 2],
                    events: vec![FrameEvent {
                        frame: 0,
                        name: "recover_start".into(),
                    }],
                },
                BakedClip {
                    name: "idle".into(),
                    start_frame: 6,
                    frames: vec![vec![]; 3],
                    events: vec![FrameEvent {
                        frame: 0,
                        name: "idle_start".into(),
                    }],
                },
                BakedClip {
                    name: "death".into(),
                    start_frame: 9,
                    frames: vec![vec![]; 2],
                    events: vec![],
                },
            ],
        },
    }
}

fn frame(frame: usize, name: &str) -> PlaybackEvent {
    PlaybackEvent::Frame {
        clip: 0,
        frame,
        name: name.into(),
    }
}

fn clip_frame(clip: usize, frame: usize, name: &str) -> PlaybackEvent {
    PlaybackEvent::Frame {
        clip,
        frame,
        name: name.into(),
    }
}

#[test]
fn skipped_frames_and_multiple_loops_preserve_event_order() {
    let asset = asset();
    let mut player = VabPlayer::default();
    assert_eq!(
        player.advance(&asset, 0.85),
        vec![
            frame(0, "start"),
            frame(2, "hit"),
            frame(3, "hit"),
            frame(0, "start"),
            frame(2, "hit"),
            frame(3, "hit"),
            frame(0, "start")
        ]
    );
    assert_eq!(player.current_frame, 0);
    assert!((player.timer - 0.5).abs() < 1e-9);
    assert!(player.advance(&asset, 0.0).is_empty());
}

#[test]
fn completion_holds_last_frame_and_occurs_once() {
    let asset = asset();
    let mut player = VabPlayer::default();
    player.set_looping(false);
    assert_eq!(
        player.advance(&asset, 1.0),
        vec![
            frame(0, "start"),
            frame(2, "hit"),
            frame(3, "hit"),
            PlaybackEvent::Complete { clip: 0 }
        ]
    );
    assert_eq!(player.current_frame, 3);
    assert!(player.is_complete());
    assert!(player.advance(&asset, 1.0).is_empty());
}

#[test]
fn fallback_transition_consumes_remaining_time_without_losing_event_source() {
    let asset = asset();
    let mut player = VabPlayer::default();
    player.set_fallback_loop(&asset, "idle").unwrap();
    player.play_once(&asset, "attack").unwrap();

    assert_eq!(
        player.advance(&asset, 0.45),
        vec![
            frame(0, "start"),
            frame(2, "hit"),
            frame(3, "hit"),
            PlaybackEvent::Complete { clip: 0 },
            clip_frame(2, 0, "idle_start"),
        ]
    );
    assert_eq!(player.clip(), 2);
    assert_eq!(player.current_frame, 0);
    assert!((player.timer - 0.5).abs() < 1e-9);
    assert!(player.is_looping());
}

#[test]
fn queued_sequence_can_cross_multiple_clips_in_one_update() {
    let asset = asset();
    let mut player = VabPlayer::default();
    player
        .play_once(&asset, "attack")
        .unwrap()
        .then_once(&asset, "recover")
        .unwrap()
        .then_loop(&asset, "idle")
        .unwrap();

    assert_eq!(
        player.advance(&asset, 0.7),
        vec![
            frame(0, "start"),
            frame(2, "hit"),
            frame(3, "hit"),
            PlaybackEvent::Complete { clip: 0 },
            clip_frame(1, 0, "recover_start"),
            PlaybackEvent::Complete { clip: 1 },
            clip_frame(2, 0, "idle_start"),
        ]
    );
    assert_eq!(player.clip(), 2);
    assert_eq!(player.current_frame, 1);
    assert!(player.is_looping());
}

#[test]
fn terminal_playback_suppresses_fallback_and_rejects_overrides() {
    let asset = asset();
    let mut player = VabPlayer::default();
    player.set_fallback_loop(&asset, "idle").unwrap();
    player.play_terminal(&asset, "death").unwrap();

    assert_eq!(
        player.advance(&asset, 1.0),
        vec![PlaybackEvent::Complete { clip: 3 }]
    );
    assert!(player.is_terminal());
    assert!(player.is_complete());
    assert_eq!(player.clip(), 3);
    assert_eq!(player.current_frame, 1);
    assert!(player.play_loop(&asset, "idle").is_err());
    player.set_looping(true);
    assert!(!player.is_looping());

    player.reset_terminal();
    player.play_loop(&asset, "idle").unwrap();
    assert!(!player.is_terminal());
    assert_eq!(player.clip(), 2);
}

#[test]
fn shortcut_validates_both_clips_before_changing_playback() {
    let asset = asset();
    let mut player = VabPlayer::default();
    player.play_loop(&asset, "idle").unwrap();
    assert!(player.play_once_then(&asset, "attack", "missing").is_err());
    assert_eq!(player.clip(), 2);
    assert!(player.is_looping());
}

#[test]
fn seek_pause_replay_and_invalid_inputs() {
    let asset = asset();
    let mut player = VabPlayer::default();
    player.seek(&asset, 2).unwrap();
    assert!(player.advance(&asset, 0.0).is_empty());
    player.pause();
    assert!(player.advance(&asset, 1.0).is_empty());
    assert_eq!(player.current_frame, 2);
    assert!(player.set_speed(f64::INFINITY).is_err());
    assert!(player.play(&asset, "missing").is_err());
    assert!(player.seek(&asset, 4).is_err());
    player.play(&asset, "attack").unwrap();
    assert_eq!(player.advance(&asset, 0.0), vec![frame(0, "start")]);
    player.frame_rate_override = Some(f32::INFINITY);
    assert!(player.advance(&asset, 1.0).is_empty());
    assert_eq!(player.current_frame, 0);
}

#[test]
fn ecs_system_emits_messages_and_resets_when_asset_changes() {
    use bevy::{
        asset::{AssetPlugin, Assets},
        prelude::*,
        time::TimeUpdateStrategy,
    };
    use bevy_flash::{
        FlashPlayerPlugin, vab_asset::VabAssetHandle, vab_player::VabFrameEvent,
    };
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default(), FlashPlayerPlugin));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_millis(250),
    ));
    let handle = app
        .world_mut()
        .resource_mut::<Assets<VabAsset>>()
        .add(asset());
    let entity = app
        .world_mut()
        .spawn((VabAssetHandle(handle), VabPlayer::default()))
        .id();
    let mut cursor = bevy::ecs::message::MessageCursor::<VabFrameEvent>::default();
    app.update();
    let first: Vec<_> = cursor
        .read(app.world().resource::<Messages<VabFrameEvent>>())
        .collect();
    assert_eq!(first[0].entity, entity);
    assert_eq!(first[0].name, "start");
    app.update();
    assert!(
        cursor
            .read(app.world().resource::<Messages<VabFrameEvent>>())
            .any(|event| event.name == "hit")
    );
    app.world_mut()
        .get_mut::<VabPlayer>(entity)
        .unwrap()
        .pause();
    let replacement = app
        .world_mut()
        .resource_mut::<Assets<VabAsset>>()
        .add(asset());
    app.world_mut().get_mut::<VabAssetHandle>(entity).unwrap().0 = replacement;
    app.update();
    let player = app.world().get::<VabPlayer>(entity).unwrap();
    assert_eq!(player.current_frame, 0);
    assert!(!player.playing);
}
