//! Native button state selection; business actions belong to the host.
use super::VabImageNode;
use bevy::{
    picking::hover::Hovered,
    prelude::*,
    ui::{InteractionDisabled, Pressed},
    ui_widgets::Button,
};

/// Native vector button with Bevy's rectangular interaction and cached state images.
/// Add InteractionDisabled to show the up state and suppress state changes.
/// Business actions remain the application's responsibility.
#[derive(Component, Clone)]
#[require(Button, Hovered, VabImageNode::new(Handle::default()))]
pub struct VabButtonNode {
    pub button: Handle<crate::vab_button::VabButton>,
}
impl VabButtonNode {
    pub fn new(button: Handle<crate::vab_button::VabButton>) -> Self {
        Self { button }
    }
}

#[allow(clippy::type_complexity)]
pub(super) fn select_button_images(
    buttons: Res<Assets<crate::vab_button::VabButton>>,
    mut nodes: Query<(
        &VabButtonNode,
        &Hovered,
        Has<Pressed>,
        Has<InteractionDisabled>,
        &mut VabImageNode,
    )>,
) {
    for (node, hovered, pressed, disabled, mut image) in &mut nodes {
        let Some(button) = buttons.get(&node.button) else {
            continue;
        };
        let graphic = if disabled {
            &button.up
        } else if pressed {
            &button.down
        } else if hovered.get() {
            &button.over
        } else {
            &button.up
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
        for (hovered, pressed, expected) in [
            (false, false, &up),
            (true, false, &over),
            (true, true, &down),
        ] {
            let mut node = app.world_mut().entity_mut(entity);
            node.insert(Hovered(hovered));
            if pressed {
                node.insert(Pressed);
            } else {
                node.remove::<Pressed>();
            }
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
