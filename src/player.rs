use crate::{assets::Swf, swf_runtime::movie_clip::MovieClip};
use bevy::{
    asset::{AsAssetId, AssetId, Handle},
    log::error,
    prelude::{
        Component, Deref, DerefMut, ReflectComponent, ReflectDefault, Transform, Visibility,
    },
    reflect::Reflect,
    time::{Timer, TimerMode},
};

/// Flash 播放器组件模块，定义了与 Flash 动画播放相关的组件和逻辑。
#[derive(Component, Debug, Clone, Reflect)]
#[require(FlashPlayerTimer)]
#[reflect(Component, Default, Debug)]
pub struct FlashPlayer {
    looping: bool,
    current_animation: Option<String>,
    speed: f32,
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

    pub fn from_looping(looping: bool) -> Self {
        Self {
            looping,
            ..Default::default()
        }
    }

    pub fn from_speed(speed: f32) -> Self {
        if speed <= 0.0 {
            error!("Speed must be greater than 0.0");
            return Self::default();
        }
        Self {
            speed,
            ..Default::default()
        }
    }

    pub fn with_looping(mut self, looping: bool) -> Self {
        self.looping = looping;
        self
    }

    pub fn with_speed(mut self, speed: f32) -> Self {
        if speed <= 0.0 {
            error!("Speed must be greater than 0.0");
            return self;
        }
        self.speed = speed;
        self
    }

    pub fn with_animation_name(mut self, animation_name: impl Into<String>) -> Self {
        self.current_animation = Some(animation_name.into());
        self
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

    pub fn speed(&self) -> f32 {
        self.speed
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

impl Default for FlashPlayer {
    fn default() -> Self {
        Self {
            looping: false,
            current_animation: None,
            speed: 1.0,
            total_frames: 0,
            current_frame: 0,
            completed: false,
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

impl AsAssetId for Flash {
    type Asset = Swf;

    fn as_asset_id(&self) -> AssetId<Self::Asset> {
        self.id()
    }
}

/// Flash动画都默认设置为30FPS
#[derive(Component, Debug, Clone, Deref, DerefMut)]
pub struct FlashPlayerTimer(Timer);

impl Default for FlashPlayerTimer {
    /// 动画定时器，默认30fps
    fn default() -> Self {
        Self(Timer::from_seconds(1. / 30., TimerMode::Repeating))
    }
}
