use crate::assets::SwfLoader;
use crate::bundle::FlashAnimation;
use crate::render::FlashRenderPlugin;
use crate::render::material::{BitmapMaterial, GradientMaterial, SwfColorMaterial};

use assets::FlashAnimationSwfData;
use bevy::app::App;
use bevy::asset::AssetEvent;
use bevy::ecs::component::Component;
use bevy::ecs::schedule::IntoScheduleConfigs;
use bevy::ecs::system::Commands;
use bevy::prelude::{Deref, DerefMut, Entity, EventReader, Query};

use bevy::time::Time;
use bevy::{
    app::{Plugin, Update},
    asset::{AssetApp, Assets},
    prelude::{Res, ResMut},
};
use flash_an_runtime::core::RuntimeInstance;

pub mod assets;
pub mod bundle;
mod render;
pub mod swf;

pub struct FlashPlugin;

impl Plugin for FlashPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(FlashRenderPlugin)
            .init_asset::<FlashAnimationSwfData>()
            .init_asset_loader::<SwfLoader>()
            .add_systems(Update, (flash_update, flash_animation).chain());
    }
}

#[derive(Default, Component, Deref, DerefMut)]
pub struct FlashAnimationActiveInstance(Vec<RuntimeInstance>);

fn flash_update(
    mut commands: Commands,
    mut query: Query<(
        Entity,
        &FlashAnimation,
        Option<&mut FlashAnimationActiveInstance>,
    )>,
    mut swf_assets: ResMut<Assets<FlashAnimationSwfData>>,
    time: Res<Time>,
) {
    query
        .iter_mut()
        .for_each(|(entity, flash_animation, active_instance)| {
            if let Some(flash_asset) = swf_assets.get_mut(flash_animation.swf_asset.id()) {
                let player = &mut flash_asset.player;
                if let Some(mut active_instance) = active_instance {
                    player
                        .update(&mut active_instance, time.delta_secs())
                        .unwrap();
                } else {
                    let mut active_instance = FlashAnimationActiveInstance::default();
                    // 驱动动画
                    player
                        .update(&mut active_instance, time.delta_secs())
                        .unwrap();
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

#[derive(Clone)]
pub struct SwfMesh {
    pub positions: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
    pub colors: Vec<[f32; 4]>,
}

#[derive(Clone)]
pub struct ShapeMesh {
    pub mesh: SwfMesh,
    pub draw_type: ShapeDrawType,
}

fn flash_animation(
    mut query: Query<(Entity, &mut FlashAnimation)>,
    mut flash_assets: ResMut<Assets<FlashAnimationSwfData>>,
    mut flash_swf_data_events: EventReader<AssetEvent<FlashAnimationSwfData>>,
) {
    for event in flash_swf_data_events.read() {
        if let AssetEvent::LoadedWithDependencies { id } = event {
            if let Some((entity, mut flash_animation)) = query
                .iter_mut()
                .find(|(_, flash_animation)| flash_animation.swf_asset.id() == *id)
            {
                let flash_asset = flash_assets.get_mut(*id).unwrap();
                flash_asset
                    .player
                    .set_play_animation("default", true, None)
                    .unwrap();
                // flash_asset.player.set_skin("head", "4").unwrap();
            }
        }
    }
}
