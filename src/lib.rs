use std::collections::BTreeMap;

use crate::assets::SwfLoader;
use crate::render::FlashRenderPlugin;
use crate::render::material::{BitmapMaterial, GradientMaterial, SwfColorMaterial};

use assets::FlashAnimationSwfData;
use bevy::app::{App, PostUpdate};
use bevy::asset::{Asset, Handle};
use bevy::ecs::component::Component;
use bevy::ecs::system::Commands;
use bevy::platform::collections::HashMap;
use bevy::prelude::{Deref, DerefMut, Entity, Query};

use bevy::reflect::TypePath;
use bevy::time::Time;
use bevy::{
    app::Plugin,
    asset::{AssetApp, Assets},
    prelude::{Res, ResMut},
};
use bundle::FlashAnimation;
use flash_runtime::core::RuntimeInstance;
use flash_runtime::parser::DepthTimeline;
use swf::Depth;

pub mod assets;
pub mod bundle;
mod render;

pub struct FlashPlugin;

impl Plugin for FlashPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(FlashRenderPlugin)
            .init_asset::<FlashAnimationSwfData>()
            .init_asset_loader::<SwfLoader>()
            .add_systems(PostUpdate, advance_flash_animation);
    }
}

#[derive(Default, Debug, Clone, Asset, TypePath)]
pub struct Animation {
    pub duration: f32,
    pub timeline: BTreeMap<Depth, DepthTimeline>,
}

#[derive(Default, Debug, Clone, Asset, TypePath)]
pub struct MovieClip {
    pub duration: f32,
    pub timeline: BTreeMap<Depth, DepthTimeline>,
    pub skin_frames: HashMap<String, u32>,
}

#[derive(Clone)]
pub struct FlashActiveAnimation {
    pub frame_rate: f32,
    pub animations: HashMap<String, Handle<Animation>>,
    pub children_clip: HashMap<String, Handle<MovieClip>>,
}

#[derive(Clone, Component)]
pub struct FlashAnimationPlayer(pub FlashActiveAnimation);

#[derive(Default, Component, Deref, DerefMut)]
pub struct FlashAnimationActiveInstance(Vec<RuntimeInstance>);

fn advance_flash_animation(
    mut commands: Commands,
    mut query: Query<(
        Entity,
        &FlashAnimation,
        Option<&mut FlashAnimationActiveInstance>,
    )>,
    mut flash_asset: ResMut<Assets<FlashAnimationSwfData>>,
    time: Res<Time>,
) {
    query
        .iter_mut()
        .for_each(|(entity, flash_animation, active_instance)| {
            if let Some(flash) = flash_asset.get_mut(flash_animation.swf.id()) {
                let player = &mut flash.player;
                if let Some(mut active_instance) = active_instance {
                    player.update(&mut active_instance, time.delta_secs());
                } else {
                    let mut active_instance = FlashAnimationActiveInstance::default();
                    // 驱动动画
                    player.update(&mut active_instance, time.delta_secs());
                    commands.entity(entity).insert(active_instance);
                }
            }
        });
}

#[derive(Clone)]
pub enum ShapeDrawType {
    Color(SwfColorMaterial),
    Gradient(GradientMaterial),
    Bitmap(BitmapMaterial),
}
