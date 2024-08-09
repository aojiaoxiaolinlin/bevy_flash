use bevy::{
    asset::Handle,
    prelude::{Bundle, Component, ReflectComponent, ReflectDefault},
    reflect::Reflect,
    render::view::{InheritedVisibility, ViewVisibility, Visibility},
    sprite::Anchor,
    transform::components::{GlobalTransform, Transform},
};
use glam::Vec2;

use crate::assets::SwfMovie;

#[derive(Bundle, Default)]
pub struct SwfBundle {
    /// 指明实体的渲染属性。
    pub swf_sprite: SwfSprite,
    /// 要渲染的swf资源的引用计数句柄。
    pub swf: Handle<SwfMovie>,
    /// 实体的local变换属性。
    pub transform: Transform,
    /// 实体的global变换属性。
    pub global_transform: GlobalTransform,
    /// 用户指明的实体是否可见。
    pub visibility: Visibility,
    /// 实体在层次结构中是否可见。
    pub inherited_visibility: InheritedVisibility,
    /// 通过算法计算得出的实体是否可见并应被提取用于渲染的指示。 每帧的`PostUpdate`阶段都会被设置为false。
    pub view_visibility: ViewVisibility,
}

/// 指定swf sprite的渲染属性
#[derive(Component, Default, Debug, Clone, Reflect)]
#[reflect(Component, Default)]
#[repr(C)]
pub struct SwfSprite {
    /// 沿着 x 轴翻转。
    pub flip_x: bool,
    /// 沿着 y 轴翻转。
    pub flip_y: bool,
    /// 自定义sprite大小
    pub custom_size: Option<Vec2>,
    /// 锚点的位置
    pub anchor: Anchor,
}
