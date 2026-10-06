//! Shared headless app setup, output allocation and GPU readback.
use super::*;

pub(super) fn app(path: PathBuf) -> App {
    app_with_render_diagnostics(path, false)
}

pub(super) fn app_with_render_diagnostics(path: PathBuf, render_diagnostics: bool) -> App {
    app_with_diagnostic_options(path, render_diagnostics, true)
}

pub(super) fn app_with_diagnostic_options(
    path: PathBuf,
    render_diagnostics: bool,
    vab_diagnostics: bool,
) -> App {
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit,
                ..default()
            })
            .set(AssetPlugin {
                file_path: path.to_string_lossy().into(),
                ..default()
            })
            .disable::<WinitPlugin>()
            .disable::<PipelinedRenderingPlugin>(),
    )
    .add_plugins(FlashPlayerPlugin);
    if vab_diagnostics {
        app.add_plugins(VabDiagnosticsPlugin {
            gpu_timing: render_diagnostics,
        });
    } else if render_diagnostics {
        app.add_plugins(bevy::render::diagnostic::RenderDiagnosticsPlugin);
    }
    #[cfg(feature = "ui")]
    app.add_plugins(bevy_flash_remake::vab_ui::VabUiPlugin);
    while app.plugins_state() != bevy::app::PluginsState::Ready {
        app.update();
    }
    app.finish();
    app.cleanup();
    app
}

pub(super) fn capture(app: &mut App, image: Handle<Image>, size: UVec2) -> Vec<u8> {
    let result = Arc::new(Mutex::new(None));
    let out = result.clone();
    let readback = app
        .world_mut()
        .spawn(Readback::texture(image))
        .observe(move |event: On<ReadbackComplete>| {
            *out.lock().unwrap() = Some(event.data.clone());
        })
        .id();
    let start = Instant::now();
    let baseline = app
        .sub_app(RenderApp)
        .world()
        .resource::<FlashRenderDiagnostics>()
        .rendered_views;
    let mut rendered_at = None;
    loop {
        app.update();
        let diagnostics = app
            .sub_app(RenderApp)
            .world()
            .resource::<FlashRenderDiagnostics>();
        assert!(
            diagnostics.errors.is_empty(),
            "render errors: {:?}",
            diagnostics.errors
        );
        if diagnostics.rendered_views > baseline && rendered_at.is_none() {
            rendered_at = Some(Instant::now());
        }
        if rendered_at.is_some_and(|time| time.elapsed() > Duration::from_millis(200))
            && let Some(bytes) = result.lock().unwrap().take()
        {
            app.world_mut().despawn(readback);
            let stride = RenderDevice::align_copy_bytes_per_row(size.x as usize * 4);
            let mut tight = Vec::new();
            for row in bytes.chunks(stride).take(size.y as usize) {
                tight.extend_from_slice(&row[..size.x as usize * 4]);
            }
            return tight;
        }
        assert!(
            start.elapsed() < Duration::from_secs(45),
            "GPU capture timeout"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

pub(super) fn target(app: &mut App, size: UVec2) -> Handle<Image> {
    app.world_mut()
        .resource_mut::<Assets<Image>>()
        .add(OffscreenViewTarget::create_image(size))
}

pub(super) fn capture_camera(app: &mut App, image: Handle<Image>, size: UVec2) -> Vec<u8> {
    // Asset extraction and pipeline compilation are asynchronous. Do not let the
    // one-shot readback capture the first, intentionally empty warm-up frame.
    let warmup = Instant::now();
    loop {
        app.update();
        let diagnostics = app
            .sub_app(RenderApp)
            .world()
            .resource::<FlashRenderDiagnostics>();
        if diagnostics.vab_instances_queued > 0
            && diagnostics
                .vab_draw_calls
                .load(std::sync::atomic::Ordering::Relaxed)
                > 0
        {
            break;
        }
        assert!(
            warmup.elapsed() < Duration::from_secs(45),
            "VAB instance pipeline warm-up timeout: {diagnostics:?}"
        );
    }
    let result = Arc::new(Mutex::new(None));
    let out = result.clone();
    let readback = app
        .world_mut()
        .spawn(Readback::texture(image))
        .observe(move |event: On<ReadbackComplete>| {
            *out.lock().unwrap() = Some(event.data.clone());
        })
        .id();
    let start = Instant::now();
    loop {
        app.update();
        if let Some(bytes) = result.lock().unwrap().take() {
            app.world_mut().despawn(readback);
            let stride = RenderDevice::align_copy_bytes_per_row(size.x as usize * 4);
            let mut tight = Vec::new();
            for row in bytes.chunks(stride).take(size.y as usize) {
                tight.extend_from_slice(&row[..size.x as usize * 4]);
            }
            return tight;
        }
        assert!(
            start.elapsed() < Duration::from_secs(45),
            "GPU capture timeout"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
