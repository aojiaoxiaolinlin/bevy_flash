use bevy::{
    asset::Handle,
    prelude::{Component, Entity, Transform, Visibility},
    utils::hashbrown::HashMap,
};
use swf::{CharacterId, Depth};

use crate::{
    assets::SwfMovie,
    swf::display_object::{movie_clip::MovieClip, TDisplayObject},
};
#[derive(Component, Default)]
#[require(Transform, Visibility)]
pub struct SwfBundle {
    /// 要渲染的swf资源的引用计数句柄。
    pub swf_handle: Handle<SwfMovie>,
    /// 根movie_clip对象
    pub swf: Swf,
    /// shape对应实体
    pub shape_mark_entities: ShapeMarkEntities,
}

#[derive(Default, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ShapeMark {
    // 记录shape被多次引用的情况
    pub graphic_ref_count: u8,
    pub depth: Depth,
    pub id: CharacterId,
}
#[derive(Component, Default)]
pub struct ShapeMarkEntities {
    graphic_entities: HashMap<ShapeMark, Entity>,
    current_frame_entities: Vec<ShapeMark>,
}

impl ShapeMarkEntities {
    pub fn entity(&self, shape_mark: &ShapeMark) -> Option<&Entity> {
        self.graphic_entities.get(shape_mark)
    }

    pub fn add_entities_pool(&mut self, shape_mark: ShapeMark, entity: Entity) {
        self.graphic_entities.insert(shape_mark, entity);
    }

    pub fn record_current_frame_entity(&mut self, shape_mark: ShapeMark) {
        self.current_frame_entities.push(shape_mark);
    }

    pub fn clear_current_frame_entity(&mut self) {
        self.current_frame_entities.clear();
    }

    pub fn graphic_entities(&self) -> &HashMap<ShapeMark, bevy::prelude::Entity> {
        &self.graphic_entities
    }

    pub fn current_frame_entities(&self) -> &Vec<ShapeMark> {
        &self.current_frame_entities
    }
}

#[derive(Component)]
pub struct Swf {
    pub root_movie_clip: MovieClip,
    /// 要渲染和控制的movie_clip，子影片默认为根影片
    pub name: Option<String>,
    /// 加载处理状态
    pub status: SwfState,
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
            status: Default::default(),
        }
    }
}

#[derive(Default)]
pub enum SwfState {
    #[default]
    Loading,
    Ready,
}

#[derive(Default, Component)]
pub struct SwfGraphicComponent;
