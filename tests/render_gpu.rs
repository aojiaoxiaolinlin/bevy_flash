//! Run explicitly: cargo test --test render_gpu -- --ignored --nocapture
//! These tests require a graphics adapter. They never create an OS window.
use bevy::{
    asset::{AssetPlugin, RenderAssetUsages},
    camera::RenderTarget,
    diagnostic::{DiagnosticPath, DiagnosticsStore, FrameCount},
    prelude::*,
    render::{
        RenderApp,
        gpu_readback::{Readback, ReadbackComplete},
        pipelined_rendering::PipelinedRenderingPlugin,
        render_resource::{
            Extent3d, PrimitiveTopology, TextureDimension, TextureFormat, WgpuFeatures,
        },
        renderer::RenderDevice,
    },
    window::{ExitCondition, WindowPlugin},
    winit::WinitPlugin,
};
use bevy_flash::{
    FlashPlayerPlugin,
    material::BitmapMaterial,
    render::{
        FlashRenderDiagnostics, OffscreenViewTarget, TransientTexturePoolSettings,
        VAB_FILTER_DIAGNOSTIC_SLOTS, VabDiagnosticsPlugin, VabFilterCacheSettings, VabFilterMsaa,
        VabFilterWorkload,
    },
    vab_asset::{MeshMaterial, RenderMeshGroup, VabAsset, VabAssetHandle},
    vab_player::VabPlayer,
};

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use vatf::{
    animation::{
        AnimBevelFilter, AnimBlurFilter, AnimColorMatrixFilter, AnimConvolutionFilter,
        AnimDropShadowFilter, AnimFilter, AnimGlowFilter, AnimGradientFilter, AnimGradientRecord,
        AnimMatrix, AnimTransform,
    },
    baked::{BakedClip, BakedMovie, BakedNode},
};

use support::*;
#[path = "gpu/benchmarks.rs"]
mod benchmarks;
#[path = "gpu/diagnostics.rs"]
mod diagnostics;
#[path = "gpu/filters.rs"]
mod filters;
#[path = "gpu/instances.rs"]
mod instances;
#[path = "gpu/support.rs"]
mod support;
#[path = "gpu/ui.rs"]
mod ui;
