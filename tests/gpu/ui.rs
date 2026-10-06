//! GPU coverage for ui.
use super::*;

#[test]
#[cfg(feature = "ui")]
#[ignore = "requires GPU"]
fn hidden_ui_recreates_an_evicted_unfinished_request() {
    use bevy_flash::vab_ui::{VabImageNode, VabUiCacheSettings, VabUiSystems};
    #[derive(Resource, Default)]
    struct Stalled(bool);
    fn stall_first_request(
        mut commands: Commands,
        jobs: Query<Entity, With<OffscreenViewTarget>>,
        nodes: Query<Entity, With<VabImageNode>>,
        mut stalled: ResMut<Stalled>,
    ) {
        if !stalled.0
            && let Some(job) = jobs.iter().next()
        {
            // Simulate work that cannot complete before the node becomes hidden.
            commands.entity(job).remove::<OffscreenViewTarget>();
            for node in &nodes {
                commands.entity(node).insert(Visibility::Hidden);
            }
            stalled.0 = true;
        }
    }
    let mut app = app(PathBuf::from("assets"));
    app.init_resource::<Stalled>().add_systems(
        PostUpdate,
        stall_first_request.after(VabUiSystems::Rasterize),
    );
    *app.world_mut().resource_mut::<VabUiCacheSettings>() = VabUiCacheSettings {
        unused_frames: 0,
        idle_budget_bytes: 0,
        ..default()
    };
    let screen = target(&mut app, UVec2::new(800, 600));
    app.world_mut().spawn((
        Camera2d,
        IsDefaultUiCamera,
        RenderTarget::Image(screen.into()),
    ));
    let graphic = app
        .world()
        .resource::<AssetServer>()
        .load("ui_demo.vab#button_background");
    let node = app
        .world_mut()
        .spawn((
            VabImageNode::new(graphic),
            Node {
                width: px(240),
                ..default()
            },
        ))
        .id();
    let deadline = Instant::now() + Duration::from_secs(30);
    while !app.world().resource::<Stalled>().0 {
        app.update();
        assert!(
            Instant::now() < deadline,
            "initial raster was never requested"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    for _ in 0..10 {
        app.update();
    }
    app.world_mut().entity_mut(node).insert(Visibility::Visible);
    for _ in 0..60 {
        app.update();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_ne!(
        app.world().get::<ImageNode>(node).unwrap().image,
        bevy::image::TRANSPARENT_IMAGE_HANDLE,
        "visible node waited forever for a cancelled, evicted raster"
    );
}

#[test]
#[ignore = "requires GPU"]
fn ui_export_rasterizes_centered_graphic() {
    use bevy_flash::vab_graphic::VabGraphic;
    let mut app = app(PathBuf::from("assets"));
    let graphic: Handle<VabGraphic> = app
        .world()
        .resource::<AssetServer>()
        .load("ui_demo.vab#button_background");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        app.update();
        let server = app.world().resource::<AssetServer>();
        if let Some(bevy::asset::LoadState::Failed(error)) = server.get_load_state(&graphic) {
            panic!("{error}");
        }
        if server.is_loaded_with_dependencies(&graphic) {
            break;
        }
        assert!(Instant::now() < deadline);
    }
    let handle = app
        .world()
        .resource::<Assets<VabGraphic>>()
        .get(&graphic)
        .unwrap()
        .render_asset
        .clone();
    let size = UVec2::new(120, 60);
    let image = target(&mut app, size);
    let mut output = OffscreenViewTarget::new(image.clone(), size);
    output.origin = Vec2::new(-60.0, -30.0);
    let mut player = VabPlayer::default();
    player.pause();
    app.world_mut()
        .spawn((VabAssetHandle(handle), player, output, Msaa::Off));
    let pixels = capture(&mut app, image, size);
    for y in 0..60 {
        for x in 0..120 {
            let pixel = &pixels[(y * 120 + x) * 4..][..4];
            if (11..109).contains(&x) && (11..49).contains(&y) {
                assert!(
                    pixel[0].abs_diff(230) <= 2
                        && pixel[1].abs_diff(130) <= 2
                        && pixel[2].abs_diff(40) <= 2
                        && pixel[3] > 250,
                    "bad centered graphic at {x},{y}: {pixel:?}"
                );
            } else if !(9..111).contains(&x) || !(9..51).contains(&y) {
                assert_eq!(pixel[3], 0, "graphic escaped expected bounds at {x},{y}");
            }
        }
    }
}

#[test]
#[ignore = "requires GPU"]
fn nameplate_ui_renders_without_editable_text() {
    use bevy_flash::vab_graphic::VabGraphic;
    let directory = std::env::temp_dir().join("vab_nameplate_gpu");
    std::fs::create_dir_all(&directory).unwrap();
    vatf::convert_swf_ui_to_vab(
        std::path::Path::new("assets/nameplate3.swf"),
        &directory.join("nameplate3.vab"),
    )
    .unwrap();
    let mut app = app(directory);
    let graphic: Handle<VabGraphic> = app
        .world()
        .resource::<AssetServer>()
        .load("nameplate3.vab#name_kuang");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        app.update();
        let server = app.world().resource::<AssetServer>();
        if let Some(bevy::asset::LoadState::Failed(error)) = server.get_load_state(&graphic) {
            panic!("{error}");
        }
        if server.is_loaded_with_dependencies(&graphic) {
            break;
        }
        assert!(Instant::now() < deadline);
    }
    let graphic = app
        .world()
        .resource::<Assets<VabGraphic>>()
        .get(&graphic)
        .unwrap();
    let handle = graphic.render_asset.clone();
    let size = (graphic.size() * 4.0).ceil().as_uvec2();
    let asset = app
        .world()
        .resource::<Assets<VabAsset>>()
        .get(&handle)
        .unwrap();
    assert!(
        !asset.shape_map.contains_key(&7),
        "editable text must be absent"
    );
    let image = target(&mut app, size);
    let mut output = OffscreenViewTarget::new(image.clone(), size);
    output.scale = Vec2::splat(4.0);
    output.origin = -size.as_vec2() * 0.5;
    let mut player = VabPlayer::default();
    player.pause();
    app.world_mut()
        .spawn((VabAssetHandle(handle), player, output, Msaa::Sample4));
    let pixels = capture(&mut app, image, size);
    assert!(pixels.chunks_exact(4).filter(|p| p[3] > 0).count() > pixels.len() / 40);
    let path = std::env::temp_dir().join("vab_nameplate_ui.png");
    image::save_buffer(&path, &pixels, size.x, size.y, image::ColorType::Rgba8).unwrap();
    println!("nameplate GPU capture: {}", path.display());
}

#[test]
#[cfg(feature = "ui")]
#[ignore = "requires GPU"]
fn cached_ui_shares_rasters_stops_rendering_and_invalidates() {
    use bevy_flash::vab_ui::{VabImageNode, VabUiCacheSettings};
    let mut app = app(PathBuf::from("assets"));

    let screen = target(&mut app, UVec2::new(800, 600));
    app.world_mut().spawn((
        Camera2d,
        IsDefaultUiCamera,
        RenderTarget::Image(screen.into()),
    ));
    let graphic = app
        .world()
        .resource::<AssetServer>()
        .load("ui_demo.vab#button_background");
    let mut spawn = |width| {
        app.world_mut()
            .spawn((
                VabImageNode::new(graphic.clone()),
                Node {
                    width: px(width),
                    height: px(40),
                    ..default()
                },
            ))
            .id()
    };
    let a = spawn(100.0);
    let b = spawn(100.0);
    let c = spawn(200.0);
    fn settle(app: &mut App) {
        for _ in 0..80 {
            app.update();
            std::thread::sleep(Duration::from_millis(5));
        }
        let diagnostics = app
            .sub_app(RenderApp)
            .world()
            .resource::<FlashRenderDiagnostics>();
        assert!(diagnostics.errors.is_empty(), "{:?}", diagnostics.errors);
    }
    fn image(app: &App, e: Entity) -> Handle<Image> {
        app.world().get::<ImageNode>(e).unwrap().image.clone()
    }
    fn renders(app: &App) -> u64 {
        app.sub_app(RenderApp)
            .world()
            .resource::<FlashRenderDiagnostics>()
            .rendered_views
    }
    settle(&mut app);
    assert_eq!(image(&app, a), image(&app, b));
    assert_ne!(image(&app, a), image(&app, c));
    assert_ne!(image(&app, a), Handle::default());
    let baseline = renders(&app);
    assert!(baseline >= 2);
    settle(&mut app);
    assert_eq!(renders(&app), baseline, "static nodes must not redraw");
    app.world_mut().get_mut::<Node>(c).unwrap().width = px(100);
    settle(&mut app);
    assert_eq!(image(&app, a), image(&app, c));
    assert_eq!(
        renders(&app),
        baseline,
        "resizing to a cached size must reuse it"
    );
    app.world_mut().resource_mut::<UiScale>().0 = 2.0;
    settle(&mut app);
    let dpi_renders = renders(&app);
    assert!(dpi_renders > baseline, "DPI change must rerasterize");
    let previous = image(&app, a);
    let graphic_id = app.world().get::<VabImageNode>(a).unwrap().graphic.id();
    app.world_mut()
        .resource_mut::<Assets<bevy_flash::vab_graphic::VabGraphic>>()
        .get_mut(graphic_id)
        .unwrap()
        .source_bounds[0] -= 1.0;
    settle(&mut app);
    assert_ne!(
        image(&app, a),
        previous,
        "asset edits invalidate cached output"
    );
    assert!(renders(&app) > dpi_renders);
    app.world_mut()
        .resource_mut::<VabUiCacheSettings>()
        .unused_frames = 0;
    let final_image = image(&app, a).id();
    app.world_mut().despawn(a);
    app.world_mut().despawn(b);
    app.world_mut().despawn(c);
    settle(&mut app);
    assert!(
        app.world()
            .resource::<Assets<Image>>()
            .get(final_image)
            .is_none(),
        "idle output must be released"
    );
    assert_eq!(
        app.world_mut()
            .query::<&OffscreenViewTarget>()
            .iter(app.world())
            .count(),
        0
    );
}

#[test]
#[cfg(feature = "ui")]
#[ignore = "requires GPU"]
fn ui_intrinsic_size_and_aspect_fit() {
    use bevy_flash::vab_ui::{VabImageFit, VabImageNode};
    let mut app = app(PathBuf::from("assets"));
    let screen = target(&mut app, UVec2::new(800, 1000));
    app.world_mut().spawn((
        Camera2d,
        IsDefaultUiCamera,
        RenderTarget::Image(screen.into()),
    ));
    let graphic = app
        .world()
        .resource::<AssetServer>()
        .load("ui_demo.vab#button_background");
    let root = app
        .world_mut()
        .spawn(Node {
            width: percent(100),
            height: percent(100),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Start,
            ..default()
        })
        .id();
    let cases = [
        (Val::Auto, Val::Auto, VabImageFit::Contain),
        (px(200), Val::Auto, VabImageFit::Contain),
        (Val::Auto, px(80), VabImageFit::Contain),
        (px(200), px(200), VabImageFit::Contain),
        (px(200), px(200), VabImageFit::Stretch),
    ];
    let entities: Vec<_> = cases
        .into_iter()
        .map(|(width, height, fit)| {
            let mut image = VabImageNode::new(graphic.clone());
            image.fit = fit;
            app.world_mut()
                .spawn((
                    image,
                    Node {
                        width,
                        height,
                        flex_shrink: 0.0,
                        ..default()
                    },
                    ChildOf(root),
                ))
                .id()
        })
        .collect();
    for _ in 0..100 {
        app.update();
        std::thread::sleep(Duration::from_millis(5));
    }
    for (&entity, expected) in entities.iter().zip([
        Vec2::new(100., 40.),
        Vec2::new(200., 80.),
        Vec2::new(200., 80.),
        Vec2::splat(200.),
        Vec2::splat(200.),
    ]) {
        assert_eq!(
            app.world().get::<ComputedNode>(entity).unwrap().size,
            expected
        );
    }
    let image_handle = |app: &App, i| {
        app.world()
            .get::<ImageNode>(entities[i])
            .unwrap()
            .image
            .clone()
    };
    assert_eq!(image_handle(&app, 1), image_handle(&app, 2));
    assert_ne!(
        image_handle(&app, 3),
        image_handle(&app, 4),
        "fit is part of cache identity"
    );
    for index in [3, 4] {
        let image = image_handle(&app, index);
        let out = Arc::new(Mutex::new(None));
        let result = out.clone();
        let read = app
            .world_mut()
            .spawn(Readback::texture(image))
            .observe(move |e: On<ReadbackComplete>| {
                *result.lock().unwrap() = Some(e.data.clone());
            })
            .id();
        for _ in 0..40 {
            app.update();
            std::thread::sleep(Duration::from_millis(5));
        }
        let bytes = out.lock().unwrap().take().expect("GPU readback");
        let stride = RenderDevice::align_copy_bytes_per_row(200 * 4);
        let alpha = |x: usize, y: usize| bytes[y * stride + x * 4 + 3];
        assert!(alpha(100, 100) > 250);
        if index == 3 {
            assert_eq!(alpha(100, 20), 0, "contain has transparent upper margin");
            assert_eq!(alpha(100, 180), 0, "contain has transparent lower margin");
        } else {
            assert!(alpha(100, 20) > 250, "stretch fills the height");
        }
        app.world_mut().despawn(read);
    }
    app.world_mut().resource_mut::<UiScale>().0 = 1.5;
    for _ in 0..60 {
        app.update();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        app.world().get::<ComputedNode>(entities[0]).unwrap().size,
        Vec2::new(150., 60.)
    );
    assert_eq!(
        app.world().get::<ComputedNode>(entities[1]).unwrap().size,
        Vec2::new(300., 120.)
    );
}

#[test]
#[cfg(feature = "ui")]
#[ignore = "requires GPU"]
fn animated_ui_updates_only_changed_frames_and_shares_results() {
    use bevy::time::TimeUpdateStrategy;
    use bevy_flash::vab_ui::{VabImageNode, VabUiPlayback};
    let mut app = app(PathBuf::from("assets"));
    app.world_mut()
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
    app.world_mut()
        .resource_mut::<bevy_flash::vab_ui::VabUiCacheSettings>()
        .unused_frames = 1000;
    let screen = target(&mut app, UVec2::new(800, 600));
    app.world_mut().spawn((
        Camera2d,
        IsDefaultUiCamera,
        RenderTarget::Image(screen.into()),
    ));
    let graphic = app
        .world()
        .resource::<AssetServer>()
        .load("animated_ui.vab#sparkles");
    let mut spawn = || {
        app.world_mut()
            .spawn((
                VabImageNode::new(graphic.clone()),
                Node {
                    width: px(240),
                    ..default()
                },
            ))
            .id()
    };
    let a = spawn();
    let b = spawn();
    fn settle(app: &mut App) {
        for _ in 0..35 {
            app.update();
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn image(app: &App, e: Entity) -> Handle<Image> {
        app.world().get::<ImageNode>(e).unwrap().image.clone()
    }
    fn draws(app: &App) -> u64 {
        app.sub_app(RenderApp)
            .world()
            .resource::<FlashRenderDiagnostics>()
            .rendered_views
    }
    settle(&mut app);
    let g = app
        .world()
        .resource::<Assets<bevy_flash::vab_graphic::VabGraphic>>()
        .get(&graphic)
        .unwrap();
    assert_eq!(g.frame_count, 6);
    assert!(g.frame_rate > 0.0);
    let rate = g.frame_rate;
    assert_eq!(image(&app, a), image(&app, b));
    let original = image(&app, a);
    assert_ne!(
        original,
        bevy::image::TRANSPARENT_IMAGE_HANDLE,
        "UI never published its initial raster: layout={:?}, visibility={:?}, draws={}",
        app.world().get::<ComputedNode>(a),
        app.world().get::<InheritedVisibility>(a),
        draws(&app)
    );
    let baseline = draws(&app);
    let size = app.world().get::<ComputedNode>(a).unwrap().size;
    settle(&mut app);
    assert_eq!(draws(&app), baseline, "unchanged frame redraw");
    for frame in 1..=6 {
        app.world_mut()
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
                1.0 / rate as f64 + 1e-6,
            )));
        app.update();
        app.world_mut()
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
        settle(&mut app);
        assert_eq!(
            app.world().get::<VabUiPlayback>(a).unwrap().current_frame,
            frame % 6
        );
        assert_eq!(image(&app, a), image(&app, b));
        assert_eq!(
            app.world().get::<ComputedNode>(a).unwrap().size,
            size,
            "frame changed UI layout"
        );
        if frame < 6 {
            assert_ne!(image(&app, a), original);
        }
    }
    assert_eq!(
        image(&app, a),
        original,
        "loop should reuse first frame output"
    );
    assert_eq!(
        draws(&app),
        baseline + 5,
        "one render per unique frame shared by both nodes"
    );
    app.world_mut().get_mut::<VabUiPlayback>(a).unwrap().pause();
    app.world_mut().get_mut::<VabUiPlayback>(b).unwrap().pause();
    app.world_mut()
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            1.0 / rate as f64,
        )));
    settle(&mut app);
    assert_eq!(draws(&app), baseline + 5, "paused animation redraw");
    app.world_mut()
        .get_mut::<VabUiPlayback>(a)
        .unwrap()
        .resume();
    app.world_mut()
        .get_mut::<Visibility>(a)
        .unwrap()
        .set_if_neq(Visibility::Hidden);
    settle(&mut app);
    assert_eq!(
        app.world().get::<VabUiPlayback>(a).unwrap().current_frame,
        0,
        "hidden UI must stop advancing"
    );
}

#[test]
#[cfg(feature = "ui")]
#[ignore = "requires GPU"]
fn animated_ui_publishes_only_completed_rasters_during_continuous_playback() {
    use bevy::time::TimeUpdateStrategy;
    use bevy_flash::vab_ui::{VabImageNode, VabUiCacheSettings, VabUiSystems};
    #[derive(Resource, Default)]
    struct Published(std::collections::HashSet<bevy::asset::AssetId<Image>>);
    fn check(
        nodes: Query<&ImageNode, With<VabImageNode>>,
        jobs: Query<&OffscreenViewTarget>,
        mut published: ResMut<Published>,
    ) {
        for node in &nodes {
            assert!(
                !jobs.iter().any(|job| job.target.handle == node.image),
                "UI published an unfinished offscreen raster"
            );
            if node.image != bevy::image::TRANSPARENT_IMAGE_HANDLE {
                published.0.insert(node.image.id());
            } else {
                assert!(
                    published.0.is_empty(),
                    "loaded UI returned to an empty image"
                );
            }
        }
    }
    let mut app = app(PathBuf::from("assets"));
    app.init_resource::<Published>()
        .add_systems(PostUpdate, check.after(VabUiSystems::Rasterize));
    app.world_mut()
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            1.0 / 12.0,
        )));
    *app.world_mut().resource_mut::<VabUiCacheSettings>() = VabUiCacheSettings {
        idle_budget_bytes: 0,
        unused_frames: 0,
        ..default()
    };
    let screen = target(&mut app, UVec2::new(800, 600));
    app.world_mut().spawn((
        Camera2d,
        IsDefaultUiCamera,
        RenderTarget::Image(screen.into()),
    ));
    let graphic = app
        .world()
        .resource::<AssetServer>()
        .load("animated_ui.vab#sparkles");
    for _ in 0..2 {
        app.world_mut().spawn((
            VabImageNode::new(graphic.clone()),
            Node {
                width: px(240),
                ..default()
            },
        ));
    }
    for _ in 0..150 {
        app.update();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        app.world().resource::<Published>().0.len() > 6,
        "continuous playback must publish new completed images despite immediate idle eviction"
    );
}

#[test]
#[cfg(feature = "ui")]
#[ignore = "requires GPU"]
fn native_login_button_switches_states_without_layout_jumps_and_reuses_rasters() {
    use bevy_flash::{
        vab_button::VabButton,
        vab_ui::{VabButtonNode, VabImageNode, VabUiSystems},
    };
    #[derive(Resource)]
    struct Requested(Interaction);
    fn set_interaction(
        state: Res<Requested>,
        mut nodes: Query<&mut Interaction, With<VabButtonNode>>,
    ) {
        for mut interaction in &mut nodes {
            *interaction = state.0;
        }
    }
    let mut app = app(PathBuf::from("assets"));
    app.insert_resource(Requested(Interaction::None))
        .add_systems(
            PostUpdate,
            set_interaction.before(VabUiSystems::SelectButton),
        );
    app.world_mut()
        .resource_mut::<bevy_flash::vab_ui::VabUiCacheSettings>()
        .unused_frames = 1000;
    let screen = target(&mut app, UVec2::new(1000, 400));
    app.world_mut().spawn((
        Camera2d,
        IsDefaultUiCamera,
        RenderTarget::Image(screen.clone().into()),
    ));
    let button: Handle<VabButton> = app
        .world()
        .resource::<AssetServer>()
        .load("login.vab#login_button");
    let mut entities = vec![];
    for width in [180.0, 180.0, 360.0] {
        entities.push(
            app.world_mut()
                .spawn((
                    VabButtonNode::new(button.clone()),
                    Node {
                        width: px(width),
                        ..default()
                    },
                ))
                .id(),
        );
    }
    fn settle(app: &mut App) {
        for _ in 0..40 {
            app.update();
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn image(app: &App, entity: Entity) -> Handle<Image> {
        app.world().get::<ImageNode>(entity).unwrap().image.clone()
    }
    fn draws(app: &App) -> u64 {
        app.sub_app(RenderApp)
            .world()
            .resource::<FlashRenderDiagnostics>()
            .rendered_views
    }
    settle(&mut app);
    assert!(
        app.world()
            .resource::<AssetServer>()
            .is_loaded_with_dependencies(&button)
    );
    let original = image(&app, entities[0]);
    assert_ne!(original, bevy::image::TRANSPARENT_IMAGE_HANDLE);
    let layout = app.world().get::<ComputedNode>(entities[0]).unwrap().size;
    assert!(layout.y > 0.0);
    let baseline = draws(&app);
    for (index, interaction) in [
        Interaction::Hovered,
        Interaction::Pressed,
        Interaction::None,
    ]
    .into_iter()
    .enumerate()
    {
        app.world_mut().resource_mut::<Requested>().0 = interaction;
        settle(&mut app);
        let asset = app
            .world()
            .resource::<Assets<VabButton>>()
            .get(&button)
            .unwrap();
        let expected = match interaction {
            Interaction::None => &asset.up,
            Interaction::Hovered => &asset.over,
            Interaction::Pressed => &asset.down,
        };
        assert_eq!(
            &app.world()
                .get::<VabImageNode>(entities[0])
                .unwrap()
                .graphic,
            expected
        );
        assert_eq!(
            app.world().get::<ComputedNode>(entities[0]).unwrap().size,
            layout
        );
        assert_eq!(image(&app, entities[0]), image(&app, entities[1]));
        assert_ne!(image(&app, entities[0]), image(&app, entities[2]));
        assert_ne!(
            image(&app, entities[0]),
            bevy::image::TRANSPARENT_IMAGE_HANDLE
        );
        assert_eq!(
            draws(&app),
            baseline + (index + 1).min(2) as u64 * 2,
            "one raster per state and size; returning to up must reuse its image"
        );
    }
    assert_eq!(image(&app, entities[0]), original);
}

#[test]
#[cfg(feature = "ui")]
#[ignore = "requires GPU"]
fn native_login_button_states_render_different_pixels() {
    use bevy_flash::{vab_button::VabButton, vab_graphic::VabGraphic};
    let mut app = app(PathBuf::from("assets"));
    let button: Handle<VabButton> = app
        .world()
        .resource::<AssetServer>()
        .load("login.vab#login_button");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        app.update();
        let server = app.world().resource::<AssetServer>();
        if let Some(bevy::asset::LoadState::Failed(e)) = server.get_load_state(&button) {
            panic!("{e}");
        }
        if server.is_loaded_with_dependencies(&button) {
            break;
        }
        assert!(Instant::now() < deadline);
    }
    let buttons = app.world().resource::<Assets<VabButton>>();
    let b = buttons.get(&button).unwrap();
    let states = [b.up.clone(), b.over.clone(), b.down.clone()];
    let graphics = app.world().resource::<Assets<VabGraphic>>();
    let bounds = graphics.get(&states[0]).unwrap().visual_bounds;
    let renders: Vec<_> = states
        .iter()
        .map(|state| graphics.get(state).unwrap().render_asset.clone())
        .collect();
    let scale = 400.0 / bounds[2];
    let size = (Vec2::new(bounds[2], bounds[3]) * scale).ceil().as_uvec2();
    let image = target(&mut app, size);
    let mut output = OffscreenViewTarget::new(image.clone(), size);
    output.scale = Vec2::splat(scale);
    output.origin = Vec2::new(bounds[0], bounds[1]) * scale;
    let mut player = VabPlayer::default();
    player.pause();
    let entity = app
        .world_mut()
        .spawn((
            VabAssetHandle(renders[0].clone()),
            player,
            output,
            Msaa::Sample4,
        ))
        .id();
    let mut captures = vec![];
    for (state, render) in ["up", "over", "down"].into_iter().zip(renders) {
        app.world_mut().get_mut::<VabAssetHandle>(entity).unwrap().0 = render;
        let bytes = capture(&mut app, image.clone(), size);
        assert!(
            bytes.chunks_exact(4).any(|pixel| pixel[3] > 200),
            "{state} is blank"
        );
        image::RgbaImage::from_raw(size.x, size.y, bytes.clone())
            .unwrap()
            .save(std::env::temp_dir().join(format!("vab_login_{state}.png")))
            .unwrap();
        captures.push(bytes);
    }
    assert_ne!(captures[0], captures[1], "hover transform was lost");
    assert_ne!(captures[1], captures[2], "pressed shape/color was lost");
}

#[test]
#[cfg(feature = "ui")]
#[ignore = "requires GPU"]
fn background551284_export_animates_and_renders() {
    use bevy_flash::vab_graphic::VabGraphic;
    let dir = std::env::temp_dir().join("vab_background551284");
    std::fs::create_dir_all(&dir).unwrap();
    vatf::convert_swf_animated_ui_to_vab(
        std::path::Path::new("assets/background551284.swf"),
        &dir.join("background.vab"),
    )
    .unwrap();
    let mut app = app(dir);
    let graphic: Handle<VabGraphic> = app
        .world()
        .resource::<AssetServer>()
        .load("background.vab#sparkles");
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        app.update();
        let server = app.world().resource::<AssetServer>();
        if let Some(bevy::asset::LoadState::Failed(e)) = server.get_load_state(&graphic) {
            panic!("{e}");
        }
        if server.is_loaded_with_dependencies(&graphic) {
            break;
        }
        assert!(Instant::now() < deadline);
    }
    let g = app
        .world()
        .resource::<Assets<VabGraphic>>()
        .get(&graphic)
        .unwrap();
    println!(
        "sparkles: frames={} fps={} visual_bounds={:?}",
        g.frame_count, g.frame_rate, g.visual_bounds
    );
    assert!(g.frame_count > 1);
    let count = g.frame_count;
    let bounds = g.visual_bounds;
    let handle = g.render_asset.clone();
    let scale = 512.0 / bounds[2].max(bounds[3]);
    let size = (Vec2::new(bounds[2], bounds[3]) * scale).ceil().as_uvec2();
    let image = target(&mut app, size);
    let mut output = OffscreenViewTarget::new(image.clone(), size);
    output.scale = Vec2::splat(scale);
    output.origin = Vec2::new(bounds[0], bounds[1]) * scale;
    let mut player = VabPlayer::default();
    player.pause();
    let entity = app
        .world_mut()
        .spawn((VabAssetHandle(handle), player, output, Msaa::Sample4))
        .id();
    let mut captures = Vec::new();
    for frame in [0, count / 3, count * 2 / 3] {
        app.world_mut()
            .get_mut::<VabPlayer>(entity)
            .unwrap()
            .current_frame = frame;
        let pixels = capture(&mut app, image.clone(), size);
        assert!(
            pixels.chunks_exact(4).any(|p| p[3] > 0),
            "empty frame {frame}"
        );
        let path = std::env::temp_dir().join(format!("background551284_{frame}.png"));
        image::save_buffer(&path, &pixels, size.x, size.y, image::ColorType::Rgba8).unwrap();
        println!("capture {}", path.display());
        captures.push(pixels);
    }
    assert!(
        captures.windows(2).any(|p| p[0] != p[1]),
        "child animations did not change the rendered image"
    );
}
