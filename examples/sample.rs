use bevy::{dev_tools::fps_overlay::FpsOverlayPlugin, prelude::*};
use bevy_flash::{
    FlashCompleteEvent, FlashFrameEvent, FlashPlugin,
    assets::Swf,
    player::{Flash, FlashPlayer},
    swf_runtime::movie_clip::MovieClip,
};

fn main() {
    App::new()
        .insert_resource(ClearColor(Color::srgb_u8(102, 102, 102)))
        .add_plugins((
            DefaultPlugins.set(WindowPlugin {
                primary_window: Some(Window {
                    present_mode: bevy::window::PresentMode::AutoVsync,
                    ..Default::default()
                }),
                ..Default::default()
            }),
            FlashPlugin,
            FpsOverlayPlugin::default(),
        ))
        .add_systems(Startup, setup)
        .add_systems(Update, animation_control)
        .add_observer(flash_complete)
        .add_observer(frame_event)
        .run();
}

fn setup(mut commands: Commands, assert_server: Res<AssetServer>) {
    commands.spawn((Camera2d, Msaa::Sample8));
    commands.spawn((
        Name::new("冲霄"),
        Flash(assert_server.load("spirit2159src.swf")),
        FlashPlayer::from_animation_name("WAI"),
        Transform::from_scale(Vec3::splat(2.0)),
    ));

    commands.spawn((
        Flash(assert_server.load("埃及太阳神.swf")),
        Transform::from_scale(Vec3::splat(2.0)),
    ));

    commands.spawn((Flash(assert_server.load("loading_event_test.swf")),));
}

/// 按下 Space 控制动画跳转
fn animation_control(
    keyboard_input: Res<ButtonInput<KeyCode>>,
    mut player: Query<(&Name, &mut FlashPlayer, &mut MovieClip, &Flash)>,
    swf_res: Res<Assets<Swf>>,
) {
    if keyboard_input.just_pressed(KeyCode::Space) {
        for (name, mut player, mut root, flash) in player.iter_mut() {
            let Some(swf) = swf_res.get(flash.id()) else {
                return;
            };
            // 控制动画跳转
            if name.as_str() == "冲霄" {
                player.set_play("WAI", &swf, root.as_mut());
                player.set_looping(true);
            }
        }
    }
}

fn flash_complete(trigger: Trigger<FlashCompleteEvent>, mut player: Query<&mut FlashPlayer>) {
    let Ok(_player) = player.get_mut(trigger.target()) else {
        return;
    };
    if let Some(animation_name) = &trigger.event().animation_name {
        info!(
            "实体: {}, 动画: {:?}, 播放完毕",
            trigger.target(),
            animation_name
        );
    }
}

fn frame_event(trigger: Trigger<FlashFrameEvent>, mut player: Query<&mut FlashPlayer>) {
    let Ok(_player) = player.get_mut(trigger.target()) else {
        return;
    };
    let event_name = trigger.event().name();
    info!("实体: {}, 触发帧事件: {:?}", trigger.target(), event_name);
}
