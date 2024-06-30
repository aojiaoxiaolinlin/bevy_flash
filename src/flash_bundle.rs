use bevy::{
    asset::Handle,
    prelude::Bundle,
    render::view::{InheritedVisibility, ViewVisibility, Visibility},
    transform::components::{GlobalTransform, Transform},
};

use crate::assets::FlashData;

#[derive(Bundle, Default)]
pub struct FlashBundle {
    pub flash: Handle<FlashData>,
    pub transform: Transform,
    pub global_transform: GlobalTransform,
    pub visibility: Visibility,
    pub inherited_visibility: InheritedVisibility,
    pub view_visibility: ViewVisibility,
}
