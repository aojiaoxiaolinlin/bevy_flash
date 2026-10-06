//! Opt-in VAB profiling. This module collects measurements but never logs them.
use bevy::{
    diagnostic::{Diagnostic, DiagnosticPath, Diagnostics, RegisterDiagnostic},
    prelude::*,
    render::{RenderApp, diagnostic::RenderDiagnosticsPlugin},
};

use super::{FlashRenderDiagnostics, VabFilterWorkload};

/// Enables VAB CPU timings, workload history and Bevy diagnostics.
///
/// Add alongside `FlashPlayerPlugin` after Bevy's `DefaultPlugins`.
/// No performance logs are printed; use `DiagnosticsStore` or
/// `VabFilterWorkload` from your own UI/logger. GPU timings also require
/// timestamp support on the graphics adapter.
///
/// ```no_run
/// use bevy::prelude::*;
/// use bevy_flash::{FlashPlayerPlugin, render::VabDiagnosticsPlugin};
/// App::new()
///     .add_plugins((DefaultPlugins, FlashPlayerPlugin, VabDiagnosticsPlugin::default()))
///     .run();
/// ```
///
/// Without this plugin, VAB skips CPU timing, workload-tree traversal, history
/// publication and GPU frame/timestamp readback. Lightweight internal render
/// counters and the texture pool's budget accounting remain available internally.
/// GPU results use Bevy's `render/vab_filter_prepass_*/elapsed_gpu` paths;
/// pair them with the frame markers as shown in `spirit_diagnostics`.
pub struct VabDiagnosticsPlugin {
    /// Install Bevy's render diagnostics if not already installed. Set false
    /// for CPU/workload profiling without enabling GPU timestamp collection.
    pub gpu_timing: bool,
}

impl Default for VabDiagnosticsPlugin {
    fn default() -> Self {
        Self { gpu_timing: true }
    }
}

impl VabDiagnosticsPlugin {
    pub const FILTER_PASSES: DiagnosticPath = DiagnosticPath::const_new("vab/filter/passes");
    pub const FILTER_PIXELS: DiagnosticPath = DiagnosticPath::const_new("vab/filter/pixels");
    pub const CACHE_HITS: DiagnosticPath = DiagnosticPath::const_new("vab/filter/cache_hits");
    pub const CACHE_MISSES: DiagnosticPath = DiagnosticPath::const_new("vab/filter/cache_misses");
    pub const CACHE_BYTES: DiagnosticPath = DiagnosticPath::const_new("vab/filter/cache_bytes");
    pub const CPU_MS: DiagnosticPath = DiagnosticPath::const_new("vab/cpu_ms");
    pub const PREPASS_MS: DiagnosticPath = DiagnosticPath::const_new("vab/filter/prepass_ms");
    pub const POOL_BYTES: DiagnosticPath = DiagnosticPath::const_new("vab/pool/resident_bytes");
    pub const ALLOCATED_BYTES: DiagnosticPath =
        DiagnosticPath::const_new("vab/pool/allocated_bytes");
    pub const LIVE_PEAK_BYTES: DiagnosticPath =
        DiagnosticPath::const_new("vab/pool/live_peak_bytes");
}

impl Plugin for VabDiagnosticsPlugin {
    fn build(&self, app: &mut App) {
        if self.gpu_timing && !app.is_plugin_added::<RenderDiagnosticsPlugin>() {
            app.add_plugins(RenderDiagnosticsPlugin);
        }
        app.init_resource::<VabFilterWorkload>();
        let workload = app.world().resource::<VabFilterWorkload>().clone();
        if let Some(render) = app.get_sub_app_mut(RenderApp) {
            render.insert_resource(workload);
        }
        for (path, suffix) in [
            (Self::FILTER_PASSES, ""),
            (Self::FILTER_PIXELS, ""),
            (Self::CACHE_HITS, ""),
            (Self::CACHE_MISSES, ""),
            (Self::CACHE_BYTES, " B"),
            (Self::CPU_MS, " ms"),
            (Self::PREPASS_MS, " ms"),
            (Self::POOL_BYTES, " B"),
            (Self::ALLOCATED_BYTES, " B"),
            (Self::LIVE_PEAK_BYTES, " B"),
        ] {
            app.register_diagnostic(Diagnostic::new(path).with_suffix(suffix));
        }
        app.add_systems(Update, publish_metrics);
    }

    fn finish(&self, app: &mut App) {
        if let Some(render) = app.get_sub_app_mut(RenderApp) {
            let mut diagnostics = render.world_mut().resource_mut::<FlashRenderDiagnostics>();
            diagnostics.enabled = true;
            diagnostics.gpu_timing = self.gpu_timing;
        }
    }
}

fn publish_metrics(
    workload: Res<VabFilterWorkload>,
    mut diagnostics: Diagnostics,
    mut last: Local<u64>,
) {
    // Publish each render sample once, including when the render world runs
    // ahead of the main world. Missing GPU results are never synthesized here.
    for (sample, _) in workload.recent_since(*last) {
        *last = sample.frame;
        let cpu = sample.extract_cpu_ns
            + sample.queue_cpu_ns
            + sample.prepare_buffers_cpu_ns
            + sample.prepare_bind_groups_cpu_ns
            + sample.prepass_cpu_ns;
        for (path, value) in [
            (VabDiagnosticsPlugin::FILTER_PASSES, sample.passes as f64),
            (
                VabDiagnosticsPlugin::FILTER_PIXELS,
                sample.processed_pixels as f64,
            ),
            (VabDiagnosticsPlugin::CACHE_HITS, sample.cache_hits as f64),
            (
                VabDiagnosticsPlugin::CACHE_MISSES,
                sample.cache_misses as f64,
            ),
            (
                VabDiagnosticsPlugin::CACHE_BYTES,
                sample.filter_cache_bytes as f64,
            ),
            (VabDiagnosticsPlugin::CPU_MS, cpu as f64 / 1e6),
            (
                VabDiagnosticsPlugin::PREPASS_MS,
                sample.prepass_cpu_ns as f64 / 1e6,
            ),
            (
                VabDiagnosticsPlugin::ALLOCATED_BYTES,
                sample.texture_allocated_bytes as f64,
            ),
            (
                VabDiagnosticsPlugin::LIVE_PEAK_BYTES,
                sample.peak_live_transient_bytes as f64,
            ),
        ] {
            diagnostics.add_measurement(&path, || value);
        }
        if let Some(bytes) = sample.pool_end_bytes {
            diagnostics.add_measurement(&VabDiagnosticsPlugin::POOL_BYTES, || bytes as f64);
        }
    }
}
