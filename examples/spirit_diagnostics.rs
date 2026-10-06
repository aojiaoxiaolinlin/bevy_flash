//! Detailed filter profiling. Run with cargo run --example spirit_diagnostics.
//! SPIRIT_POOL_MIB selects 256/512/1024 MiB; SPIRIT_TRACE_LARGE_ALLOCATIONS=1 enables allocation tracing.

use std::{
    collections::{HashMap, VecDeque},
    time::Instant,
};

use bevy::{
    dev_tools::fps_overlay::FpsOverlayPlugin,
    diagnostic::{DiagnosticPath, DiagnosticsStore, FrameCount},
    prelude::*,
};
use bevy_flash_remake::{
    FlashPlayerPlugin,
    render::{
        TransientTexturePoolSettings, VAB_FILTER_DIAGNOSTIC_SLOTS, VabDiagnosticsPlugin,
        VabFilterMsaa, VabFilterWorkload, VabFilterWorkloadSample,
    },
    vab_asset::{VabAsset, VabAssetHandle},
    vab_player::{VabFrameEvent, VabPlayer},
};

#[derive(Component)]
struct Spirit;

#[derive(Component)]
struct StartWhenLoaded;

#[derive(Default)]
struct FilterLogState {
    elapsed: f32,
    last_frame: u64,
    peak_app_frame_ms: f32,
    peak_matched_app: Option<(f32, VabFilterWorkloadSample)>,
    pending_app: HashMap<u32, f32>,
    pending_render: HashMap<u32, VabFilterWorkloadSample>,
    slow_intervals_awaiting_gpu: HashMap<u64, f32>,
    gpu_by_render_frame: HashMap<u64, f64>,
    peak_filter: VabFilterWorkloadSample,
    peak_cpu: VabFilterWorkloadSample,
    peak_allocations: VabFilterWorkloadSample,
    peak_gpu: Option<(f64, VabFilterWorkloadSample)>,
    gpu_matches: usize,
    gpu_expired: usize,
    peak_transparent_gpu_ms: Option<f64>,
    last_transparent_gpu_at: Option<Instant>,
    pending_gpu: VecDeque<(VabFilterWorkloadSample, Instant)>,
}

fn cpu_ns(sample: &VabFilterWorkloadSample) -> u64 {
    sample
        .extract_cpu_ns
        .saturating_add(sample.queue_cpu_ns)
        .saturating_add(sample.prepare_buffers_cpu_ns)
        .saturating_add(sample.prepare_bind_groups_cpu_ns)
        .saturating_add(sample.prepass_cpu_ns)
}

fn record_matched_interval(
    log: &mut FilterLogState,
    milliseconds: f32,
    sample: VabFilterWorkloadSample,
) {
    if log
        .peak_matched_app
        .is_none_or(|(peak, _)| milliseconds > peak)
    {
        log.peak_matched_app = Some((milliseconds, sample));
    }
    if milliseconds >= 25.0 {
        if let Some(gpu_ms) = log.gpu_by_render_frame.remove(&sample.frame) {
            info!(
                "VAB slow interval GPU match: app_frame={} render_frame={} interval={milliseconds:.2} ms filter_gpu={gpu_ms:.2} ms",
                sample.app_frame, sample.frame,
            );
        } else {
            log.slow_intervals_awaiting_gpu
                .insert(sample.frame, milliseconds);
        }
        info!(
            "VAB slow app interval={milliseconds:.2} ms after app_frame={} render_frame={}: layers={} passes={} new={} allocated={:.1} MiB prepass={:.2} ms",
            sample.app_frame,
            sample.frame,
            sample.layers,
            sample.passes,
            sample.texture_allocations,
            sample.texture_allocated_bytes as f64 / 1024.0 / 1024.0,
            sample.prepass_cpu_ns as f64 / 1e6,
        );
    }
}

fn main() {
    let budget_mib = std::env::var("SPIRIT_POOL_MIB")
        .unwrap_or_else(|_| "256".to_string())
        .parse::<u64>()
        .expect("SPIRIT_POOL_MIB must be 256, 512, or 1024");
    assert!(
        matches!(budget_mib, 256 | 512 | 1024),
        "SPIRIT_POOL_MIB must be 256, 512, or 1024"
    );
    App::new()
        .insert_resource(ClearColor(Color::srgb_u8(102, 102, 102)))
        .insert_resource(VabFilterMsaa::Off)
        // Retain recurring target sizes across animation cycles. This example's
        // budget can be overridden with SPIRIT_POOL_MIB.
        .insert_resource(TransientTexturePoolSettings {
            max_resident_bytes: budget_mib * 1024 * 1024,
            max_unused_frames: 1000,
            trace_large_allocations: std::env::var("SPIRIT_TRACE_LARGE_ALLOCATIONS")
                .is_ok_and(|value| value == "1"),
        })
        .add_plugins((
            DefaultPlugins,
            FpsOverlayPlugin::default(),
            VabDiagnosticsPlugin::default(),
            FlashPlayerPlugin,
        ))
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (
                start_when_loaded,
                keyboard_control,
                handle_frame_events,
                log_filter_workload,
            ),
        )
        .run();
}

fn setup(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    pool_settings: Res<TransientTexturePoolSettings>,
) {
    // Keep the camera at 4x MSAA while measuring single-sampled filter targets at 2x scale.
    commands.spawn((
        Camera2d,
        Transform::from_xyz(0.0, 0.0, 0.0),
        CompositingSpace::Srgb,
        Msaa::Sample4,
    ));
    commands.spawn((
        Name::new("123620-2x"),
        Spirit,
        StartWhenLoaded,
        VabAssetHandle(asset_server.load("123620.vab")),
        VabPlayer::default(),
        Transform::from_scale(Vec3::splat(2.0)).with_translation(Vec3::new(0., 0., 0.)),
    ));

    info!("controls: Space = ATT, R = WAI, P = pause/resume");
    info!(
        "filter texture pool: resident budget={} MiB, unused lifetime={} frames, large-allocation trace={}",
        pool_settings.max_resident_bytes / 1024 / 1024,
        pool_settings.max_unused_frames,
        pool_settings.trace_large_allocations
    );
}

fn start_when_loaded(
    mut commands: Commands,
    assets: Res<Assets<VabAsset>>,
    mut spirits: Query<(Entity, &VabAssetHandle, &mut VabPlayer), With<StartWhenLoaded>>,
) {
    for (entity, handle, mut player) in &mut spirits {
        let Some(asset) = assets.get(&handle.0) else {
            continue;
        };
        if let Err(error) = player.set_fallback_loop(asset, "ATTACK") {
            warn!("ATTACK is unavailable: {error}");
        } else if let Err(error) = player.play_loop(asset, "ATTACK") {
            warn!("ATTACK is unavailable: {error}");
        }
        commands.entity(entity).remove::<StartWhenLoaded>();
    }
}

fn keyboard_control(
    keyboard: Res<ButtonInput<KeyCode>>,
    assets: Res<Assets<VabAsset>>,
    mut spirits: Query<(&VabAssetHandle, &mut VabPlayer), With<Spirit>>,
) {
    for (handle, mut player) in &mut spirits {
        let Some(asset) = assets.get(&handle.0) else {
            continue;
        };
        if keyboard.just_pressed(KeyCode::Space)
            && let Err(error) = player.play_once(asset, "ATT")
        {
            warn!("ATT is unavailable: {error}");
        }
        if keyboard.just_pressed(KeyCode::KeyR)
            && let Err(error) = player.play_loop(asset, "WAI")
        {
            warn!("WAI is unavailable: {error}");
        }
        if keyboard.just_pressed(KeyCode::KeyP) {
            if player.playing {
                player.pause();
            } else {
                player.resume();
            }
        }
    }
}

fn handle_frame_events(mut events: MessageReader<VabFrameEvent>) {
    for event in events.read() {
        info!(
            "entity={} animation={} frame={} event={}",
            event.entity, event.animation, event.frame, event.name
        );
    }
}

// Frame identity and timing must come from the same Bevy diagnostic batch.
fn matched_filter_gpu(diagnostics: &DiagnosticsStore, frame: u64) -> Option<f64> {
    let low = diagnostics
        .get(&DiagnosticPath::const_new("render/vab_filter_frame_low"))?
        .measurement()?;
    let high = diagnostics
        .get(&DiagnosticPath::const_new("render/vab_filter_frame_high"))?
        .measurement()?;
    let elapsed = diagnostics
        .get(&DiagnosticPath::new(format!(
            "render/vab_filter_prepass_{:03}/elapsed_gpu",
            frame % VAB_FILTER_DIAGNOSTIC_SLOTS
        )))?
        .measurement()?;
    let reported_frame = ((high.value as u64) << 32) | low.value as u64;
    (reported_frame == frame
        && low.time == high.time
        && low.time == elapsed.time
        && elapsed.value.is_finite()
        && elapsed.value >= 0.0)
        .then_some(elapsed.value)
}

fn gpu_duration(milliseconds: f64) -> String {
    if milliseconds < 0.01 {
        "<0.01 ms".to_string()
    } else {
        format!("{milliseconds:.2} ms")
    }
}

fn log_filter_workload(
    time: Res<Time>,
    frame_count: Res<FrameCount>,
    workload: Res<VabFilterWorkload>,
    diagnostics: Res<DiagnosticsStore>,
    mut log: Local<FilterLogState>,
) {
    // FrameCount increments in Last. At Update, this delta is the interval
    // following the previous update, whose render extraction bears this count.
    let app_frame = frame_count.0;
    let app_interval_ms = time.delta_secs() * 1000.0;
    if let Some(sample) = log.pending_render.remove(&app_frame) {
        record_matched_interval(&mut log, app_interval_ms, sample);
    } else {
        log.pending_app.insert(app_frame, app_interval_ms);
    }
    for (sample, published_at) in workload.recent_since(log.last_frame) {
        log.last_frame = sample.frame;
        if let Some(milliseconds) = log.pending_app.remove(&sample.app_frame) {
            record_matched_interval(&mut log, milliseconds, sample);
        } else {
            log.pending_render.insert(sample.app_frame, sample);
        }
        if (sample.passes, sample.processed_pixels)
            > (log.peak_filter.passes, log.peak_filter.processed_pixels)
        {
            log.peak_filter = sample;
        }
        if cpu_ns(&sample) > cpu_ns(&log.peak_cpu) {
            log.peak_cpu = sample;
        }
        if sample.texture_allocations > log.peak_allocations.texture_allocations {
            log.peak_allocations = sample;
        }
        if sample.passes > 0 {
            log.pending_gpu.push_back((sample, published_at));
        }
    }
    while log.pending_gpu.len() > VAB_FILTER_DIAGNOSTIC_SLOTS as usize {
        log.pending_gpu.pop_front();
        log.gpu_expired += 1;
    }
    log.pending_app
        .retain(|frame, _| app_frame.wrapping_sub(*frame) < VAB_FILTER_DIAGNOSTIC_SLOTS as u32);
    log.pending_render
        .retain(|frame, _| app_frame.wrapping_sub(*frame) < VAB_FILTER_DIAGNOSTIC_SLOTS as u32);
    let oldest_render_frame = log.last_frame.saturating_sub(VAB_FILTER_DIAGNOSTIC_SLOTS);
    log.slow_intervals_awaiting_gpu
        .retain(|frame, _| *frame >= oldest_render_frame);
    log.gpu_by_render_frame
        .retain(|frame, _| *frame >= oldest_render_frame);
    let mut peak_gpu = log.peak_gpu;
    let mut completed_gpu = Vec::new();
    let last_frame = log.last_frame;
    let mut expired = 0;
    log.pending_gpu.retain(|(sample, published_at)| {
        if let Some(milliseconds) = matched_filter_gpu(&diagnostics, sample.frame) {
            completed_gpu.push((*sample, milliseconds));
            if peak_gpu.is_none_or(|(peak, _)| milliseconds > peak) {
                peak_gpu = Some((milliseconds, *sample));
            }
            return false;
        }
        // A missing result is not zero GPU time. Expire unreported generations.
        let keep = last_frame.saturating_sub(sample.frame) < VAB_FILTER_DIAGNOSTIC_SLOTS
            && published_at.elapsed().as_secs() < 2;
        expired += usize::from(!keep);
        keep
    });
    log.gpu_expired += expired;
    log.gpu_matches += completed_gpu.len();
    log.peak_gpu = peak_gpu;
    for (sample, gpu_ms) in completed_gpu {
        if let Some(app_ms) = log.slow_intervals_awaiting_gpu.remove(&sample.frame) {
            info!(
                "VAB slow interval GPU match: app_frame={} render_frame={} interval={app_ms:.2} ms filter_gpu={gpu_ms:.2} ms",
                sample.app_frame, sample.frame,
            );
        } else {
            log.gpu_by_render_frame.insert(sample.frame, gpu_ms);
        }
    }
    let transparent_path = DiagnosticPath::const_new("render/main_transparent_pass_2d/elapsed_gpu");
    if let Some(measurement) = diagnostics
        .get(&transparent_path)
        .and_then(|entry| entry.measurement())
        && log
            .last_transparent_gpu_at
            .is_none_or(|last| measurement.time > last)
    {
        log.last_transparent_gpu_at = Some(measurement.time);
        if log
            .peak_transparent_gpu_ms
            .is_none_or(|peak| measurement.value > peak)
        {
            log.peak_transparent_gpu_ms = Some(measurement.value);
        }
    }
    log.peak_app_frame_ms = log.peak_app_frame_ms.max(time.delta_secs() * 1000.0);
    log.elapsed += time.delta_secs();
    if log.elapsed < 1.0 {
        return;
    }
    log.elapsed = 0.0;
    let gpu = log.peak_gpu.map_or_else(
        || "unavailable".to_string(),
        |(milliseconds, sample)| {
            format!(
                "{} (frame {}: {} passes, {} frames behind latest)",
                gpu_duration(milliseconds),
                sample.frame,
                sample.passes,
                log.last_frame.saturating_sub(sample.frame)
            )
        },
    );
    let filter = log.peak_filter;
    let cpu = log.peak_cpu;
    let allocations = log.peak_allocations;
    let transparent_gpu = log
        .peak_transparent_gpu_ms
        .map_or_else(|| "unavailable".to_string(), gpu_duration);
    let matched_app = log.peak_matched_app.map_or_else(
        || "unavailable".to_string(),
        |(milliseconds, sample)| {
            format!(
                "{milliseconds:.2} ms (app_frame {} render_frame {}: {} passes, {} new, {:.1} MiB allocated, {:.2} ms prepass)",
                sample.app_frame,
                sample.frame,
                sample.passes,
                sample.texture_allocations,
                sample.texture_allocated_bytes as f64 / 1024.0 / 1024.0,
                sample.prepass_cpu_ns as f64 / 1e6,
            )
        },
    );
    info!(
        "VAB peak filter frame={}: layers={} hits={} misses={} passes={} pixels={} cache={:.1} MiB; CPU peak frame={}: extract={:.2} queue={:.2} buffers={:.2} bind_groups={:.2} prepass={:.2} ms; filter GPU received peak={gpu} [matched={} expired={} pending={}]; transparent GPU interval peak={transparent_gpu}; texture allocations peak frame={}: new={} (first={} exhausted={}) reused={} allocated={:.1} MiB pool_work={:.1} MiB live_peak={:.1} MiB pool_end={:.1} MiB; app_frame_peak={:.2} ms matched_app_peak={matched_app}",
        filter.frame,
        filter.layers,
        filter.cache_hits,
        filter.cache_misses,
        filter.passes,
        filter.processed_pixels,
        filter.filter_cache_bytes as f64 / 1024.0 / 1024.0,
        cpu.frame,
        cpu.extract_cpu_ns as f64 / 1e6,
        cpu.queue_cpu_ns as f64 / 1e6,
        cpu.prepare_buffers_cpu_ns as f64 / 1e6,
        cpu.prepare_bind_groups_cpu_ns as f64 / 1e6,
        cpu.prepass_cpu_ns as f64 / 1e6,
        log.gpu_matches,
        log.gpu_expired,
        log.pending_gpu.len(),
        allocations.frame,
        allocations.texture_allocations,
        allocations.texture_first_in_bucket_allocations,
        allocations.texture_exhausted_bucket_allocations,
        allocations.texture_reuses,
        allocations.texture_allocated_bytes as f64 / 1024.0 / 1024.0,
        allocations.pooled_texture_bytes as f64 / 1024.0 / 1024.0,
        allocations.peak_live_transient_bytes as f64 / 1024.0 / 1024.0,
        allocations.pool_end_bytes.unwrap_or_default() as f64 / 1024.0 / 1024.0,
        log.peak_app_frame_ms,
    );
    log.peak_filter = VabFilterWorkloadSample::default();
    log.peak_cpu = VabFilterWorkloadSample::default();
    log.peak_allocations = VabFilterWorkloadSample::default();
    log.peak_gpu = None;
    log.gpu_matches = 0;
    log.gpu_expired = 0;
    log.peak_transparent_gpu_ms = None;
    log.peak_app_frame_ms = 0.0;
    log.peak_matched_app = None;
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::diagnostic::{Diagnostic, DiagnosticMeasurement};
    use std::time::Duration;

    fn store(frame: u64, mismatched_batch: bool) -> DiagnosticsStore {
        let mut store = DiagnosticsStore::default();
        let now = Instant::now();
        for (path, value, time) in [
            (
                "render/vab_filter_frame_low".to_string(),
                (frame as u32) as f64,
                now,
            ),
            (
                "render/vab_filter_frame_high".to_string(),
                (frame >> 32) as f64,
                now,
            ),
            (
                format!(
                    "render/vab_filter_prepass_{:03}/elapsed_gpu",
                    frame % VAB_FILTER_DIAGNOSTIC_SLOTS
                ),
                2.5,
                if mismatched_batch {
                    now - Duration::from_millis(1)
                } else {
                    now
                },
            ),
        ] {
            let mut diagnostic = Diagnostic::new(DiagnosticPath::new(path));
            diagnostic.add_measurement(DiagnosticMeasurement { time, value });
            store.add(diagnostic);
        }
        store
    }

    #[test]
    fn gpu_results_require_exact_frame_and_batch() {
        let frame = (1_u64 << 32) + 7;
        let diagnostics = store(frame, false);
        assert_eq!(matched_filter_gpu(&diagnostics, frame), Some(2.5));
        assert_eq!(
            matched_filter_gpu(&diagnostics, frame - VAB_FILTER_DIAGNOSTIC_SLOTS),
            None
        );
        assert_eq!(matched_filter_gpu(&store(frame, true), frame), None);
        assert_eq!(
            matched_filter_gpu(&DiagnosticsStore::default(), frame),
            None
        );
    }

    #[test]
    fn tiny_valid_gpu_measurements_are_not_displayed_as_missing_or_zero() {
        assert_eq!(gpu_duration(0.001), "<0.01 ms");
        assert_eq!(gpu_duration(2.5), "2.50 ms");
    }
}
