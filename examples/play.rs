use std::collections::HashMap;

use bevy::{
    DefaultPlugins,
    app::{App, Startup, Update},
    asset::{AssetEvent, AssetServer, Assets},
    color::{Color, palettes::css::RED},
    dev_tools::fps_overlay::FpsOverlayPlugin,
    ecs::{
        children,
        event::EventReader,
        system::{Query, ResMut},
    },
    math::Vec3,
    prelude::*,
    ui::{AlignItems, JustifyContent, Node, Val},
};
use bevy_flash::{FlashPlugin, assets::FlashAnimationSwfData, bundle::FlashAnimation};

const NORMAL_BUTTON: Color = Color::srgb(0.15, 0.15, 0.15);
const HOVERED_BUTTON: Color = Color::srgb(0.25, 0.25, 0.25);
const PRESSED_BUTTON: Color = Color::srgb(0.35, 0.75, 0.35);

const SWF_ASSET: [&str; 4] = [
    "spirit2159src.swf",
    "spirit2158src.swf",
    "wu_kong.swf",
    "leiyi.swf",
];

fn main() {
    App::new()
        .insert_resource(ClearColor(Color::srgb(
            102.0 / 255.0,
            102.0 / 255.0,
            102.0 / 255.0,
        )))
        .add_plugins((DefaultPlugins, FlashPlugin, FpsOverlayPlugin::default()))
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (flash_asset_event, change_animation, update_swf, update_skin),
        )
        .run();
}

#[derive(Component, Default)]
pub struct AnimationCtrlRoot;

#[derive(Component, Default)]
pub struct SkinCtrlRoot;

fn setup(mut commands: Commands, assert_server: Res<AssetServer>) {
    commands.spawn((Camera2d, Msaa::Sample8));
    commands.spawn((
        Name::new("flash"),
        FlashAnimation {
            swf: assert_server.load("PeaShooterSingle.swf"),
        },
        Transform::from_translation(Vec3::new(200.0, 0.0, 0.0)).with_scale(Vec3::splat(2.0)),
    ));

    commands
        .spawn((Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            align_items: AlignItems::Center,
            ..Default::default()
        },))
        .with_child((
            Node {
                width: Val::Percent(20.0),
                height: Val::Percent(100.),
                flex_direction: FlexDirection::Column,
                margin: UiRect::top(Val::Px(100.0)).with_left(Val::Px(10.0)),
                ..Default::default()
            },
            AnimationCtrlRoot,
        ))
        .with_child((
            Node {
                width: Val::Auto,
                height: Val::Percent(100.),
                column_gap: Val::Px(10.0),
                ..Default::default()
            },
            SkinCtrlRoot,
        ))
        .with_children(|parent| {
            parent
                .spawn(Node {
                    width: Val::Auto,
                    height: Val::Auto,
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    column_gap: Val::Px(10.0),
                    position_type: PositionType::Absolute,
                    bottom: Val::Px(10.0),
                    left: Val::Percent(50.0),
                    ..Default::default()
                })
                .with_children(|btn_parent| {
                    btn_parent.spawn((
                        button_node_template("Up", &assert_server),
                        ChangFlashAsset::Up,
                    ));
                    btn_parent.spawn((
                        button_node_template("Down", &assert_server),
                        ChangFlashAsset::Down,
                    ));
                });
        });
}

fn flash_asset_event(
    mut commands: Commands,
    assert_server: Res<AssetServer>,
    animation_ui: Single<Entity, With<AnimationCtrlRoot>>,
    skin_ui: Single<Entity, With<SkinCtrlRoot>>,
    mut flashes: ResMut<Assets<FlashAnimationSwfData>>,
    mut flash_swf_data_events: EventReader<AssetEvent<FlashAnimationSwfData>>,
) {
    for event in flash_swf_data_events.read() {
        if let AssetEvent::LoadedWithDependencies { id } = event {
            let flash = flashes.get_mut(*id).unwrap();
            let animation_names = flash.player.animation_names();
            let skins = flash.player.get_skips();

            button(
                &mut commands,
                animation_ui.entity(),
                skin_ui.entity(),
                &assert_server,
                animation_names,
                skins,
            );
        }
    }
}

#[derive(Component, Debug, Clone, Deref, DerefMut)]
pub struct AnimationName(String);

#[derive(Component, Debug, Clone)]
pub enum ChangFlashAsset {
    Up,
    Down,
}

#[derive(Component, Debug, Clone)]
pub struct SkinChange {
    name: String,
    skin: String,
}

fn button(
    commands: &mut Commands,
    animation_ui: Entity,
    skin_ui: Entity,
    asset_server: &AssetServer,
    animation_names: Vec<&String>,
    skins: Vec<HashMap<&str, Vec<&String>>>,
) {
    commands.entity(animation_ui).with_children(|btn_parent| {
        for name in animation_names {
            btn_parent.spawn((
                button_node_template(&name, asset_server),
                AnimationName(name.clone()),
            ));
        }
    });

    for skin in skins {
        commands.entity(skin_ui).with_children(|skin_column| {
            for (name, skins) in skin.iter() {
                let mut commands = skin_column.spawn(Node {
                    width: Val::Percent(10.0),
                    height: Val::Percent(100.),
                    flex_direction: FlexDirection::Column,
                    justify_content: JustifyContent::End,
                    padding: UiRect::bottom(Val::Px(100.0)),
                    row_gap: Val::Px(10.0),
                    ..Default::default()
                });
                commands.with_child((
                    Text::new(format!("{} skin", name)),
                    TextFont {
                        font: asset_server.load("fonts/FiraSans-Bold.ttf"),
                        font_size: 33.0,
                        ..default()
                    },
                    TextColor(Color::srgb(0.9, 0.9, 0.9)),
                    TextShadow::default(),
                ));
                for skin_name in skins {
                    commands.with_children(|skin_btn_parent| {
                        skin_btn_parent.spawn((
                            button_node_template(skin_name, asset_server),
                            SkinChange {
                                name: name.to_string(),
                                skin: skin_name.to_string(),
                            },
                        ));
                    });
                }
            }
        });
    }
}

fn change_animation(
    mut interaction_query: Query<
        (
            &Interaction,
            &AnimationName,
            &mut BackgroundColor,
            &mut BorderColor,
        ),
        (Changed<Interaction>, With<Button>),
    >,
    query: Query<(&FlashAnimation, &Name)>,
    mut flashes: ResMut<Assets<FlashAnimationSwfData>>,
) {
    for (interaction, animation_name, mut color, mut border_color) in &mut interaction_query {
        match *interaction {
            Interaction::Pressed => {
                *color = PRESSED_BUTTON.into();
                border_color.0 = RED.into();
                for (flash_animation, flash_name) in query.iter() {
                    if flash_name.as_str().ne("flash") {
                        continue;
                    }
                    if let Some(flash) = flashes.get_mut(&flash_animation.swf) {
                        flash
                            .player
                            .set_play_animation(&animation_name, true, None)
                            .unwrap();
                    }
                }
            }
            Interaction::Hovered => {
                *color = HOVERED_BUTTON.into();
                border_color.0 = Color::WHITE;
            }
            Interaction::None => {
                *color = NORMAL_BUTTON.into();
                border_color.0 = Color::BLACK;
            }
        }
    }
}

fn update_swf(
    mut commands: Commands,
    mut interaction_query: Query<
        (
            &Interaction,
            &ChangFlashAsset,
            &mut BackgroundColor,
            &mut BorderColor,
        ),
        (Changed<Interaction>, With<Button>),
    >,
    animation_buttons: Query<Entity, With<AnimationName>>,
    skin_ui: Single<Entity, With<SkinCtrlRoot>>,
    query: Query<(Entity, &Name), With<FlashAnimation>>,
    asset_server: Res<AssetServer>,
    mut index: Local<usize>,
) {
    for (interaction, change_asset, mut color, mut border_color) in &mut interaction_query {
        match *interaction {
            Interaction::Pressed => {
                *color = PRESSED_BUTTON.into();
                border_color.0 = RED.into();

                match change_asset {
                    ChangFlashAsset::Up => {
                        *index += 1;
                        if *index >= SWF_ASSET.len() {
                            *index = 0;
                        }
                    }
                    ChangFlashAsset::Down if *index > 0 => {
                        *index -= 1;
                    }
                    ChangFlashAsset::Down if *index == 0 => {
                        *index = SWF_ASSET.len() - 1;
                    }
                    _ => {}
                }

                for (entity, flash_name) in query.iter() {
                    if flash_name.as_str().ne("flash") {
                        continue;
                    }
                    commands.entity(entity).despawn();
                    for animation_button in animation_buttons.iter() {
                        commands.entity(animation_button).despawn();
                    }
                    commands
                        .entity(skin_ui.entity())
                        .despawn_related::<Children>();
                    commands.spawn((
                        Name::new("flash"),
                        FlashAnimation {
                            swf: asset_server.load(SWF_ASSET[*index]),
                            ..Default::default()
                        },
                        Transform::from_translation(Vec3::new(0.0, 0.0, 0.0))
                            .with_scale(Vec3::splat(2.0)),
                    ));
                }
            }
            Interaction::Hovered => {
                *color = HOVERED_BUTTON.into();
                border_color.0 = Color::WHITE;
            }
            Interaction::None => {
                *color = NORMAL_BUTTON.into();
                border_color.0 = Color::BLACK;
            }
        }
    }
}

fn update_skin(
    mut interaction_query: Query<
        (
            &Interaction,
            &SkinChange,
            &mut BackgroundColor,
            &mut BorderColor,
        ),
        (Changed<Interaction>, With<Button>),
    >,
    query: Query<(&FlashAnimation, &Name)>,
    mut flashes: ResMut<Assets<FlashAnimationSwfData>>,
) -> Result {
    for (interaction, skin_change, mut background_color, mut border_color) in &mut interaction_query
    {
        match *interaction {
            Interaction::Pressed => {
                *background_color = PRESSED_BUTTON.into();
                border_color.0 = RED.into();

                for (flash_animation, flash_name) in query.iter() {
                    if flash_name.as_str().ne("flash") {
                        continue;
                    }
                    if let Some(flash) = flashes.get_mut(flash_animation.swf.id()) {
                        flash
                            .player
                            .set_skin(&skin_change.name, &skin_change.skin)?;
                    }
                }
            }
            Interaction::Hovered => {
                *background_color = HOVERED_BUTTON.into();
                border_color.0 = Color::WHITE;
            }
            Interaction::None => {
                *background_color = NORMAL_BUTTON.into();
                border_color.0 = Color::BLACK;
            }
        }
    }
    Ok(())
}

fn button_node_template(name: &str, asset_server: &AssetServer) -> impl Bundle + use<> {
    (
        Button,
        Node {
            width: Val::Px(150.0),
            height: Val::Px(65.0),
            border: UiRect::all(Val::Px(5.0)),
            // horizontally center child text
            justify_content: JustifyContent::Center,
            // vertically center child text
            align_items: AlignItems::Center,
            margin: UiRect::top(Val::Px(5.0)),
            ..default()
        },
        BorderColor(Color::BLACK),
        BorderRadius::MAX,
        BackgroundColor(NORMAL_BUTTON),
        children![(
            Text::new(name),
            TextFont {
                font: asset_server.load("fonts/FiraSans-Bold.ttf"),
                font_size: 33.0,
                ..default()
            },
            TextColor(Color::srgb(0.9, 0.9, 0.9)),
            TextShadow::default(),
        )],
    )
}
