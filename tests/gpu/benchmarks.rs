//! GPU coverage for benchmarks.
use super::*;

#[test]
#[ignore = "requires GPU and source SWF"]
fn spirit_frames_to_png() {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/render-validation");
    std::fs::create_dir_all(&directory).unwrap();
    let custom_swf = std::env::var_os("VAB_TEST_SWF");
    let swf = custom_swf.clone().map_or_else(
        || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../bevy_flash/assets/spirit2159src.swf"),
        PathBuf::from,
    );
    let output_name = format!(
        "{}.vab",
        swf.file_stem().unwrap_or_default().to_string_lossy()
    );
    let conversion_started = Instant::now();
    vatf::convert_swf_to_vab(&swf, &directory.join(&output_name)).unwrap();
    eprintln!(
        "source={}, swf_bytes={}, vab_bytes={}, conversion={:.1}ms",
        swf.display(),
        std::fs::metadata(&swf).unwrap().len(),
        std::fs::metadata(directory.join(&output_name))
            .unwrap()
            .len(),
        conversion_started.elapsed().as_secs_f64() * 1_000.0,
    );
    let mut app = app(directory.clone());
    if let Some(mebibytes) = std::env::var_os("VAB_POOL_BUDGET_MIB") {
        let mebibytes = mebibytes
            .to_string_lossy()
            .parse::<u64>()
            .expect("VAB_POOL_BUDGET_MIB must be an integer");
        app.world_mut()
            .resource_mut::<TransientTexturePoolSettings>()
            .max_resident_bytes = mebibytes * 1024 * 1024;
    }
    let handle: Handle<VabAsset> = app
        .world()
        .resource::<AssetServer>()
        .load(output_name.clone());
    let start = Instant::now();
    while !app.world().resource::<Assets<VabAsset>>().contains(&handle) {
        app.update();
        assert!(start.elapsed() < Duration::from_secs(30));
        std::thread::sleep(Duration::from_millis(5));
    }
    let asset = app
        .world()
        .resource::<Assets<VabAsset>>()
        .get(&handle)
        .unwrap();
    eprintln!(
        "clips={}, shapes={}, materials={:?}",
        asset.baked.clips.len(),
        asset.render_meshes.len(),
        asset.render_meshes.iter().fold([0usize; 3], |mut c, m| {
            c[match m.material {
                MeshMaterial::Color => 0,
                MeshMaterial::Bitmap(_) => 1,
                MeshMaterial::Gradient(_) => 2,
            }] += 1;
            c
        })
    );
    fn bounds(
        nodes: &[bevy_flash::vab_asset::VabCommand],
        asset: &VabAsset,
        b: &mut [f32; 4],
    ) {
        use bevy_flash::vab_asset::VabCommand;
        for n in nodes {
            match n {
                VabCommand::RenderShape { handle, transform } => {
                    let r = asset.render_meshes[*handle].local_bounds;
                    let m = transform.matrix;
                    for p in [[r[0], r[1]], [r[2], r[1]], [r[0], r[3]], [r[2], r[3]]] {
                        let x = m.a * p[0] + m.c * p[1] + m.tx;
                        let y = m.b * p[0] + m.d * p[1] + m.ty;
                        b[0] = b[0].min(x);
                        b[1] = b[1].min(y);
                        b[2] = b[2].max(x);
                        b[3] = b[3].max(y);
                    }
                }
                VabCommand::ApplyFilter { commands, .. } | VabCommand::Blend(commands, _) => {
                    bounds(&commands.commands, asset, b)
                }
                _ => {}
            }
        }
    }
    let mut b = [
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    ];
    let count = asset.baked.clips[0].frames.len();
    for frame in [0, count / 3, count * 2 / 3] {
        bounds(
            &asset
                .sample(0, frame, &default(), Vec3::ONE)
                .unwrap()
                .commands,
            asset,
            &mut b,
        );
    }
    eprintln!("bounds={b:?}, frames={count}");
    let size = UVec2::new(
        (b[2] - b[0]).ceil() as u32 + 64,
        (b[3] - b[1]).ceil() as u32 + 64,
    );
    let render_scale = std::env::var("VAB_TEST_SCALE").map_or(1.0, |value| {
        value.parse::<f32>().expect("invalid VAB_TEST_SCALE")
    });
    assert!(render_scale.is_finite() && render_scale > 0.0);
    eprintln!("render_scale={render_scale}");
    let output = target(&mut app, size);
    app.world_mut().spawn((
        Camera2d,
        Camera {
            clear_color: ClearColorConfig::Custom(Color::NONE),
            ..default()
        },
        RenderTarget::Image(output.clone().into()),
        Transform::from_xyz(
            (b[0] + b[2]) * 0.5 * render_scale,
            -(b[1] + b[3]) * 0.5 * render_scale,
            0.0,
        ),
        bevy::render::view::Msaa::Off,
    ));
    let mut player = VabPlayer::default();
    player.pause();
    let entity = app
        .world_mut()
        .spawn((
            VabAssetHandle(handle.clone()),
            player,
            Transform::from_scale(Vec3::splat(render_scale)),
        ))
        .id();
    for frame in [0, count / 3, count * 2 / 3] {
        app.world_mut()
            .get_mut::<VabPlayer>(entity)
            .unwrap()
            .current_frame = frame;
        let pixels = capture_camera(&mut app, output.clone(), size);
        assert!(
            pixels.chunks_exact(4).filter(|p| p[3] > 0).count() > 100,
            "empty rendered frame"
        );
        image::save_buffer(
            directory.join(format!(
                "{}_{frame:04}.png",
                swf.file_stem().unwrap_or_default().to_string_lossy()
            )),
            &pixels,
            size.x,
            size.y,
            image::ColorType::Rgba8,
        )
        .unwrap();
        let paused = measure_batch_baseline(&mut app, 120);
        print_batch_baseline(&format!("paused source frame {frame}"), 120, &paused);
    }
    let animated = measure_animated_baseline(&mut app, entity, count, 240);
    print_batch_baseline(
        "playing at one source frame per two renders",
        240,
        &animated,
    );
    let diagnostics = app
        .sub_app(RenderApp)
        .world()
        .resource::<FlashRenderDiagnostics>();
    eprintln!("VAB instance diagnostics: {diagnostics:?}");
    if custom_swf.is_none() {
        assert!(diagnostics.material_bind_group_creations > 0);
        assert!(
            diagnostics.material_bind_group_reuses > diagnostics.material_bind_group_creations,
            "material bind groups should be reused across frames"
        );
        assert_eq!(diagnostics.instance_buffer_allocations, 1);
        assert!(
            diagnostics.instance_buffer_reuses > 0,
            "instance storage buffer was not reused: {diagnostics:?}"
        );
        assert_eq!(diagnostics.instance_bind_group_creations, 1);
        assert!(
            diagnostics.instance_bind_group_reuses > 0,
            "instance storage bind group was not reused: {diagnostics:?}"
        );
        assert!(
            diagnostics.filter_uniform_buffer_reuses
                > diagnostics.filter_uniform_buffer_allocations,
            "filter uniform slots did not reuse capacity across spirit frames: {diagnostics:?}"
        );
    }
    assert_eq!(
        app.world()
            .resource::<Assets<Image>>()
            .get(&output)
            .unwrap()
            .texture_descriptor
            .format,
        TextureFormat::Rgba8UnormSrgb
    );
}

#[derive(Default)]
struct BatchBaseline {
    wall_ns: u128,
    extract_ns: u128,
    queue_ns: u128,
    prepare_buffers_ns: u128,
    prepare_bind_groups_ns: u128,
    draw_calls: u64,
    packets: u128,
    filter_layers: u128,
    filter_cache_misses: u128,
    filter_output_pixels: u128,
    texture_allocations: u128,
    texture_reuses: u128,
}

fn measure_batch_baseline(app: &mut App, frames: u64) -> BatchBaseline {
    use std::sync::atomic::Ordering;

    let calls_before = app
        .sub_app(RenderApp)
        .world()
        .resource::<FlashRenderDiagnostics>()
        .vab_draw_calls
        .load(Ordering::Relaxed);
    let mut result = BatchBaseline::default();
    for _ in 0..frames {
        let started = Instant::now();
        app.update();
        result.wall_ns += started.elapsed().as_nanos();
        let diagnostics = app
            .sub_app(RenderApp)
            .world()
            .resource::<FlashRenderDiagnostics>();
        assert!(diagnostics.errors.is_empty(), "{diagnostics:?}");
        result.extract_ns += u128::from(diagnostics.vab_extract_cpu_ns);
        result.queue_ns += u128::from(diagnostics.vab_queue_cpu_ns);
        result.prepare_buffers_ns += u128::from(diagnostics.vab_prepare_buffers_cpu_ns);
        result.prepare_bind_groups_ns += u128::from(diagnostics.vab_prepare_bind_groups_cpu_ns);
        result.packets += diagnostics.vab_draw_packets as u128;
        result.filter_layers += diagnostics.vab_filter_layers as u128;
        result.filter_cache_misses += diagnostics.vab_filter_cache_misses as u128;
        result.filter_output_pixels += u128::from(diagnostics.vab_filter_output_pixels);
        result.texture_allocations += u128::from(diagnostics.texture_allocations);
        result.texture_reuses += u128::from(diagnostics.texture_reuses);
    }
    let calls_after = app
        .sub_app(RenderApp)
        .world()
        .resource::<FlashRenderDiagnostics>()
        .vab_draw_calls
        .load(Ordering::Relaxed);
    result.draw_calls = calls_after - calls_before;
    result
}

fn print_batch_baseline(name: &str, frames: u64, result: &BatchBaseline) {
    let average_us = |nanoseconds: u128| nanoseconds as f64 / frames as f64 / 1_000.0;
    eprintln!(
        "{name}: frames={frames}, draw_calls/frame={:.1}, packets/frame={:.1}, \
         filters/frame={:.1}, filter_misses/frame={:.1}, filter_pixels/frame={:.0}, \
         texture_allocations/frame={:.1}, texture_reuses/frame={:.1}, \
         app_update={:.1}us, \
         extract={:.1}us, queue={:.1}us, prepare_buffers={:.1}us, \
         prepare_bind_groups={:.1}us",
        result.draw_calls as f64 / frames as f64,
        result.packets as f64 / frames as f64,
        result.filter_layers as f64 / frames as f64,
        result.filter_cache_misses as f64 / frames as f64,
        result.filter_output_pixels as f64 / frames as f64,
        result.texture_allocations as f64 / frames as f64,
        result.texture_reuses as f64 / frames as f64,
        average_us(result.wall_ns),
        average_us(result.extract_ns),
        average_us(result.queue_ns),
        average_us(result.prepare_buffers_ns),
        average_us(result.prepare_bind_groups_ns),
    );
}

fn measure_animated_baseline(
    app: &mut App,
    entity: Entity,
    source_frames: usize,
    render_frames: u64,
) -> BatchBaseline {
    use std::sync::atomic::Ordering;

    let calls_before = app
        .sub_app(RenderApp)
        .world()
        .resource::<FlashRenderDiagnostics>()
        .vab_draw_calls
        .load(Ordering::Relaxed);
    let mut result = BatchBaseline::default();
    for render_frame in 0..render_frames {
        app.world_mut()
            .get_mut::<VabPlayer>(entity)
            .unwrap()
            .current_frame = (render_frame as usize / 2) % source_frames;
        let started = Instant::now();
        app.update();
        result.wall_ns += started.elapsed().as_nanos();
        let diagnostics = app
            .sub_app(RenderApp)
            .world()
            .resource::<FlashRenderDiagnostics>();
        assert!(diagnostics.errors.is_empty(), "{diagnostics:?}");
        result.extract_ns += u128::from(diagnostics.vab_extract_cpu_ns);
        result.queue_ns += u128::from(diagnostics.vab_queue_cpu_ns);
        result.prepare_buffers_ns += u128::from(diagnostics.vab_prepare_buffers_cpu_ns);
        result.prepare_bind_groups_ns += u128::from(diagnostics.vab_prepare_bind_groups_cpu_ns);
        result.packets += diagnostics.vab_draw_packets as u128;
        result.filter_layers += diagnostics.vab_filter_layers as u128;
        result.filter_cache_misses += diagnostics.vab_filter_cache_misses as u128;
        result.filter_output_pixels += u128::from(diagnostics.vab_filter_output_pixels);
        result.texture_allocations += u128::from(diagnostics.texture_allocations);
        result.texture_reuses += u128::from(diagnostics.texture_reuses);
    }
    result.draw_calls = app
        .sub_app(RenderApp)
        .world()
        .resource::<FlashRenderDiagnostics>()
        .vab_draw_calls
        .load(Ordering::Relaxed)
        - calls_before;
    result
}

fn warm_batch_baseline(
    app: &mut App,
    expected_instances: usize,
    expected_packets: usize,
    expected_calls: u64,
) {
    use std::sync::atomic::Ordering;

    let started = Instant::now();
    let mut stable_frames = 0;
    loop {
        let calls_before = app
            .sub_app(RenderApp)
            .world()
            .resource::<FlashRenderDiagnostics>()
            .vab_draw_calls
            .load(Ordering::Relaxed);
        app.update();
        let diagnostics = app
            .sub_app(RenderApp)
            .world()
            .resource::<FlashRenderDiagnostics>();
        let calls = diagnostics.vab_draw_calls.load(Ordering::Relaxed) - calls_before;
        if diagnostics.vab_instances_queued == expected_instances
            && diagnostics.vab_draw_packets == expected_packets
            && calls == expected_calls
        {
            stable_frames += 1;
            if stable_frames == 3 {
                return;
            }
        } else {
            stable_frames = 0;
        }
        assert!(
            started.elapsed() < Duration::from_secs(45),
            "batch baseline warm-up timeout: calls={calls}, diagnostics={diagnostics:?}"
        );
    }
}

/// Structural and CPU baseline for optimization-list item 7. Run serially:
/// `cargo test --test render_gpu cross_animation_batching_baseline -- --ignored --nocapture`
#[test]
#[ignore = "requires GPU; performance baseline"]
fn cross_animation_batching_baseline() {
    let mut app = app(PathBuf::from("assets"));
    let size = UVec2::splat(256);
    let output = target(&mut app, size);
    app.world_mut().spawn((
        Camera2d,
        Camera {
            clear_color: ClearColorConfig::Custom(Color::NONE),
            ..default()
        },
        RenderTarget::Image(output.into()),
        bevy::render::view::Msaa::Off,
    ));

    let make_mesh = |color: [f32; 4], offset: f32| {
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![
                [offset - 4.0, -4.0, 0.0],
                [offset + 4.0, -4.0, 0.0],
                [offset, 4.0, 0.0],
            ],
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, vec![color; 3])
    };
    let mesh_a = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(make_mesh([1.0, 0.2, 0.1, 1.0], -2.0));
    let mesh_b = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(make_mesh([0.1, 0.4, 1.0, 1.0], 3.0));
    let shape = |id| BakedNode::Shape {
        id,
        ratio: 0,
        transform: AnimTransform::default(),
    };
    let asset = app
        .world_mut()
        .resource_mut::<Assets<VabAsset>>()
        .add(VabAsset {
            shape_map: [(1, vec![0]), (2, vec![1])].into(),
            morph_map: default(),
            render_meshes: vec![
                RenderMeshGroup {
                    mesh: mesh_a,
                    material: MeshMaterial::Color,
                    local_bounds: [-6.0, -4.0, 2.0, 4.0],
                },
                RenderMeshGroup {
                    mesh: mesh_b,
                    material: MeshMaterial::Color,
                    local_bounds: [-1.0, -4.0, 7.0, 4.0],
                },
            ],
            baked: BakedMovie {
                frame_rate: 30.0,
                skins: vec![],
                clips: vec![BakedClip {
                    name: "default".into(),
                    start_frame: 0,
                    events: vec![],
                    frames: vec![vec![shape(1)], vec![shape(1), shape(2)]],
                }],
            },
        });

    let spawn_instance = |app: &mut App, index: usize| {
        let mut player = VabPlayer::default();
        player.pause();
        let x = (index % 10) as f32 * 20.0 - 90.0;
        let y = (index / 10) as f32 * 20.0 - 90.0;
        app.world_mut()
            .spawn((
                VabAssetHandle(asset.clone()),
                player,
                Transform::from_xyz(x, y, 0.0),
            ))
            .id()
    };
    let mut entities = Vec::with_capacity(100);
    entities.push(spawn_instance(&mut app, 0));
    warm_batch_baseline(&mut app, 1, 1, 1);

    const FRAMES: u64 = 120;
    let one_instance = measure_batch_baseline(&mut app, FRAMES);
    print_batch_baseline("one unfiltered VAB instance", FRAMES, &one_instance);
    assert_eq!(one_instance.draw_calls, FRAMES);

    for index in 1..100 {
        entities.push(spawn_instance(&mut app, index));
    }
    warm_batch_baseline(&mut app, 100, 100, 100);

    let same_frame = measure_batch_baseline(&mut app, FRAMES);
    print_batch_baseline(
        "100 identical VAB instances, same frame",
        FRAMES,
        &same_frame,
    );
    assert_eq!(same_frame.draw_calls, 100 * FRAMES);
    {
        let diagnostics = app
            .sub_app(RenderApp)
            .world()
            .resource::<FlashRenderDiagnostics>();
        assert_eq!(diagnostics.vab_instances_queued, 100);
        assert_eq!(diagnostics.vab_draw_packets, 100);
        assert_eq!(diagnostics.vab_sample_cache_hits, 100);
        assert_eq!(diagnostics.vab_sample_cache_misses, 0);
    }

    for (index, entity) in entities.into_iter().enumerate() {
        app.world_mut()
            .get_mut::<VabPlayer>(entity)
            .unwrap()
            .current_frame = index % 2;
    }
    warm_batch_baseline(&mut app, 100, 150, 150);
    let varied_frames = measure_batch_baseline(&mut app, FRAMES);
    print_batch_baseline(
        "100 identical VAB instances, alternating frames",
        FRAMES,
        &varied_frames,
    );
    assert_eq!(varied_frames.draw_calls, 150 * FRAMES);
    let diagnostics = app
        .sub_app(RenderApp)
        .world()
        .resource::<FlashRenderDiagnostics>();
    assert_eq!(diagnostics.vab_instances_queued, 100);
    assert_eq!(diagnostics.vab_draw_packets, 150);
    assert_eq!(diagnostics.vab_sample_cache_hits, 100);
    assert_eq!(diagnostics.vab_sample_cache_misses, 0);
}
