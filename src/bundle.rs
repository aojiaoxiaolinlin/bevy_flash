use bevy::{
    asset::Handle,
    prelude::{Bundle, Component, SpatialBundle},
};

use crate::{assets::SwfMovie, swf::display_object::movie_clip::MovieClip};

#[derive(Bundle, Default)]
pub struct SwfBundle {
    /// 要渲染的swf资源的引用计数句柄。
    pub swf_handle: Handle<SwfMovie>,
    // /// 实体的local变换属性。
    // pub transform: Transform,
    // /// 实体的global变换属性。
    // pub global_transform: GlobalTransform,
    // /// 用户指明的实体是否可见。
    // pub visibility: Visibility,
    // /// 实体在层次结构中是否可见。
    // pub inherited_visibility: InheritedVisibility,
    // /// 通过算法计算得出的实体是否可见并应被提取用于渲染的指示。 每帧的`PostUpdate`阶段都会被设置为false。
    // pub view_visibility: ViewVisibility,
    /// 包含实体的空间属性。
    pub spatial: SpatialBundle,
}

#[derive(Component, Default)]
pub struct Swf {
    pub root_movie_clip: MovieClip,
}
