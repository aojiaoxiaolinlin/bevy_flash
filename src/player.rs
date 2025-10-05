use crate::{assets::Swf, swf_runtime::movie_clip::MovieClip};
use bevy::{
    asset::Handle,
    log::error,
    prelude::{
        Component, Deref, DerefMut, ReflectComponent, ReflectDefault, Transform, Visibility,
    },
    reflect::Reflect,
};

/// Flash 播放器组件模块，定义了与 Flash 动画播放相关的组件和逻辑。
#[derive(Component, Default, Debug, Clone, Reflect)]
#[reflect(Component, Default, Debug)]
pub struct FlashPlayer {
    pub looping: bool,
    pub current_animation: Option<String>,
    total_frames: u16,
    current_frame: u16,
    /// 是否完成，用于标记触发一次触发完成事件
    completed: bool,
}

impl FlashPlayer {
    pub fn from_animation_name(animation_name: impl Into<String>) -> Self {
        Self {
            current_animation: Some(animation_name.into()),
            ..Default::default()
        }
    }

    pub fn with_looping(mut self, looping: bool) -> Self {
        self.looping = looping;
        self
    }

    pub fn from_looping(looping: bool) -> Self {
        Self {
            looping,
            ..Default::default()
        }
    }

    pub fn reset(&mut self) {
        self.current_frame = 1;
        self.completed = false;
    }

    pub fn completed(&self) -> bool {
        self.completed
    }

    pub fn set_completed(&mut self, completed: bool) {
        self.completed = completed;
    }

    pub fn is_looping(&self) -> bool {
        self.looping
    }

    pub fn current_animation(&self) -> Option<&str> {
        self.current_animation.as_deref()
    }

    pub fn set_looping(&mut self, looping: bool) {
        self.looping = looping;
    }

    pub fn is_completed(&mut self) -> bool {
        self.current_frame >= self.total_frames
    }

    fn set_total_frames(&mut self, total_frames: u16) {
        self.total_frames = total_frames;
    }

    pub fn current_frame(&self) -> u16 {
        self.current_frame
    }

    pub fn total_frames(&self) -> u16 {
        self.total_frames
    }

    pub fn incr_frame(&mut self) {
        self.current_frame += 1;
    }

    pub fn set_play(&mut self, name: &str, swf: &Swf, root: &mut McRoot) {
        self.current_animation = Some(name.to_owned());
        self.play_target_animation(swf, root);
    }

    pub(crate) fn play_target_animation(&mut self, swf: &Swf, root: &mut McRoot) {
        if let Some(name) = &self.current_animation {
            match swf.animations().get(name.as_str()) {
                Some((frame, total_frames)) => {
                    root.goto_frame(swf.characters(), *frame, false);
                    self.reset();
                    self.set_total_frames(*total_frames);
                }
                None => {
                    error!("Animation '{}' not found", name);
                }
            }
        } else {
            self.reset();
            self.total_frames = root.total_frames();
        }
    }
}

/// Flash 动画中的根 影片 需要通过它来控制动画的播放
#[derive(Debug, Clone, Component, DerefMut, Deref)]
pub struct McRoot(pub MovieClip);

#[derive(Debug, Clone, Component, Default, Reflect, Deref, DerefMut)]
#[require(FlashPlayer, Transform, Visibility)]
#[reflect(Component, Default)]
pub struct Flash(pub Handle<Swf>);
