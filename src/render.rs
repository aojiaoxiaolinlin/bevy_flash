mod diagnostics;
mod extract;
mod gpu;
mod instance;
mod pool;
mod texture_cache;

use bevy::{
    camera::ImageRenderTarget,
    color::Srgba,
    core_pipeline::schedule::camera_driver,
    ecs::schedule::{Schedule, ScheduleBuildSettings, ScheduleLabel},
    prelude::*,
    render::{
        ExtractSchedule, GpuResourceAppExt, Render, RenderApp, RenderSystems,
        extract_component::{ExtractComponent, ExtractComponentPlugin},
        extract_resource::ExtractResourcePlugin,
        render_resource::{TextureFormat, TextureUsages},
        renderer::{CurrentView, RenderGraph, RenderGraphSystems},
        view::{ExtractedView, Msaa},
    },
};
pub use diagnostics::VabDiagnosticsPlugin;
pub use gpu::FlashRenderDiagnostics;
pub use instance::{
    VAB_FILTER_DIAGNOSTIC_SLOTS, VabFilterCacheSettings, VabFilterMsaa, VabFilterWorkload,
    VabFilterWorkloadSample,
};
pub use texture_cache::TransientTexturePoolSettings;

pub struct FlashRenderPlugin;
impl Plugin for FlashRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            ExtractComponentPlugin::<OffscreenViewTarget>::default(),
            ExtractComponentPlugin::<RasterOnce>::default(),
            ExtractResourcePlugin::<TransientTexturePoolSettings>::default(),
            instance::VabInstanceRenderPlugin,
        ))
        .init_resource::<TransientTexturePoolSettings>();
        if let Some(render) = app.get_sub_app_mut(RenderApp) {
            render
                .init_gpu_resource::<texture_cache::FrameInternalTextureCache>()
                .init_gpu_resource::<gpu::FlashGpu>()
                .init_resource::<FlashRenderDiagnostics>()
                .init_resource::<gpu::OffscreenUniformBuffers>()
                .add_systems(ExtractSchedule, extract::extract_frames)
                .add_systems(
                    Render,
                    gpu::prepare_frames.in_set(RenderSystems::PrepareBindGroups),
                )
                .add_schedule(FilterCore::base_schedule())
                .add_systems(FilterCore, gpu::render_frame)
                .add_schedule(VabInstanceFilterCore::base_schedule())
                .add_systems(VabInstanceFilterCore, instance::render_vab_instance_filters)
                .add_systems(
                    RenderGraph,
                    filter_driver
                        .in_set(RenderGraphSystems::Render)
                        .before(camera_driver),
                );
        }
    }
}

#[derive(Component, Default)]
pub struct RenderOffscreenTexture;

/// Attach to the same entity as VabAssetHandle and VabPlayer.
/// `origin` is the output-pixel coordinate mapped to the image's top-left corner.
#[derive(Component, ExtractComponent, Clone)]
#[require(Msaa, RenderOffscreenTexture)]
pub struct OffscreenViewTarget {
    pub target: ImageRenderTarget,
    pub size: UVec2,
    pub scale: Vec2,
    pub origin: Vec2,
    pub clear_color: Srgba,
}

impl OffscreenViewTarget {
    pub fn new(image: Handle<Image>, size: UVec2) -> Self {
        Self {
            target: ImageRenderTarget {
                handle: image,
                scale_factor: 1.0,
            },
            size,
            scale: Vec2::ONE,
            origin: Vec2::ZERO,
            clear_color: Srgba::NONE,
        }
    }

    /// Straight-alpha sRGB Image, ready for Sprite/UI or a host material.
    pub fn create_image(size: UVec2) -> Image {
        let mut image =
            Image::new_target_texture(size.x, size.y, TextureFormat::Rgba8UnormSrgb, None);
        image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
        image
    }
}

#[derive(ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct FilterCore;
impl FilterCore {
    pub fn base_schedule() -> Schedule {
        let mut schedule = Schedule::new(Self);
        schedule.set_build_settings(ScheduleBuildSettings {
            auto_insert_apply_deferred: false,
            ..Default::default()
        });
        schedule
    }
}

#[derive(ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash, Default)]
struct VabInstanceFilterCore;
impl VabInstanceFilterCore {
    fn base_schedule() -> Schedule {
        let mut schedule = Schedule::new(Self);
        schedule.set_build_settings(ScheduleBuildSettings {
            auto_insert_apply_deferred: false,
            ..Default::default()
        });
        schedule
    }
}

pub fn filter_driver(world: &mut World) {
    let settings = world.resource::<TransientTexturePoolSettings>().clone();
    world
        .resource_mut::<texture_cache::FrameInternalTextureCache>()
        .begin_frame(&settings);
    let instance_view = world
        .query_filtered::<Entity, (With<Camera2d>, With<ExtractedView>)>()
        .iter(world)
        .next();
    if let Some(entity) = instance_view {
        world.insert_resource(CurrentView(entity));
        world.run_schedule(VabInstanceFilterCore);
    }
    let entities = world
        .query_filtered::<Entity, With<OffscreenViewTarget>>()
        .iter(world)
        .collect::<Vec<_>>();
    for entity in entities {
        world.insert_resource(CurrentView(entity));
        world.run_schedule(FilterCore);
    }
    world.remove_resource::<CurrentView>();
}
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
// Shared completion token: set only after successful render command encoding.
// Pending GPU assets keep the job alive; no guessed frame delay is used.
#[derive(Component, Clone, Default, bevy::render::extract_component::ExtractComponent)]
pub(crate) struct RasterOnce(pub Arc<AtomicBool>);
impl RasterOnce {
    pub fn complete(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}
