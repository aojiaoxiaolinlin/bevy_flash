//! Load Flash animations into the Bevy game engine.
//! This plugin supports loading Flash animations from SWF files and playing them in Bevy.
//! It also provides a player component to control the animation playback.
//!
//! ## Example
//! ```
//! use bevy::prelude::*;
//! use bevy_flash::FlashPlugin, Flash;
//!
//! let mut app = App::new();
//! app.add_plugins((DefaultPlugins, FlashPlugin))
//!    .add_systems(Startup, setup)
//!     .run();
//!
//! fn setup(mut commands: Commands, asset_server: Res<AssetServer>) {
//!     let swf_handle = asset_server.load("path/to/animation.swf");
//!     commands.spawn(Flash(swf_handle));
//! }
//! ```

mod animator;
pub mod assets;
mod commands;
pub mod player;
mod render;
pub mod shape;
pub(crate) mod swf_runtime;

use crate::{
    animator::{advance_animation, prepare_root_clip},
    assets::{Shape, Swf, SwfLoader},
    render::FlashRenderPlugin,
};

use bevy::{
    app::{App, Plugin, PostUpdate},
    asset::AssetApp,
    ecs::schedule::IntoScheduleConfigs,
    transform::TransformSystems,
};

pub use animator::{FlashCompleteEvent, FlashFrameEvent};

/// Flash 插件，为 Bevy 引入 Flash 动画。
pub struct FlashPlugin;

impl Plugin for FlashPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(FlashRenderPlugin)
            .init_asset::<Swf>()
            .init_asset::<Shape>()
            .init_asset_loader::<SwfLoader>()
            .add_systems(
                PostUpdate,
                (prepare_root_clip, advance_animation)
                    .chain()
                    .before(TransformSystems::Propagate),
            );
    }
}
