//! SWF sources → Bevy processed cache → ordinary VAB/UI loaders.
use bevy::{asset::AssetMode, prelude::*};
use bevy_flash_remake::{
    FlashPlayerPlugin,
    asset_processing::{VabAssetProcessorPlugin, vab_processed_asset_path},
    vab_graphic::VabAssetLabel,
    vab_ui::{VabButtonNode, VabImageNode, VabUiPlugin},
};

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins.set(AssetPlugin {
                mode: AssetMode::Processed,
                file_path: "examples/processed_assets".into(),
                processed_file_path: vab_processed_asset_path("imported_assets/processed_ui")
                    .to_string_lossy()
                    .into_owned(),
                ..default()
            }),
            FlashPlayerPlugin,
            VabAssetProcessorPlugin,
            VabUiPlugin,
        ))
        .add_systems(Startup, setup)
        .run();
}

fn setup(mut commands: Commands, assets: Res<AssetServer>) {
    commands.spawn(Camera2d);
    commands
        .spawn(Node {
            width: percent(100),
            height: percent(100),
            flex_direction: FlexDirection::Column,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            row_gap: px(36),
            ..default()
        })
        .with_children(|parent| {
            parent.spawn((
                VabImageNode::new(
                    assets.load(
                        VabAssetLabel::Graphic("button_background").from_asset("ui_demo.swf"),
                    ),
                ),
                Node {
                    width: px(320),
                    ..default()
                },
            ));
            parent.spawn((
                VabImageNode::new(assets.load("animated_ui.swf#sparkles")),
                Node {
                    width: px(320),
                    ..default()
                },
            ));
            parent.spawn((
                VabButtonNode::new(
                    assets.load(VabAssetLabel::Button("login_button").from_asset("login.swf")),
                ),
                Node {
                    width: px(180),
                    ..default()
                },
            ));
        });
}
