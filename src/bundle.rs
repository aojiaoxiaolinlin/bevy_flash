use bevy::{
    asset::Handle,
    platform_support::collections::HashMap,
    prelude::{
        Component, Deref, DerefMut, Entity, ReflectComponent, ReflectDefault, Transform, Visibility,
    },
    reflect::Reflect,
};

use crate::assets::FlashAnimationSwfData;

/// 用于记录以及生成的Shape，缓存起来，某一帧需要时再显示
#[derive(Component, Debug, Clone, Default, Reflect, DerefMut, Deref)]
#[reflect(Component, Default, Debug)]
pub struct FlashShapeSpawnRecord(HashMap<String, Entity>);

impl FlashShapeSpawnRecord {
    pub fn is_generate(&self, id: &String) -> bool {
        self.contains_key(id)
    }

    pub fn cache_entities(&self) -> &HashMap<String, Entity> {
        &self
    }

    pub fn get_entity(&self, key: &String) -> Option<&Entity> {
        self.get(key)
    }

    pub fn mark_cached_shape(&mut self, id: &String, entity: Entity) {
        self.insert(id.clone(), entity);
    }
}

#[derive(Default, Component)]
#[require(Transform, Visibility)]
pub struct SwfGraph;

#[derive(Component, Debug, Clone, Reflect)]
#[require(Transform, Visibility, FlashShapeSpawnRecord)]
#[reflect(Component, Default, Debug)]
pub struct FlashAnimation {
    /// 要渲染的swf资源的引用计数句柄。
    pub swf_asset: Handle<FlashAnimationSwfData>,
    /// 要渲染和控制的movie_clip，影片默认为根影片
    pub name: Option<String>,
    /// 是否应用根影片的变换 默认为true，不会应用根影片的变换; 若为false则会应用根影片的变换
    pub ignore_root_swf_transform: bool,
}

impl Default for FlashAnimation {
    fn default() -> Self {
        Self {
            swf_asset: Default::default(),
            name: None,
            ignore_root_swf_transform: true,
        }
    }
}
