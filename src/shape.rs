use bevy::{
    asset::Handle,
    camera::visibility::Visibility,
    ecs::component::Component,
    prelude::{Deref, DerefMut, ReflectComponent, ReflectDefault},
    reflect::Reflect,
    transform::components::Transform,
};

use crate::assets::Shape;

#[derive(Debug, Clone, Default, Component, Deref, DerefMut, Reflect)]
#[require(Transform, Visibility)]
#[reflect(Component, Default)]
pub struct FlashShape(pub Handle<Shape>);
