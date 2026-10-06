//! Native button state selection; business actions belong to the host.
use super::VabImageNode;
use bevy::{prelude::*, ui::InteractionDisabled};

/// Native vector button with Bevy's rectangular interaction and cached state images.
/// Add InteractionDisabled to show the up state and suppress state changes.
/// Business actions remain the application's responsibility.
#[derive(Component, Clone)]
#[require(Button, VabImageNode::new(Handle::default()))]
pub struct VabButtonNode {
    pub button: Handle<crate::vab_button::VabButton>,
}
impl VabButtonNode {
    pub fn new(button: Handle<crate::vab_button::VabButton>) -> Self {
        Self { button }
    }
}

pub(super) fn select_button_images(
    buttons: Res<Assets<crate::vab_button::VabButton>>,
    mut nodes: Query<(
        &VabButtonNode,
        &Interaction,
        Has<InteractionDisabled>,
        &mut VabImageNode,
    )>,
) {
    for (node, interaction, disabled, mut image) in &mut nodes {
        let Some(button) = buttons.get(&node.button) else {
            continue;
        };
        let graphic = if disabled {
            &button.up
        } else {
            match interaction {
                Interaction::None => &button.up,
                Interaction::Hovered => &button.over,
                Interaction::Pressed => &button.down,
            }
        };
        if image.graphic != *graphic {
            image.graphic = graphic.clone();
        }
    }
}

#[cfg(test)]
mod button_tests {
    use super::*;
    use crate::vab_button::VabButton;
    use crate::vab_graphic::VabGraphic;
    #[test]
    fn button_selects_native_states_and_disabled_uses_up() {
        let mut graphics = Assets::<VabGraphic>::default();
        let mut graphic = || {
            graphics.add(VabGraphic {
                source_bounds: [0.0, 0.0, 100.0, 40.0],
                visual_bounds: [-50.0, -20.0, 100.0, 40.0],
                frame_count: 1,
                frame_rate: 24.0,
                render_asset: Handle::default(),
            })
        };
        let (up, over, down) = (graphic(), graphic(), graphic());
        let mut buttons = Assets::<VabButton>::default();
        let button = buttons.add(VabButton {
            up: up.clone(),
            over: over.clone(),
            down: down.clone(),
            hit_test: None,
        });
        let mut app = App::new();
        app.insert_resource(buttons)
            .add_systems(Update, select_button_images);
        let entity = app.world_mut().spawn(VabButtonNode::new(button)).id();
        for (interaction, expected) in [
            (Interaction::None, &up),
            (Interaction::Hovered, &over),
            (Interaction::Pressed, &down),
        ] {
            *app.world_mut().get_mut::<Interaction>(entity).unwrap() = interaction;
            app.update();
            assert_eq!(
                &app.world().get::<VabImageNode>(entity).unwrap().graphic,
                expected
            );
        }
        app.world_mut()
            .entity_mut(entity)
            .insert(InteractionDisabled);
        app.update();
        assert_eq!(app.world().get::<VabImageNode>(entity).unwrap().graphic, up);
        app.world_mut()
            .entity_mut(entity)
            .remove::<InteractionDisabled>();
        app.update();
        assert_eq!(
            app.world().get::<VabImageNode>(entity).unwrap().graphic,
            down
        );
    }
}
