use bevy::{dev_tools::fps_overlay::FpsOverlayPlugin, prelude::*};
use bevy_flash::{
    FlashCompleteEvent, FlashFrameEvent, FlashPlugin,
    assets::Swf,
    player::{Flash, FlashPlayer, McRoot},
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
    commands.spawn(Camera2d);
    commands.spawn((
        Name::new("冲霄"),
        Flash(assert_server.load("spirit2159src.swf")),
        FlashPlayer::from_animation_name("WAI").with_flip_x(true),
        Transform::from_scale(Vec3::splat(1.0))
            // 不要将x、y轴的缩放值设为负数，会导致显示异常，请使用FlashPlayer的with_flip_x方法,
            // 因为通过Transform的变换实现翻转时，会在动画推进中生成子的子实体，变换时继承这个变换。
            // 实现翻转时并不需要那些子实体也进行翻转，即使我将父实体的GlobalTransform的缩放设置为为绝对值，也无法即使应用到子实体。子实体还是集成的负值。
            // 因此这里目前采用多加一个参数来控制翻转，通过FlashPlayer的with_flip_x方法实现翻转。
            .with_scale(Vec3::splat(2.0))
            .with_translation(Vec3::new(-200.0, 200.0, 0.0)),
    ));

    commands.spawn((
        Flash(assert_server.load("埃及太阳神.swf")),
        FlashPlayer::from_looping(true),
        Transform::from_scale(Vec3::splat(2.0)).with_translation(Vec3::new(200.0, 200.0, 0.0)),
    ));

    // 提示按下空格键，触发动画 ATT 播放
    commands.spawn((
        Text::new("按下空格键，触发动画 ATT 播放"),
        TextFont {
            font: assert_server.load("fonts/SourceHanSansCN-Normal.otf"),
            font_size: 24.0,
            ..default()
        },
        TextLayout::new_with_justify(Justify::Center),
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(5.0),
            left: Val::Px(5.0),
            ..default()
        },
    ));
}

/// 按下 Space 控制动画跳转
fn animation_control(
    keyboard_input: Res<ButtonInput<KeyCode>>,
    mut player: Query<(&Name, &mut FlashPlayer, &mut McRoot, &Flash)>,
    swf_res: Res<Assets<Swf>>,
) {
    if keyboard_input.just_pressed(KeyCode::Space) {
        for (name, mut player, mut root, flash) in player.iter_mut() {
            let Some(swf) = swf_res.get(flash.id()) else {
                return;
            };
            // 控制动画跳转
            if name.as_str() == "冲霄" {
                player.set_play("ATT", swf, root.as_mut());
                player.set_looping(false);
            }
        }
    }
}

fn flash_complete(
    complete: On<FlashCompleteEvent>,
    mut player: Query<(&mut FlashPlayer, &Flash, &mut McRoot)>,
    swf_res: Res<Assets<Swf>>,
) {
    let Ok((mut player, flash, mut root)) = player.get_mut(complete.event_target()) else {
        return;
    };
    if let Some(animation_name) = &complete.event().name() {
        info!(
            "实体: {}, 动画: {:?}, 播放完毕",
            complete.event_target(),
            animation_name
        );
        let Some(swf) = swf_res.get(flash.id()) else {
            return;
        };
        player.set_play("WAI", swf, root.as_mut());
        player.set_looping(true);
    }
}

/// 需要在 Flash 动画中添加标签，标签名称为事件名称。
/// 事件标签格式: `event_<EventName>`
fn frame_event(frame_event: On<FlashFrameEvent>, mut player: Query<&mut FlashPlayer>) {
    let Ok(_player) = player.get_mut(frame_event.event_target()) else {
        return;
    };
    let event_name = frame_event.event().name();
    info!(
        "实体: {}, 触发帧事件: {:?}",
        frame_event.event_target(),
        event_name
    );
}
