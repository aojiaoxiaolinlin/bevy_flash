//! Static vector UI: layout/DPI-sized rasterization with shared caching.
use bevy::prelude::*;
use bevy_flash::{
    FlashPlayerPlugin,
    vab_graphic::VabAssetLabel,
    vab_ui::{VabImageFit, VabImageNode, VabUiPlugin},
};

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, FlashPlayerPlugin, VabUiPlugin))
        .add_systems(Startup, setup)
        .run();
}
fn setup(mut commands: Commands, assets: Res<AssetServer>) {
    commands.spawn(Camera2d);
    let graphic = assets.load(VabAssetLabel::Graphic("name_kuang").from_asset("nameplate3.vab"));
    commands
        .spawn(Node {
            width: percent(100),
            height: percent(100),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            flex_direction: FlexDirection::Column,
            row_gap: px(32),
            ..default()
        })
        .with_children(|parent| {
            // Equal dimensions share a raster; the larger node gets its own sharper image.
            for width in [240.0, 240.0, 880.0] {
                parent.spawn((
                    VabImageNode::new(graphic.clone()),
                    Node {
                        width: px(width),
                        // Height is measured from the graphic aspect ratio.
                        ..default()
                    },
                ));
            }
            // Same square layout: preserve the artwork by default, or explicitly stretch.
            for fit in [VabImageFit::Contain, VabImageFit::Stretch] {
                let mut node = VabImageNode::new(graphic.clone());
                node.fit = fit;
                parent.spawn((
                    node,
                    Node {
                        width: px(160),
                        height: px(100),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.12, 0.12, 0.12)),
                ));
            }
        });
}
