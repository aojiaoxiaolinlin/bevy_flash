//! Native SWF vector button: hover/press, shared raster cache and stable layout.
use bevy::prelude::*;
use bevy::ui::InteractionDisabled;
use bevy_flash::{
    FlashPlayerPlugin,
    vab_graphic::VabAssetLabel,
    vab_ui::{VabButtonNode, VabUiPlugin},
};

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, FlashPlayerPlugin, VabUiPlugin))
        .add_systems(Startup, setup)
        .add_observer(clicked)
        .add_systems(Update, toggle_disabled)
        .run();
}
fn setup(mut commands: Commands, assets: Res<AssetServer>) {
    commands.spawn(Camera2d);
    let button = assets.load(VabAssetLabel::Button("login_button").from_asset("login.vab"));
    commands
        .spawn(Node {
            width: percent(100),
            height: percent(100),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            column_gap: px(32),
            ..default()
        })
        .with_children(|parent| {
            for width in [180.0, 180.0, 360.0] {
                parent.spawn((
                    VabButtonNode::new(button.clone()),
                    Node {
                        width: px(width),
                        ..default()
                    },
                ));
            }
        });
    info!(
        "Hover and press the login buttons; Space toggles disabled. SWF hit geometry is retained; interaction uses the UI node rectangle."
    );
}
fn clicked(event: On<bevy::ui_widgets::Activate>, nodes: Query<(), With<VabButtonNode>>) {
    if nodes.contains(event.entity) {
        info!("login activated");
    }
}
fn toggle_disabled(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    nodes: Query<(Entity, Has<InteractionDisabled>), With<VabButtonNode>>,
) {
    if keys.just_pressed(KeyCode::Space) {
        for (entity, disabled) in &nodes {
            if disabled {
                commands.entity(entity).remove::<InteractionDisabled>();
            } else {
                commands.entity(entity).insert(InteractionDisabled);
            }
        }
    }
}
