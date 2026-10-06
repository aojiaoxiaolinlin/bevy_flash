//! GPU coverage for diagnostics.
use super::*;

#[test]
#[ignore = "requires GPU"]
fn vab_diagnostics_are_opt_in_even_with_bevy_render_diagnostics() {
    let mut app = app_with_diagnostic_options(PathBuf::from("assets"), true, false);
    let image = target(&mut app, UVec2::splat(64));
    app.world_mut()
        .spawn((Camera2d, RenderTarget::Image(image.into())));
    for _ in 0..8 {
        app.update();
    }
    assert!(!app.world().contains_resource::<VabFilterWorkload>());
    assert!(
        !app.sub_app(RenderApp)
            .world()
            .contains_resource::<VabFilterWorkload>()
    );
    let diagnostics = app
        .sub_app(RenderApp)
        .world()
        .resource::<FlashRenderDiagnostics>();
    assert_eq!(diagnostics.vab_extract_cpu_ns, 0);
    assert_eq!(diagnostics.vab_queue_cpu_ns, 0);
    assert_eq!(diagnostics.vab_prepare_buffers_cpu_ns, 0);
    assert_eq!(diagnostics.vab_prepare_bind_groups_cpu_ns, 0);
    let store = app.world().resource::<DiagnosticsStore>();
    assert!(store.get(&VabDiagnosticsPlugin::CPU_MS).is_none());
    assert!(
        store
            .get(&DiagnosticPath::const_new("render/vab_filter_frame_low"))
            .is_none()
    );
}
