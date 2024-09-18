use bevy::{
    asset::Handle,
    prelude::{Bundle, Component, SpatialBundle},
};

use crate::{
    assets::SwfMovie,
    swf::display_object::{movie_clip::MovieClip, TDisplayObject},
};

#[derive(Bundle, Default)]
pub struct SwfBundle {
    /// 要渲染的swf资源的引用计数句柄。
    pub swf_handle: Handle<SwfMovie>,
    /// 根movie_clip对象
    pub swf: Swf,
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

#[derive(Component)]
pub struct Swf {
    pub root_movie_clip: MovieClip,
    /// 要渲染和控制的movie_clip，子影片默认为根影片
    pub name: Option<String>,
}
impl Swf {
    /// 判断根影片是否为目标影片
    pub fn is_target_movie_clip(&self) -> bool {
        if self.root_movie_clip.name().unwrap_or("root")
            == self.name.clone().unwrap_or(String::from("root"))
        {
            true
        } else {
            false
        }
    }
}
impl Default for Swf {
    fn default() -> Self {
        Self {
            root_movie_clip: Default::default(),
            name: Some(String::from("root")),
        }
    }
}
