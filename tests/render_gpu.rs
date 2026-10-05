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
use bevy_flash_remake::{
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

fn app(path: PathBuf) -> App {
    app_with_render_diagnostics(path, false)
}

fn app_with_render_diagnostics(path: PathBuf, render_diagnostics: bool) -> App {
    app_with_diagnostic_options(path, render_diagnostics, true)
}

fn app_with_diagnostic_options(
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
    while app.plugins_state() != bevy::app::PluginsState::Ready {
        app.update();
    }
    app.finish();
    app.cleanup();
    app
}

fn capture(app: &mut App, image: Handle<Image>, size: UVec2) -> Vec<u8> {
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

fn target(app: &mut App, size: UVec2) -> Handle<Image> {
    app.world_mut()
        .resource_mut::<Assets<Image>>()
        .add(OffscreenViewTarget::create_image(size))
}

fn capture_camera(app: &mut App, image: Handle<Image>, size: UVec2) -> Vec<u8> {
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

#[test]
#[ignore = "requires GPU"]
fn vab_instance_queues_and_sorts_with_bevy_2d() {
    let mut app = app(PathBuf::from("assets"));
    let size = UVec2::splat(64);
    let output = target(&mut app, size);
    let camera = app
        .world_mut()
        .spawn((
            Camera2d,
            Camera {
                clear_color: ClearColorConfig::Custom(Color::NONE),
                ..default()
            },
            RenderTarget::Image(output.clone().into()),
        ))
        .id();
    assert_eq!(
        app.world().get::<bevy::render::view::Msaa>(camera),
        Some(&bevy::render::view::Msaa::Sample4)
    );
    let mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![[-20.0, -20.0, 0.0], [20.0, -20.0, 0.0], [-20.0, 20.0, 0.0]],
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0, 0.0, 0.0, 0.5]; 3]);
    let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(mesh);
    let red_frame = vec![
        BakedNode::Shape {
            id: 1,
            ratio: 0,
            transform: AnimTransform::default(),
        },
        BakedNode::Shape {
            id: 1,
            ratio: 0,
            transform: AnimTransform::default(),
        },
    ];
    let asset = VabAsset {
        shape_map: [(1, vec![0])].into(),
        morph_map: default(),
        render_meshes: vec![RenderMeshGroup {
            mesh,
            material: MeshMaterial::Color,
            local_bounds: [-20.0, -20.0, 20.0, 20.0],
        }],
        baked: BakedMovie {
            frame_rate: 30.0,
            skins: vec![],
            clips: vec![BakedClip {
                name: "default".into(),
                start_frame: 0,
                events: vec![],
                frames: vec![red_frame.clone(), red_frame],
            }],
        },
    };
    let handle = app
        .world_mut()
        .resource_mut::<Assets<VabAsset>>()
        .add(asset);
    let mut red_player = VabPlayer::default();
    red_player.pause();
    let red_entity = app
        .world_mut()
        .spawn((VabAssetHandle(handle.clone()), red_player))
        .id();
    app.world_mut().spawn((
        VabAssetHandle(handle),
        VabPlayer::default(),
        Visibility::Hidden,
    ));
    let green_mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![[-10.0, -10.0, 0.0], [10.0, -10.0, 0.0], [-10.0, 10.0, 0.0]],
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.0, 1.0, 0.0, 1.0]; 3]);
    let green_mesh = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(green_mesh);
    let green_asset = VabAsset {
        shape_map: [(1, vec![0])].into(),
        morph_map: default(),
        render_meshes: vec![RenderMeshGroup {
            mesh: green_mesh,
            material: MeshMaterial::Color,
            local_bounds: [-20.0, -20.0, 20.0, 20.0],
        }],
        baked: BakedMovie {
            frame_rate: 30.0,
            skins: vec![],
            clips: vec![BakedClip {
                name: "default".into(),
                start_frame: 0,
                events: vec![],
                frames: vec![vec![BakedNode::Shape {
                    id: 1,
                    ratio: 0,
                    transform: AnimTransform::default(),
                }]],
            }],
        },
    };
    let green_handle = app
        .world_mut()
        .resource_mut::<Assets<VabAsset>>()
        .add(green_asset);
    app.world_mut().spawn((
        VabAssetHandle(green_handle),
        VabPlayer::default(),
        Transform::from_xyz(0.0, 0.0, 1.0),
    ));
    let pixels = capture_camera(&mut app, output, size);
    assert!(
        pixels.chunks_exact(4).any(|p| p[0] > 150 && p[3] > 100),
        "VAB instance render produced no red pixels"
    );
    assert!(
        pixels.chunks_exact(4).any(|p| p[1] > 240 && p[0] < 10),
        "VAB at z=1 should sort over VAB at z=0"
    );
    {
        let diagnostics = app
            .sub_app(RenderApp)
            .world()
            .resource::<FlashRenderDiagnostics>();
        assert_eq!(diagnostics.vab_draw_instances_prepared, 3);
        assert_eq!(diagnostics.vab_draw_packets, 2);
        assert_eq!(diagnostics.vab_instances_extracted, 2);
        assert_eq!(diagnostics.vab_instances_prepared, 2);
        assert_eq!(diagnostics.vab_instances_queued, 2);
        assert_eq!(diagnostics.vab_sample_cache_hits, 2);
        assert_eq!(diagnostics.vab_sample_cache_misses, 0);
    }
    app.world_mut()
        .get_mut::<VabPlayer>(red_entity)
        .unwrap()
        .current_frame = 1;
    app.update();
    let diagnostics = app
        .sub_app(RenderApp)
        .world()
        .resource::<FlashRenderDiagnostics>();
    assert_eq!(diagnostics.vab_sample_cache_hits, 1);
    assert_eq!(diagnostics.vab_sample_cache_misses, 1);
}

#[test]
#[ignore = "requires GPU"]
fn bitmap_uv_translation_and_gpu_image_replacement() {
    let mut app = app(PathBuf::from("assets"));
    let size = UVec2::splat(32);
    let output = target(&mut app, size);
    app.world_mut().spawn((
        Camera2d,
        CompositingSpace::Srgb,
        Camera {
            clear_color: ClearColorConfig::Custom(Color::NONE),
            ..default()
        },
        RenderTarget::Image(output.clone().into()),
        bevy::render::view::Msaa::Sample4,
    ));

    let mut texture = Image::new(
        Extent3d {
            width: 2,
            height: 1,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        vec![255, 0, 0, 255, 0, 255, 0, 255],
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    texture.sampler = bevy::image::ImageSampler::nearest();
    let texture = app.world_mut().resource_mut::<Assets<Image>>().add(texture);
    let uv = Mat3::from_cols(Vec3::ZERO, Vec3::ZERO, Vec3::new(0.75, 0.5, 1.0));
    let material = app
        .world_mut()
        .resource_mut::<Assets<BitmapMaterial>>()
        .add(BitmapMaterial {
            texture: texture.clone(),
            texture_transform: Mat4::from_mat3(uv),
        });
    let mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![[-8.0, -8.0, 0.0], [8.0, -8.0, 0.0], [-8.0, 8.0, 0.0]],
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0; 4]; 3]);
    let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(mesh);
    let asset = VabAsset {
        shape_map: [(1, vec![0])].into(),
        morph_map: default(),
        render_meshes: vec![RenderMeshGroup {
            mesh,
            material: MeshMaterial::Bitmap(material),
            local_bounds: [-8.0, -8.0, 8.0, 8.0],
        }],
        baked: BakedMovie {
            frame_rate: 30.0,
            skins: vec![],
            clips: vec![BakedClip {
                name: "default".into(),
                start_frame: 0,
                events: vec![],
                frames: vec![vec![BakedNode::Shape {
                    id: 1,
                    ratio: 0,
                    transform: AnimTransform::default(),
                }]],
            }],
        },
    };
    let handle = app
        .world_mut()
        .resource_mut::<Assets<VabAsset>>()
        .add(asset);
    app.world_mut()
        .spawn((VabAssetHandle(handle), VabPlayer::default()));

    let green = capture_camera(&mut app, output.clone(), size);
    assert!(green.chunks_exact(4).any(|p| p[1] > 240 && p[0] < 10));
    let creations = app
        .sub_app(RenderApp)
        .world()
        .resource::<FlashRenderDiagnostics>()
        .material_bind_group_creations;

    {
        let mut images = app.world_mut().resource_mut::<Assets<Image>>();
        let mut image = images.get_mut(&texture).unwrap();
        image.resize(Extent3d {
            width: 4,
            height: 1,
            depth_or_array_layers: 1,
        });
        image.data = Some(vec![
            255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 0, 0, 255, 255,
        ]);
    }
    let blue = capture_camera(&mut app, output, size);
    assert!(blue.chunks_exact(4).any(|p| p[2] > 240 && p[1] < 10));
    assert!(
        app.sub_app(RenderApp)
            .world()
            .resource::<FlashRenderDiagnostics>()
            .material_bind_group_creations
            > creations
    );
}

#[test]
#[ignore = "requires GPU"]
fn solid_color_alpha_and_output_image() {
    let mut app = app(PathBuf::from("assets"));
    app.world_mut()
        .resource_mut::<TransientTexturePoolSettings>()
        .max_resident_bytes = 0;
    let size = UVec2::splat(64);
    let output = target(&mut app, size);
    let mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![[8.0, 8.0, 0.0], [56.0, 8.0, 0.0], [8.0, 56.0, 0.0]],
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0, 0.0, 0.0, 0.5]; 3]);
    let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(mesh);
    let asset = VabAsset {
        shape_map: [(1, vec![0])].into(),
        morph_map: default(),
        render_meshes: vec![RenderMeshGroup {
            mesh,
            material: MeshMaterial::Color,
            local_bounds: [8.0, 8.0, 56.0, 56.0],
        }],
        baked: BakedMovie {
            frame_rate: 30.0,
            skins: vec![],
            clips: vec![BakedClip {
                name: "default".into(),
                start_frame: 0,
                events: vec![],
                frames: vec![vec![BakedNode::Shape {
                    id: 1,
                    ratio: 0,
                    transform: AnimTransform::default(),
                }]],
            }],
        },
    };
    let handle = app
        .world_mut()
        .resource_mut::<Assets<VabAsset>>()
        .add(asset);
    app.world_mut().spawn((
        VabAssetHandle(handle),
        VabPlayer::default(),
        OffscreenViewTarget::new(output.clone(), size),
        bevy::render::view::Msaa::Off,
    ));
    let pixels = capture(&mut app, output.clone(), size);
    let inside = &pixels[(16 * 64 + 16) * 4..(16 * 64 + 16) * 4 + 4];
    assert!(
        inside[0] > 250 && inside[1] < 3 && inside[2] < 3 && (125..=130).contains(&inside[3]),
        "{inside:?}"
    );
    assert_eq!(&pixels[..4], &[0, 0, 0, 0]);
    let _ = capture(&mut app, output, size);
    let diagnostics = app
        .sub_app(RenderApp)
        .world()
        .resource::<FlashRenderDiagnostics>();
    assert_eq!(diagnostics.filter_uniform_buffer_allocations, 1);
    assert!(
        diagnostics.filter_uniform_buffer_reuses > 0,
        "offscreen uniform buffer was not reused: {diagnostics:?}"
    );
    assert_eq!(diagnostics.texture_allocations, 1);
    assert_eq!(
        diagnostics.texture_first_in_bucket_allocations
            + diagnostics.texture_exhausted_bucket_allocations,
        diagnostics.texture_allocations
    );
    assert_eq!(diagnostics.pooled_texture_bytes, 0);
    assert_eq!(diagnostics.live_transient_bytes, 0);
}

#[test]
#[ignore = "requires GPU"]
fn blur_filter_expands_pixels_outside_shape() {
    let mut app = app(PathBuf::from("assets"));
    let size = UVec2::splat(64);
    let output = target(&mut app, size);
    let mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![
            [24.0, 24.0, 0.0],
            [40.0, 24.0, 0.0],
            [24.0, 40.0, 0.0],
            [24.0, 40.0, 0.0],
            [40.0, 24.0, 0.0],
            [40.0, 40.0, 0.0],
        ],
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0, 0.0, 0.0, 1.0]; 6]);
    let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(mesh);
    let asset = VabAsset {
        shape_map: [(1, vec![0])].into(),
        morph_map: default(),
        render_meshes: vec![RenderMeshGroup {
            mesh,
            material: MeshMaterial::Color,
            local_bounds: [24.0, 24.0, 40.0, 40.0],
        }],
        baked: BakedMovie {
            frame_rate: 30.0,
            skins: vec![],
            clips: vec![BakedClip {
                name: "default".into(),
                start_frame: 0,
                events: vec![],
                frames: vec![vec![BakedNode::Group {
                    children: vec![BakedNode::Group {
                        children: vec![BakedNode::Shape {
                            id: 1,
                            ratio: 0,
                            transform: AnimTransform::default(),
                        }],
                        filters: vec![AnimFilter::BlurFilter(AnimBlurFilter {
                            blur_x: 5 * 65536,
                            blur_y: 5 * 65536,
                            num_passes: 1,
                        })],
                        blend_mode: 0,
                    }],
                    filters: vec![AnimFilter::BlurFilter(AnimBlurFilter {
                        blur_x: 9 * 65536,
                        blur_y: 9 * 65536,
                        num_passes: 1,
                    })],
                    blend_mode: 0,
                }]],
            }],
        },
    };
    let asset = app
        .world_mut()
        .resource_mut::<Assets<VabAsset>>()
        .add(asset);
    app.world_mut().spawn((
        VabAssetHandle(asset),
        VabPlayer::default(),
        OffscreenViewTarget::new(output.clone(), size),
        bevy::render::view::Msaa::Sample4,
    ));

    let pixels = capture(&mut app, output, size);
    let alpha = |x: usize, y: usize| pixels[(y * size.x as usize + x) * 4 + 3];
    assert!(alpha(32, 32) > 200, "blur erased the shape center");
    assert!(alpha(21, 32) > 0, "blur did not expand left of the shape");
    assert_eq!(alpha(10, 32), 0, "blur escaped its computed bounds");
    let diagnostics = app
        .sub_app(RenderApp)
        .world()
        .resource::<FlashRenderDiagnostics>();
    assert!(
        diagnostics.texture_reuses >= 3,
        "MSAA blur targets were not reused: {diagnostics:?}"
    );
}

#[test]
#[ignore = "requires GPU"]
fn color_matrix_filter_transforms_straight_color_and_alpha() {
    let mut app = app(PathBuf::from("assets"));
    let size = UVec2::splat(64);
    let output = target(&mut app, size);
    let mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![
            [16.0, 16.0, 0.0],
            [48.0, 16.0, 0.0],
            [16.0, 48.0, 0.0],
            [16.0, 48.0, 0.0],
            [48.0, 16.0, 0.0],
            [48.0, 48.0, 0.0],
        ],
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0, 0.0, 0.0, 0.5]; 6]);
    let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(mesh);
    let asset = app
        .world_mut()
        .resource_mut::<Assets<VabAsset>>()
        .add(VabAsset {
            shape_map: [(1, vec![0])].into(),
            morph_map: default(),
            render_meshes: vec![RenderMeshGroup {
                mesh,
                material: MeshMaterial::Color,
                local_bounds: [16.0, 16.0, 48.0, 48.0],
            }],
            baked: BakedMovie {
                frame_rate: 30.0,
                skins: vec![],
                clips: vec![BakedClip {
                    name: "default".into(),
                    start_frame: 0,
                    events: vec![],
                    frames: vec![vec![BakedNode::Group {
                        children: vec![BakedNode::Shape {
                            id: 1,
                            ratio: 0,
                            transform: AnimTransform::default(),
                        }],
                        filters: vec![AnimFilter::ColorMatrixFilter(AnimColorMatrixFilter {
                            // Replace red with green while preserving alpha.
                            matrix: [
                                0.0, 0.0, 0.0, 0.0, 0.0, // red
                                1.0, 0.0, 0.0, 0.0, 0.0, // green
                                0.0, 0.0, 0.0, 0.0, 0.0, // blue
                                0.0, 0.0, 0.0, 1.0, 0.0, // alpha
                            ],
                        })],
                        blend_mode: 0,
                    }]],
                }],
            },
        });
    app.world_mut().spawn((
        VabAssetHandle(asset),
        VabPlayer::default(),
        OffscreenViewTarget::new(output.clone(), size),
        bevy::render::view::Msaa::Off,
    ));

    let pixels = capture(&mut app, output, size);
    let center = &pixels[(32 * size.x as usize + 32) * 4..][..4];
    assert!(
        center[0] < 3 && center[1] > 250 && center[2] < 3,
        "color matrix did not map red to green: {center:?}"
    );
    assert!(
        (125..=130).contains(&center[3]),
        "color matrix changed alpha: {center:?}"
    );
    let diagnostics = app
        .sub_app(RenderApp)
        .world()
        .resource::<FlashRenderDiagnostics>();
    assert_eq!(diagnostics.unsupported_vab_layers, 0);
}

#[test]
#[ignore = "requires GPU"]
fn convolution_filter_handles_edges_divisor_bias_and_preserved_alpha() {
    let mut app = app(PathBuf::from("assets"));
    let size = UVec2::splat(64);
    let clamp_output = target(&mut app, size);
    let default_output = target(&mut app, size);
    let arithmetic_output = target(&mut app, size);

    let spawn =
        |app: &mut App, output: Handle<Image>, color: [f32; 4], filter: AnimConvolutionFilter| {
            let mesh = Mesh::new(
                PrimitiveTopology::TriangleList,
                RenderAssetUsages::RENDER_WORLD,
            )
            .with_inserted_attribute(
                Mesh::ATTRIBUTE_POSITION,
                vec![
                    [16.0, 16.0, 0.0],
                    [48.0, 16.0, 0.0],
                    [16.0, 48.0, 0.0],
                    [16.0, 48.0, 0.0],
                    [48.0, 16.0, 0.0],
                    [48.0, 48.0, 0.0],
                ],
            )
            .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, vec![color; 6]);
            let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(mesh);
            let asset = app
                .world_mut()
                .resource_mut::<Assets<VabAsset>>()
                .add(VabAsset {
                    shape_map: [(1, vec![0])].into(),
                    morph_map: default(),
                    render_meshes: vec![RenderMeshGroup {
                        mesh,
                        material: MeshMaterial::Color,
                        local_bounds: [16.0, 16.0, 48.0, 48.0],
                    }],
                    baked: BakedMovie {
                        frame_rate: 30.0,
                        skins: vec![],
                        clips: vec![BakedClip {
                            name: "default".into(),
                            start_frame: 0,
                            events: vec![],
                            frames: vec![vec![BakedNode::Group {
                                children: vec![BakedNode::Shape {
                                    id: 1,
                                    ratio: 0,
                                    transform: AnimTransform::default(),
                                }],
                                filters: vec![AnimFilter::ConvolutionFilter(filter)],
                                blend_mode: 0,
                            }]],
                        }],
                    },
                });
            app.world_mut().spawn((
                VabAssetHandle(asset),
                VabPlayer::default(),
                OffscreenViewTarget::new(output, size),
                bevy::render::view::Msaa::Off,
            ));
        };
    let edge_filter = |flags| AnimConvolutionFilter {
        num_matrix_rows: 1,
        num_matrix_cols: 3,
        // Select the pixel immediately to the left.
        matrix: vec![1.0, 0.0, 0.0],
        divisor: 1.0,
        bias: 0.0,
        default_color_r: 0,
        default_color_g: 255,
        default_color_b: 0,
        default_color_a: 255,
        flags,
    };
    spawn(
        &mut app,
        clamp_output.clone(),
        [1.0, 0.0, 0.0, 1.0],
        edge_filter(0x03),
    );
    spawn(
        &mut app,
        default_output.clone(),
        [1.0, 0.0, 0.0, 1.0],
        edge_filter(0x01),
    );
    spawn(
        &mut app,
        arithmetic_output.clone(),
        [0.0, 0.0, 1.0, 0.5],
        AnimConvolutionFilter {
            num_matrix_rows: 1,
            num_matrix_cols: 1,
            matrix: vec![1.0],
            divisor: 2.0,
            bias: 51.0,
            default_color_r: 0,
            default_color_g: 0,
            default_color_b: 0,
            default_color_a: 0,
            flags: 0x03,
        },
    );

    let pixel = |pixels: &[u8], x: usize, y: usize| {
        let offset = (y * 64 + x) * 4;
        <[u8; 4]>::try_from(&pixels[offset..offset + 4]).unwrap()
    };
    let clamped = capture(&mut app, clamp_output, size);
    let substituted = capture(&mut app, default_output, size);
    let arithmetic = capture(&mut app, arithmetic_output, size);
    let clamp_edge = pixel(&clamped, 16, 32);
    let default_edge = pixel(&substituted, 16, 32);
    assert!(
        clamp_edge[0] > 250 && clamp_edge[1] < 3 && clamp_edge[3] > 250,
        "clamp did not duplicate the source edge: {clamp_edge:?}"
    );
    assert!(
        default_edge[0] < 3 && default_edge[1] > 250 && default_edge[3] > 250,
        "unclamped convolution did not use its default color: {default_edge:?}"
    );
    let transformed = pixel(&arithmetic, 32, 32);
    // Flash/Ruffle run filter arithmetic on encoded sRGB channel values in an
    // Rgba8Unorm working surface. Blue / 2 + 0.2 therefore displays as 0.7
    // (about 179), rather than the roughly 218 produced by linear-light math.
    assert!(
        (48..=54).contains(&transformed[0])
            && (48..=54).contains(&transformed[1])
            && (176..=182).contains(&transformed[2]),
        "divisor/bias calculation was incorrect: {transformed:?}"
    );
    assert!(
        (125..=130).contains(&transformed[3]),
        "preserveAlpha changed source alpha: {transformed:?}"
    );
    assert_eq!(
        app.sub_app(RenderApp)
            .world()
            .resource::<FlashRenderDiagnostics>()
            .unsupported_vab_layers,
        0
    );
}

#[test]
#[ignore = "requires GPU"]
fn drop_shadow_filter_offsets_outer_and_inner_shadow() {
    let mut app = app(PathBuf::from("assets"));
    let size = UVec2::splat(64);
    let outer_output = target(&mut app, size);
    let inner_output = target(&mut app, size);
    let mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![
            [20.0, 20.0, 0.0],
            [36.0, 20.0, 0.0],
            [20.0, 36.0, 0.0],
            [20.0, 36.0, 0.0],
            [36.0, 20.0, 0.0],
            [36.0, 36.0, 0.0],
        ],
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0, 0.0, 0.0, 1.0]; 6]);
    let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(mesh);
    let spawn = |app: &mut App, output: Handle<Image>, flags: u8| {
        let asset = app
            .world_mut()
            .resource_mut::<Assets<VabAsset>>()
            .add(VabAsset {
                shape_map: [(1, vec![0])].into(),
                morph_map: default(),
                render_meshes: vec![RenderMeshGroup {
                    mesh: mesh.clone(),
                    material: MeshMaterial::Color,
                    local_bounds: [20.0, 20.0, 36.0, 36.0],
                }],
                baked: BakedMovie {
                    frame_rate: 30.0,
                    skins: vec![],
                    clips: vec![BakedClip {
                        name: "default".into(),
                        start_frame: 0,
                        events: vec![],
                        frames: vec![vec![BakedNode::Group {
                            children: vec![BakedNode::Shape {
                                id: 1,
                                ratio: 0,
                                transform: AnimTransform::default(),
                            }],
                            filters: vec![AnimFilter::DropShadowFilter(AnimDropShadowFilter {
                                flags,
                                color_r: 0,
                                color_g: 0,
                                color_b: 0,
                                color_a: 255,
                                blur_x: 65536,
                                blur_y: 65536,
                                angle: 0,
                                distance: 8 * 65536,
                                strength: 256,
                                num_passes: 1,
                            })],
                            blend_mode: 0,
                        }]],
                    }],
                },
            });
        app.world_mut().spawn((
            VabAssetHandle(asset),
            VabPlayer::default(),
            OffscreenViewTarget::new(output, size),
            bevy::render::view::Msaa::Off,
        ));
    };
    spawn(&mut app, outer_output.clone(), 0x20);
    spawn(&mut app, inner_output.clone(), 0x80 | 0x20);

    let outer = capture(&mut app, outer_output, size);
    let pixel = |pixels: &[u8], x: usize, y: usize| {
        let offset = (y * 64 + x) * 4;
        <[u8; 4]>::try_from(&pixels[offset..offset + 4]).unwrap()
    };
    let source = pixel(&outer, 24, 28);
    let shadow = pixel(&outer, 40, 28);
    assert!(
        source[0] > 240 && source[3] > 240,
        "source lost: {source:?}"
    );
    assert!(
        shadow[0] < 5 && shadow[1] < 5 && shadow[2] < 5 && shadow[3] > 240,
        "outer shadow was not shifted right: {shadow:?}"
    );
    assert_eq!(pixel(&outer, 12, 28)[3], 0, "shadow shifted left");

    let inner = capture(&mut app, inner_output, size);
    let shaded_edge = pixel(&inner, 22, 28);
    let unshaded = pixel(&inner, 32, 28);
    assert!(
        shaded_edge[0] < 5 && shaded_edge[3] > 240,
        "inner shadow missing from the opposite edge: {shaded_edge:?}"
    );
    assert!(
        unshaded[0] > 240 && unshaded[3] > 240,
        "inner shadow covered the wrong side: {unshaded:?}"
    );
    assert_eq!(pixel(&inner, 40, 28)[3], 0, "inner shadow escaped source");
    assert_eq!(
        app.sub_app(RenderApp)
            .world()
            .resource::<FlashRenderDiagnostics>()
            .unsupported_vab_layers,
        0
    );
}

#[test]
#[ignore = "requires GPU"]
fn bevel_filter_builds_highlight_and_shadow_from_opposite_offsets() {
    let mut app = app(PathBuf::from("assets"));
    let size = UVec2::splat(64);
    let output = target(&mut app, size);
    let mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![
            [20.0, 20.0, 0.0],
            [44.0, 20.0, 0.0],
            [20.0, 44.0, 0.0],
            [20.0, 44.0, 0.0],
            [44.0, 20.0, 0.0],
            [44.0, 44.0, 0.0],
        ],
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.5, 0.5, 0.5, 1.0]; 6]);
    let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(mesh);
    let asset = app
        .world_mut()
        .resource_mut::<Assets<VabAsset>>()
        .add(VabAsset {
            shape_map: [(1, vec![0])].into(),
            morph_map: default(),
            render_meshes: vec![RenderMeshGroup {
                mesh,
                material: MeshMaterial::Color,
                local_bounds: [20.0, 20.0, 44.0, 44.0],
            }],
            baked: BakedMovie {
                frame_rate: 30.0,
                skins: vec![],
                clips: vec![BakedClip {
                    name: "default".into(),
                    start_frame: 0,
                    events: vec![],
                    frames: vec![vec![BakedNode::Group {
                        children: vec![BakedNode::Shape {
                            id: 1,
                            ratio: 0,
                            transform: AnimTransform::default(),
                        }],
                        filters: vec![AnimFilter::BevelFilter(AnimBevelFilter {
                            // Inner bevel, with the source retained.
                            flags: 0x80,
                            shadow_color_r: 0,
                            shadow_color_g: 0,
                            shadow_color_b: 0,
                            shadow_color_a: 255,
                            highlight_color_r: 255,
                            highlight_color_g: 255,
                            highlight_color_b: 255,
                            highlight_color_a: 255,
                            blur_x: 65536,
                            blur_y: 65536,
                            angle: 0,
                            distance: 4 * 65536,
                            strength: 256,
                            num_passes: 1,
                        })],
                        blend_mode: 0,
                    }]],
                }],
            },
        });
    app.world_mut().spawn((
        VabAssetHandle(asset),
        VabPlayer::default(),
        OffscreenViewTarget::new(output.clone(), size),
        bevy::render::view::Msaa::Off,
    ));

    let pixels = capture(&mut app, output, size);
    let pixel = |x: usize, y: usize| {
        let offset = (y * 64 + x) * 4;
        <[u8; 4]>::try_from(&pixels[offset..offset + 4]).unwrap()
    };
    let highlight = pixel(22, 32);
    let center = pixel(32, 32);
    let shadow = pixel(42, 32);
    assert!(
        u16::from(highlight[0]) > u16::from(center[0]) + 50 && highlight[3] > 240,
        "left highlight missing: highlight={highlight:?}, center={center:?}"
    );
    assert!(
        u16::from(shadow[0]) + 80 < u16::from(center[0]) && shadow[3] > 240,
        "right shadow missing: shadow={shadow:?}, center={center:?}"
    );
    assert!(
        center[0] > 175 && center[0] < 200,
        "bevel changed the flat center: {center:?}"
    );
    assert_eq!(pixel(16, 32)[3], 0, "inner bevel escaped the source");
    assert_eq!(
        app.sub_app(RenderApp)
            .world()
            .resource::<FlashRenderDiagnostics>()
            .unsupported_vab_layers,
        0
    );
}

#[test]
#[ignore = "requires GPU"]
fn gradient_glow_and_bevel_sample_their_color_ramps() {
    let mut app = app(PathBuf::from("assets"));
    let size = UVec2::splat(64);
    let glow_output = target(&mut app, size);
    let bevel_output = target(&mut app, size);
    let mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![
            [20.0, 20.0, 0.0],
            [44.0, 20.0, 0.0],
            [20.0, 44.0, 0.0],
            [20.0, 44.0, 0.0],
            [44.0, 20.0, 0.0],
            [44.0, 44.0, 0.0],
        ],
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.5, 0.5, 0.5, 1.0]; 6]);
    let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(mesh);
    let spawn = |app: &mut App, output: Handle<Image>, filter: AnimFilter| {
        let asset = app
            .world_mut()
            .resource_mut::<Assets<VabAsset>>()
            .add(VabAsset {
                shape_map: [(1, vec![0])].into(),
                morph_map: default(),
                render_meshes: vec![RenderMeshGroup {
                    mesh: mesh.clone(),
                    material: MeshMaterial::Color,
                    local_bounds: [20.0, 20.0, 44.0, 44.0],
                }],
                baked: BakedMovie {
                    frame_rate: 30.0,
                    skins: vec![],
                    clips: vec![BakedClip {
                        name: "default".into(),
                        start_frame: 0,
                        events: vec![],
                        frames: vec![vec![BakedNode::Group {
                            children: vec![BakedNode::Shape {
                                id: 1,
                                ratio: 0,
                                transform: AnimTransform::default(),
                            }],
                            filters: vec![filter],
                            blend_mode: 0,
                        }]],
                    }],
                },
            });
        app.world_mut().spawn((
            VabAssetHandle(asset),
            VabPlayer::default(),
            OffscreenViewTarget::new(output, size),
            bevy::render::view::Msaa::Off,
        ));
    };
    spawn(
        &mut app,
        glow_output.clone(),
        AnimFilter::GradientGlowFilter(AnimGradientFilter {
            flags: 0x20,
            colors: vec![
                AnimGradientRecord {
                    ratio: 0,
                    color_r: 0,
                    color_g: 255,
                    color_b: 0,
                    color_a: 0,
                },
                AnimGradientRecord {
                    ratio: 255,
                    color_r: 0,
                    color_g: 255,
                    color_b: 0,
                    color_a: 255,
                },
            ],
            blur_x: 65_536,
            blur_y: 65_536,
            angle: 0,
            distance: 8 * 65_536,
            strength: 256,
            num_passes: 1,
        }),
    );
    spawn(
        &mut app,
        bevel_output.clone(),
        AnimFilter::GradientBevelFilter(AnimGradientFilter {
            flags: 0x80 | 0x20,
            colors: vec![
                AnimGradientRecord {
                    ratio: 0,
                    color_r: 255,
                    color_g: 255,
                    color_b: 255,
                    color_a: 255,
                },
                AnimGradientRecord {
                    ratio: 128,
                    color_r: 128,
                    color_g: 128,
                    color_b: 128,
                    color_a: 0,
                },
                AnimGradientRecord {
                    ratio: 255,
                    color_r: 0,
                    color_g: 0,
                    color_b: 0,
                    color_a: 255,
                },
            ],
            blur_x: 65_536,
            blur_y: 65_536,
            angle: 0,
            distance: 4 * 65_536,
            strength: 256,
            num_passes: 1,
        }),
    );

    let pixel = |pixels: &[u8], x: usize, y: usize| {
        let offset = (y * 64 + x) * 4;
        <[u8; 4]>::try_from(&pixels[offset..offset + 4]).unwrap()
    };
    let glow = capture(&mut app, glow_output, size);
    let source = pixel(&glow, 24, 32);
    let shifted = pixel(&glow, 48, 32);
    assert!(
        source[0] > 170 && source[3] > 240,
        "gradient glow lost its source: {source:?}"
    );
    assert!(
        shifted[1] > 240 && shifted[0] < 10 && shifted[3] > 240,
        "gradient glow did not sample its opaque green ramp endpoint: {shifted:?}"
    );

    let bevel = capture(&mut app, bevel_output, size);
    let highlight = pixel(&bevel, 22, 32);
    let center = pixel(&bevel, 32, 32);
    let shadow = pixel(&bevel, 42, 32);
    assert!(
        u16::from(highlight[0]) > u16::from(center[0]) + 50,
        "gradient bevel highlight is missing: highlight={highlight:?}, center={center:?}"
    );
    assert!(
        u16::from(shadow[0]) + 80 < u16::from(center[0]),
        "gradient bevel shadow is missing: shadow={shadow:?}, center={center:?}"
    );
    assert!(
        center[0] > 175 && center[0] < 200,
        "gradient bevel did not use its transparent middle stop: {center:?}"
    );
    assert_eq!(
        app.sub_app(RenderApp)
            .world()
            .resource::<FlashRenderDiagnostics>()
            .unsupported_vab_layers,
        0
    );
}

#[test]
#[ignore = "requires GPU"]
fn vab_instance_blur_is_composited_inside_one_transparent_item() {
    let mut app = app(PathBuf::from("assets"));
    let size = UVec2::splat(64);
    let output = target(&mut app, size);
    let camera = app
        .world_mut()
        .spawn((
            Camera2d,
            Camera {
                clear_color: ClearColorConfig::Custom(Color::NONE),
                ..default()
            },
            RenderTarget::Image(output.clone().into()),
        ))
        .id();
    let mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![
            [-8.0, -8.0, 0.0],
            [8.0, -8.0, 0.0],
            [-8.0, 8.0, 0.0],
            [-8.0, 8.0, 0.0],
            [8.0, -8.0, 0.0],
            [8.0, 8.0, 0.0],
        ],
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0, 0.0, 0.0, 1.0]; 6]);
    let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(mesh);
    let asset = VabAsset {
        shape_map: [(1, vec![0])].into(),
        morph_map: default(),
        render_meshes: vec![RenderMeshGroup {
            mesh,
            material: MeshMaterial::Color,
            local_bounds: [-8.0, -8.0, 8.0, 8.0],
        }],
        baked: BakedMovie {
            frame_rate: 30.0,
            skins: vec![],
            clips: vec![BakedClip {
                name: "default".into(),
                start_frame: 0,
                events: vec![],
                frames: vec![
                    vec![BakedNode::Group {
                        children: vec![BakedNode::Shape {
                            id: 1,
                            ratio: 0,
                            transform: AnimTransform::default(),
                        }],
                        filters: vec![AnimFilter::BlurFilter(AnimBlurFilter {
                            blur_x: 9 * 65536,
                            blur_y: 9 * 65536,
                            num_passes: 1,
                        })],
                        blend_mode: 0,
                    }];
                    2
                ],
            }],
        },
    };
    let asset = app
        .world_mut()
        .resource_mut::<Assets<VabAsset>>()
        .add(asset);
    let mut player = VabPlayer::default();
    player.pause();
    let entity = app.world_mut().spawn((VabAssetHandle(asset), player)).id();

    let pixels = capture_camera(&mut app, output, size);
    let alpha = |x: usize, y: usize| pixels[(y * size.x as usize + x) * 4 + 3];
    assert!(alpha(32, 32) > 200, "VAB instance blur erased the center");
    assert!(
        alpha(21, 32) > 0,
        "VAB instance blur did not expand outside geometry"
    );
    let diagnostics = app
        .sub_app(RenderApp)
        .world()
        .resource::<FlashRenderDiagnostics>();
    assert_eq!(diagnostics.vab_instances_queued, 1);
    assert_eq!(diagnostics.unsupported_vab_layers, 0);
    assert_eq!(diagnostics.vab_filter_cache_hits, 1);
    assert_eq!(diagnostics.vab_filter_cache_misses, 0);
    assert_eq!(diagnostics.vab_filter_cache_entries, 1);
    assert!(diagnostics.vab_filter_cache_bytes > 0);

    app.world_mut()
        .entity_mut(entity)
        .get_mut::<VabPlayer>()
        .unwrap()
        .current_frame = 1;
    app.update();
    let diagnostics = app
        .sub_app(RenderApp)
        .world()
        .resource::<FlashRenderDiagnostics>();
    assert_eq!(diagnostics.vab_filter_cache_hits, 0);
    assert_eq!(diagnostics.vab_filter_cache_misses, 1);
    assert_eq!(diagnostics.vab_filter_cache_entries, 1);

    app.update();
    let diagnostics = app
        .sub_app(RenderApp)
        .world()
        .resource::<FlashRenderDiagnostics>();
    assert_eq!(diagnostics.vab_filter_cache_hits, 1);
    assert_eq!(diagnostics.vab_filter_cache_misses, 0);

    app.world_mut()
        .entity_mut(entity)
        .get_mut::<Transform>()
        .unwrap()
        .translation
        .x = 3.0;
    app.update();
    let diagnostics = app
        .sub_app(RenderApp)
        .world()
        .resource::<FlashRenderDiagnostics>();
    assert_eq!(diagnostics.vab_filter_cache_hits, 1);
    assert_eq!(diagnostics.vab_filter_cache_misses, 0);

    app.world_mut()
        .entity_mut(entity)
        .get_mut::<Transform>()
        .unwrap()
        .scale = Vec3::splat(2.0);
    app.update();
    let diagnostics = app
        .sub_app(RenderApp)
        .world()
        .resource::<FlashRenderDiagnostics>();
    assert_eq!(diagnostics.vab_filter_cache_hits, 0);
    assert_eq!(diagnostics.vab_filter_cache_misses, 1);

    // Changing only the filter sample count must invalidate the isolated output,
    // while the camera and its Transparent2d pipeline remain at 4x MSAA.
    app.world_mut().insert_resource(VabFilterMsaa::Off);
    app.update();
    assert_eq!(app.world().get::<Msaa>(camera), Some(&Msaa::Sample4));
    let diagnostics = app
        .sub_app(RenderApp)
        .world()
        .resource::<FlashRenderDiagnostics>();
    assert_eq!(diagnostics.vab_filter_cache_hits, 0);
    assert_eq!(diagnostics.vab_filter_cache_misses, 1);

    app.world_mut()
        .resource_mut::<VabFilterCacheSettings>()
        .max_bytes = 0;
    app.update();
    let diagnostics = app
        .sub_app(RenderApp)
        .world()
        .resource::<FlashRenderDiagnostics>();
    assert_eq!(diagnostics.vab_filter_cache_entries, 0);
    assert_eq!(diagnostics.vab_filter_cache_bytes, 0);
}

#[test]
#[ignore = "requires GPU"]
fn vab_instance_nested_alpha_masks_clip_content_inside_one_transparent_item() {
    let mut app = app(PathBuf::from("assets"));
    let size = UVec2::splat(64);
    let output = target(&mut app, size);
    app.world_mut().spawn((
        Camera2d,
        Camera {
            clear_color: ClearColorConfig::Custom(Color::NONE),
            ..default()
        },
        RenderTarget::Image(output.clone().into()),
    ));
    let rectangle = |left: f32, right: f32, color: [f32; 4]| {
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![
                [left, -8.0, 0.0],
                [right, -8.0, 0.0],
                [left, 8.0, 0.0],
                [left, 8.0, 0.0],
                [right, -8.0, 0.0],
                [right, 8.0, 0.0],
            ],
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, vec![color; 6])
    };
    let mask = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(rectangle(-8.0, 0.0, [1.0; 4]));
    let content = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(rectangle(-8.0, 8.0, [0.0, 1.0, 0.0, 1.0]));
    let half_height_mask = app.world_mut().resource_mut::<Assets<Mesh>>().add(
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![
                [-8.0, -8.0, 0.0],
                [8.0, -8.0, 0.0],
                [-8.0, 0.0, 0.0],
                [-8.0, 0.0, 0.0],
                [8.0, -8.0, 0.0],
                [8.0, 0.0, 0.0],
            ],
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0; 4]; 6]),
    );
    let asset = app
        .world_mut()
        .resource_mut::<Assets<VabAsset>>()
        .add(VabAsset {
            shape_map: [(1, vec![0]), (2, vec![1]), (3, vec![2])].into(),
            morph_map: default(),
            render_meshes: vec![
                RenderMeshGroup {
                    mesh: mask,
                    material: MeshMaterial::Color,
                    local_bounds: [-8.0, -8.0, 0.0, 8.0],
                },
                RenderMeshGroup {
                    mesh: content,
                    material: MeshMaterial::Color,
                    local_bounds: [-8.0, -8.0, 8.0, 8.0],
                },
                RenderMeshGroup {
                    mesh: half_height_mask,
                    material: MeshMaterial::Color,
                    local_bounds: [-8.0, -8.0, 8.0, 0.0],
                },
            ],
            baked: BakedMovie {
                frame_rate: 30.0,
                skins: vec![],
                clips: vec![BakedClip {
                    name: "default".into(),
                    start_frame: 0,
                    events: vec![],
                    frames: vec![vec![BakedNode::Mask {
                        mask: vec![BakedNode::Shape {
                            id: 1,
                            ratio: 0,
                            transform: AnimTransform::default(),
                        }],
                        children: vec![BakedNode::Mask {
                            mask: vec![BakedNode::Shape {
                                id: 3,
                                ratio: 0,
                                transform: AnimTransform::default(),
                            }],
                            children: vec![BakedNode::Shape {
                                id: 2,
                                ratio: 0,
                                transform: AnimTransform::default(),
                            }],
                        }],
                    }]],
                }],
            },
        });
    app.world_mut()
        .spawn((VabAssetHandle(asset), VabPlayer::default()));

    let pixels = capture_camera(&mut app, output, size);
    let pixel = |x: usize, y: usize| {
        let offset = (y * size.x as usize + x) * 4;
        &pixels[offset..offset + 4]
    };
    let upper_or_lower = [pixel(28, 28), pixel(28, 36)];
    let visible = upper_or_lower
        .iter()
        .find(|pixel| pixel[3] > 240)
        .expect("nested mask removed both vertical halves");
    let clipped_vertical = upper_or_lower
        .iter()
        .find(|pixel| pixel[3] == 0)
        .expect("nested mask did not clip either vertical half");
    let clipped_horizontal = pixel(36, 28);
    assert!(
        visible[1] > 240 && visible[3] > 240,
        "masked content is missing: {visible:?}"
    );
    assert_eq!(
        clipped_vertical[3], 0,
        "content escaped the nested vertical mask: {clipped_vertical:?}"
    );
    assert_eq!(
        clipped_horizontal[3], 0,
        "content escaped the horizontal mask: {clipped_horizontal:?}"
    );
    let diagnostics = app
        .sub_app(RenderApp)
        .world()
        .resource::<FlashRenderDiagnostics>();
    assert_eq!(diagnostics.vab_instances_queued, 1);
    assert_eq!(diagnostics.unsupported_vab_layers, 0);
    assert_eq!(diagnostics.vab_mask_layers, 2);
}

#[test]
#[ignore = "requires GPU"]
fn vab_instance_fixed_function_blends_and_lighten_fallback() {
    let mut app = app(PathBuf::from("assets"));
    let size = UVec2::new(80, 64);
    let output = target(&mut app, size);
    app.world_mut().spawn((
        Camera2d,
        CompositingSpace::Srgb,
        Camera {
            clear_color: ClearColorConfig::Custom(Color::NONE),
            ..default()
        },
        RenderTarget::Image(output.clone().into()),
    ));
    let rectangle = |color: [f32; 4]| {
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![
                [-6.0, -6.0, 0.0],
                [6.0, -6.0, 0.0],
                [-6.0, 6.0, 0.0],
                [-6.0, 6.0, 0.0],
                [6.0, -6.0, 0.0],
                [6.0, 6.0, 0.0],
            ],
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, vec![color; 6])
    };
    let background = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(rectangle([0.1, 0.2, 0.4, 1.0]));
    let source = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(rectangle([0.3, 0.2, 0.1, 1.0]));
    let translucent_source = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(rectangle([0.3, 0.2, 0.1, 0.5]));
    let transform = |x: f32| AnimTransform {
        matrix: AnimMatrix {
            tx: x,
            ..AnimMatrix::IDENTITY
        },
        ..AnimTransform::default()
    };
    let mut nodes = Vec::new();
    for (x, blend_mode, source_id) in [(-24.0, 8, 2), (-8.0, 4, 2), (8.0, 9, 2), (24.0, 5, 3)] {
        nodes.push(BakedNode::Shape {
            id: 1,
            ratio: 0,
            transform: transform(x),
        });
        nodes.push(BakedNode::Group {
            children: vec![BakedNode::Shape {
                id: source_id,
                ratio: 0,
                transform: transform(x),
            }],
            filters: vec![],
            blend_mode,
        });
    }
    let asset = app
        .world_mut()
        .resource_mut::<Assets<VabAsset>>()
        .add(VabAsset {
            shape_map: [(1, vec![0]), (2, vec![1]), (3, vec![2])].into(),
            morph_map: default(),
            render_meshes: vec![
                RenderMeshGroup {
                    mesh: background,
                    material: MeshMaterial::Color,
                    local_bounds: [-6.0, -6.0, 6.0, 6.0],
                },
                RenderMeshGroup {
                    mesh: source,
                    material: MeshMaterial::Color,
                    local_bounds: [-6.0, -6.0, 6.0, 6.0],
                },
                RenderMeshGroup {
                    mesh: translucent_source,
                    material: MeshMaterial::Color,
                    local_bounds: [-6.0, -6.0, 6.0, 6.0],
                },
            ],
            baked: BakedMovie {
                frame_rate: 30.0,
                skins: vec![],
                clips: vec![BakedClip {
                    name: "default".into(),
                    start_frame: 0,
                    events: vec![],
                    frames: vec![nodes],
                }],
            },
        });
    app.world_mut()
        .spawn((VabAssetHandle(asset), VabPlayer::default()));

    let mut pixels = Vec::new();
    // Pipeline specialization may finish a few frames after geometry first
    // appears; wait for the blend pipelines rather than accepting that frame.
    for _ in 0..32 {
        pixels = capture_camera(&mut app, output.clone(), size);
        let at = |x: usize| &pixels[(32 * size.x as usize + x) * 4..][..4];
        let add = at(16);
        let screen = at(32);
        let subtract = at(48);
        if add[0] > screen[0] && add[1] > screen[1] && add[2] > screen[2] && subtract[0] < 5 {
            break;
        }
    }
    let pixel = |x: usize| {
        let offset = (32 * size.x as usize + x) * 4;
        &pixels[offset..offset + 4]
    };
    let add = pixel(16);
    let screen = pixel(32);
    let subtract = pixel(48);
    let lighten = pixel(64);
    assert!(
        add[0] > screen[0] && add[1] > screen[1] && add[2] > screen[2],
        "Add and Screen equations were not applied: add={add:?}, screen={screen:?}"
    );
    assert!(
        subtract[0] < 5 && subtract[1] < 5 && (75..=90).contains(&subtract[2]),
        "Subtract did not calculate destination minus source: {subtract:?}"
    );
    assert!(
        (84..=94).contains(&lighten[0])
            && (119..=130).contains(&lighten[1])
            && (165..=175).contains(&lighten[2]),
        "Lighten fallback did not take the maximum premultiplied attachment value: {lighten:?}"
    );
    let diagnostics = app
        .sub_app(RenderApp)
        .world()
        .resource::<FlashRenderDiagnostics>();
    assert_eq!(diagnostics.vab_instances_queued, 1);
    assert_eq!(diagnostics.unsupported_vab_layers, 0);
}

#[test]
#[ignore = "requires GPU"]
fn vab_instance_honors_camera_srgb_compositing() {
    let mut app = app(PathBuf::from("assets"));
    let size = UVec2::splat(64);
    let output = target(&mut app, size);
    app.world_mut().spawn((
        Camera2d,
        CompositingSpace::Srgb,
        Camera {
            clear_color: ClearColorConfig::Custom(Color::BLACK),
            ..default()
        },
        RenderTarget::Image(output.clone().into()),
    ));

    // VAB mesh colors are linearized at load time. The instance shader turns
    // this value back into encoded 0.5 before Flash-style compositing.
    let encoded_half_as_linear = 0.214_041_14;
    let mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![
            [-8.0, -8.0, 0.0],
            [8.0, -8.0, 0.0],
            [-8.0, 8.0, 0.0],
            [-8.0, 8.0, 0.0],
            [8.0, -8.0, 0.0],
            [8.0, 8.0, 0.0],
        ],
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_COLOR,
        vec![
            [
                encoded_half_as_linear,
                encoded_half_as_linear,
                encoded_half_as_linear,
                0.5,
            ];
            6
        ],
    );
    let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(mesh);
    let asset = app
        .world_mut()
        .resource_mut::<Assets<VabAsset>>()
        .add(VabAsset {
            shape_map: [(1, vec![0])].into(),
            morph_map: default(),
            render_meshes: vec![RenderMeshGroup {
                mesh,
                material: MeshMaterial::Color,
                local_bounds: [-8.0, -8.0, 8.0, 8.0],
            }],
            baked: BakedMovie {
                frame_rate: 30.0,
                skins: vec![],
                clips: vec![BakedClip {
                    name: "default".into(),
                    start_frame: 0,
                    events: vec![],
                    frames: vec![vec![BakedNode::Shape {
                        id: 1,
                        ratio: 0,
                        transform: AnimTransform::default(),
                    }]],
                }],
            },
        });
    app.world_mut()
        .spawn((VabAssetHandle(asset), VabPlayer::default()));

    let pixels = capture_camera(&mut app, output, size);
    let offset = (32 * size.x as usize + 32) * 4;
    let center = &pixels[offset..offset + 4];
    assert!(
        center[..3]
            .iter()
            .all(|channel| (59..=69).contains(channel)),
        "sRGB compositing should blend encoded 0.5 at alpha 0.5 to about 0.25: {center:?}"
    );
    assert!(center[3] > 250, "opaque camera clear was lost: {center:?}");
}

#[test]
#[ignore = "requires GPU"]
fn vab_instance_filters_prepare_at_each_camera_pixel_scale() {
    let mut app = app(PathBuf::from("assets"));
    app.world_mut().insert_resource(VabFilterMsaa::Off);
    let size = UVec2::splat(64);
    let output_a = target(&mut app, size);
    let output_b = target(&mut app, size);
    app.world_mut().spawn((
        Camera2d,
        Camera {
            clear_color: ClearColorConfig::Custom(Color::NONE),
            ..default()
        },
        RenderTarget::Image(output_a.clone().into()),
    ));
    app.world_mut().spawn((
        Camera2d,
        Camera {
            clear_color: ClearColorConfig::Custom(Color::NONE),
            ..default()
        },
        Projection::Orthographic(OrthographicProjection {
            scale: 0.5,
            ..OrthographicProjection::default_2d()
        }),
        RenderTarget::Image(output_b.clone().into()),
    ));
    let mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![
            [-8.0, -8.0, 0.0],
            [8.0, -8.0, 0.0],
            [-8.0, 8.0, 0.0],
            [-8.0, 8.0, 0.0],
            [8.0, -8.0, 0.0],
            [8.0, 8.0, 0.0],
        ],
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0, 0.0, 0.0, 1.0]; 6]);
    let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(mesh);
    let asset = app
        .world_mut()
        .resource_mut::<Assets<VabAsset>>()
        .add(VabAsset {
            shape_map: [(1, vec![0])].into(),
            morph_map: default(),
            render_meshes: vec![RenderMeshGroup {
                mesh,
                material: MeshMaterial::Color,
                local_bounds: [-8.0, -8.0, 8.0, 8.0],
            }],
            baked: BakedMovie {
                frame_rate: 30.0,
                skins: vec![],
                clips: vec![BakedClip {
                    name: "default".into(),
                    start_frame: 0,
                    events: vec![],
                    frames: vec![vec![BakedNode::Group {
                        children: vec![BakedNode::Shape {
                            id: 1,
                            ratio: 0,
                            transform: AnimTransform::default(),
                        }],
                        filters: vec![AnimFilter::BlurFilter(AnimBlurFilter {
                            blur_x: 9 * 65536,
                            blur_y: 9 * 65536,
                            num_passes: 1,
                        })],
                        blend_mode: 0,
                    }]],
                }],
            },
        });
    app.world_mut()
        .spawn((VabAssetHandle(asset), VabPlayer::default()));

    let pixels_a = capture_camera(&mut app, output_a, size);
    let pixels_b = capture_camera(&mut app, output_b, size);
    assert!(pixels_a.chunks_exact(4).any(|pixel| pixel[3] > 0));
    assert!(pixels_b.chunks_exact(4).any(|pixel| pixel[3] > 0));
    let diagnostics = app
        .sub_app(RenderApp)
        .world()
        .resource::<FlashRenderDiagnostics>();
    assert_eq!(diagnostics.vab_instances_queued, 2);
    assert_eq!(diagnostics.vab_filter_layers, 2);
    assert!(
        diagnostics.vab_filter_output_pixels > 34 * 34 * 2,
        "zoomed camera reused the unscaled filter target: {diagnostics:?}"
    );
    let store = app.world().resource::<DiagnosticsStore>();
    assert!(
        store
            .get(&VabDiagnosticsPlugin::CPU_MS)
            .and_then(|entry| entry.value())
            .is_some()
    );
    assert!(
        store
            .get(&VabDiagnosticsPlugin::POOL_BYTES)
            .and_then(|entry| entry.value())
            .is_some()
    );
    assert!(
        store
            .get(&DiagnosticPath::const_new("render/vab_filter_frame_low"))
            .is_none()
    );
}

#[test]
#[ignore = "requires GPU"]
fn vab_instance_glow_preserves_source_and_uses_three_work_textures() {
    let mut app = app_with_render_diagnostics(PathBuf::from("assets"), true);
    let size = UVec2::splat(64);
    let output = target(&mut app, size);
    app.world_mut().spawn((
        Camera2d,
        Camera {
            clear_color: ClearColorConfig::Custom(Color::NONE),
            ..default()
        },
        RenderTarget::Image(output.clone().into()),
    ));
    let mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![
            [-8.0, -8.0, 0.0],
            [8.0, -8.0, 0.0],
            [-8.0, 0.0, 0.0],
            [-8.0, 0.0, 0.0],
            [8.0, -8.0, 0.0],
            [8.0, 0.0, 0.0],
        ],
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0, 1.0, 1.0, 1.0]; 6]);
    let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(mesh);
    let asset = VabAsset {
        shape_map: [(1, vec![0])].into(),
        morph_map: default(),
        render_meshes: vec![RenderMeshGroup {
            mesh,
            material: MeshMaterial::Color,
            local_bounds: [-8.0, -8.0, 8.0, 0.0],
        }],
        baked: BakedMovie {
            frame_rate: 30.0,
            skins: vec![],
            clips: vec![BakedClip {
                name: "default".into(),
                start_frame: 0,
                events: vec![],
                frames: vec![vec![BakedNode::Group {
                    children: vec![BakedNode::Group {
                        children: vec![BakedNode::Shape {
                            id: 1,
                            ratio: 0,
                            transform: AnimTransform::default(),
                        }],
                        filters: vec![AnimFilter::BlurFilter(AnimBlurFilter {
                            blur_x: 5 * 65536,
                            blur_y: 5 * 65536,
                            num_passes: 1,
                        })],
                        blend_mode: 0,
                    }],
                    filters: vec![AnimFilter::GlowFilter(AnimGlowFilter {
                        flags: 0x21,
                        color_r: 255,
                        color_g: 0,
                        color_b: 0,
                        color_a: 255,
                        blur_x: 9 * 65536,
                        blur_y: 9 * 65536,
                        strength: 256,
                        num_passes: 1,
                    })],
                    blend_mode: 0,
                }]],
            }],
        },
    };
    let asset = app
        .world_mut()
        .resource_mut::<Assets<VabAsset>>()
        .add(asset);
    let entity = app
        .world_mut()
        .spawn((VabAssetHandle(asset), VabPlayer::default()))
        .id();

    let pixels = capture_camera(&mut app, output, size);
    let pixel = |x: usize, y: usize| {
        let offset = (y * size.x as usize + x) * 4;
        &pixels[offset..offset + 4]
    };
    let center = pixel(32, 28);
    let nonzero = pixels.chunks_exact(4).filter(|pixel| pixel[3] > 0).count();
    // Force one cache miss so the diagnostics below describe the nested working
    // set as well as the one persistent final output.
    app.world_mut()
        .entity_mut(entity)
        .get_mut::<Transform>()
        .unwrap()
        .scale = Vec3::splat(1.25);
    app.update();
    let diagnostics = app
        .sub_app(RenderApp)
        .world()
        .resource::<FlashRenderDiagnostics>();
    assert!(
        center[0] > 240 && center[1] > 240 && center[2] > 240,
        "glow lost its source image: center={center:?}, nonzero={nonzero}, diagnostics={diagnostics:?}"
    );
    let mirrored = pixel(32, 36);
    assert!(
        mirrored[1] < 100 && mirrored[2] < 100,
        "glow source was vertically mirrored: top={center:?}, bottom={mirrored:?}"
    );
    let glow = pixel(21, 28);
    assert!(
        glow[0] > glow[1].saturating_add(20) && glow[3] > 0,
        "outer red glow is missing: {glow:?}"
    );
    assert_eq!(diagnostics.unsupported_vab_layers, 0);
    assert_eq!(diagnostics.vab_instances_queued, 1);
    assert_eq!(diagnostics.vab_filter_passes, 5);
    assert!(diagnostics.vab_filter_processed_pixels > 5 * 16 * 16);
    let sample = app.world().resource::<VabFilterWorkload>().latest();
    assert_eq!(
        sample.app_frame,
        app.world().resource::<FrameCount>().0,
        "render workload must carry the source app frame count"
    );
    assert_eq!(sample.passes, diagnostics.vab_filter_passes);
    assert!(sample.prepass_cpu_ns > 0);
    assert_eq!(sample.texture_allocations, diagnostics.texture_allocations);
    assert_eq!(
        sample.texture_first_in_bucket_allocations + sample.texture_exhausted_bucket_allocations,
        sample.texture_allocations
    );
    assert_eq!(sample.texture_reuses, diagnostics.texture_reuses);
    assert_eq!(
        sample.pool_end_bytes,
        Some(diagnostics.pooled_texture_bytes)
    );
    assert_eq!(
        sample.pool_end_live_bytes,
        Some(diagnostics.live_transient_bytes)
    );
    assert!(sample.pooled_texture_bytes >= diagnostics.pooled_texture_bytes);
    assert_eq!(
        sample.processed_pixels,
        diagnostics.vab_filter_processed_pixels
    );
    assert!(
        diagnostics.peak_transient_textures > diagnostics.live_transient_textures,
        "nested filter work textures did not return to the pool: {diagnostics:?}"
    );
    assert!(
        diagnostics.peak_transient_bytes > diagnostics.live_transient_bytes,
        "nested filter byte peak was not recorded: {diagnostics:?}"
    );
    assert_eq!(
        diagnostics.pooled_textures,
        diagnostics.live_transient_textures + diagnostics.pooled_idle_textures,
        "resident texture count does not include exactly live plus idle: {diagnostics:?}"
    );
    assert_eq!(
        diagnostics.pooled_texture_bytes,
        diagnostics.live_transient_bytes + diagnostics.pooled_idle_bytes,
        "resident texture bytes do not include exactly live plus idle: {diagnostics:?}"
    );
    assert!(
        diagnostics.texture_pool_buckets > 0
            && diagnostics.largest_texture_pool_bucket > 0
            && diagnostics.largest_texture_pool_bucket <= diagnostics.pooled_textures
            && diagnostics.largest_texture_pool_bucket_bytes <= diagnostics.pooled_texture_bytes,
        "texture bucket metrics are inconsistent: {diagnostics:?}"
    );
    let supports_gpu_timestamps = app
        .sub_app(RenderApp)
        .world()
        .resource::<RenderDevice>()
        .features()
        .contains(WgpuFeatures::TIMESTAMP_QUERY | WgpuFeatures::TIMESTAMP_QUERY_INSIDE_ENCODERS);
    let supports_pass_timestamps = app
        .sub_app(RenderApp)
        .world()
        .resource::<RenderDevice>()
        .features()
        .contains(WgpuFeatures::TIMESTAMP_QUERY | WgpuFeatures::TIMESTAMP_QUERY_INSIDE_PASSES);
    let span = format!(
        "vab_filter_prepass_{:03}",
        app.world().resource::<VabFilterWorkload>().latest().frame % VAB_FILTER_DIAGNOSTIC_SLOTS
    );
    let gpu_path = DiagnosticPath::new(format!("render/{span}/elapsed_gpu"));
    let cpu_path = DiagnosticPath::new(format!("render/{span}/elapsed_cpu"));
    let transparent_path = DiagnosticPath::const_new("render/main_transparent_pass_2d/elapsed_gpu");
    let published_at = app
        .world()
        .resource::<VabFilterWorkload>()
        .recent_since(sample.frame - 1)
        .into_iter()
        .find(|(entry, _)| entry.frame == sample.frame)
        .unwrap()
        .1;
    let started = Instant::now();
    loop {
        let store = app.world().resource::<DiagnosticsStore>();
        let cpu_ready = store
            .get(&cpu_path)
            .and_then(|entry| entry.value())
            .is_some();
        let gpu_ready = store
            .get(&gpu_path)
            .and_then(|entry| entry.measurement())
            .is_some_and(|measurement| measurement.time >= published_at);
        let transparent_ready = store
            .get(&transparent_path)
            .and_then(|entry| entry.value())
            .is_some();
        let frame_ready = (|| {
            let low = store
                .get(&DiagnosticPath::const_new("render/vab_filter_frame_low"))?
                .measurement()?;
            let high = store
                .get(&DiagnosticPath::const_new("render/vab_filter_frame_high"))?
                .measurement()?;
            let frame = ((high.value as u64) << 32) | low.value as u64;
            let elapsed = store
                .get(&DiagnosticPath::new(format!(
                    "render/vab_filter_prepass_{:03}/elapsed_cpu",
                    frame % VAB_FILTER_DIAGNOSTIC_SLOTS
                )))?
                .measurement()?;
            Some(
                frame > 0
                    && frame <= app.world().resource::<VabFilterWorkload>().latest().frame
                    && low.time == high.time
                    && low.time == elapsed.time,
            )
        })()
        .unwrap_or(false);
        if cpu_ready
            && frame_ready
            && (!supports_gpu_timestamps || gpu_ready)
            && (!supports_pass_timestamps || transparent_ready)
        {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "missing render timing: frame={frame_ready} cpu={cpu_ready} filter_gpu={gpu_ready} transparent_gpu={transparent_ready} supports_gpu={supports_gpu_timestamps} supports_pass={supports_pass_timestamps}"
        );
        app.update();
    }
}

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
        nodes: &[bevy_flash_remake::vab_asset::VabCommand],
        asset: &VabAsset,
        b: &mut [f32; 4],
    ) {
        use bevy_flash_remake::vab_asset::VabCommand;
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

#[test]
#[ignore = "requires GPU"]
fn vab_instance_large_layer_crops_and_invalidates_on_camera_motion() {
    let mut app = app(PathBuf::from("assets"));
    app.world_mut().insert_resource(VabFilterMsaa::Off);
    let size = UVec2::splat(64);
    let output = target(&mut app, size);
    let camera = app
        .world_mut()
        .spawn((
            Camera2d,
            Msaa::Off,
            Transform::from_xyz(-1000.0, 0.0, 0.0),
            RenderTarget::Image(output.clone().into()),
        ))
        .id();
    let mut positions = Vec::new();
    let mut colors = Vec::new();
    for (left, right, color) in [
        (-3104.0, 0.0, [1.0, 0.0, 0.0, 1.0]),
        (0.0, 3104.0, [0.0, 0.0, 1.0, 1.0]),
    ] {
        positions.extend([
            [left, -3104.0, 0.0],
            [right, -3104.0, 0.0],
            [left, 3104.0, 0.0],
            [left, 3104.0, 0.0],
            [right, -3104.0, 0.0],
            [right, 3104.0, 0.0],
        ]);
        colors.extend([color; 6]);
    }
    let mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(mesh);
    let asset = app
        .world_mut()
        .resource_mut::<Assets<VabAsset>>()
        .add(VabAsset {
            shape_map: [(1, vec![0])].into(),
            morph_map: default(),
            render_meshes: vec![RenderMeshGroup {
                mesh,
                material: MeshMaterial::Color,
                local_bounds: [-3104.0, -3104.0, 3104.0, 3104.0],
            }],
            baked: BakedMovie {
                frame_rate: 30.0,
                skins: vec![],
                clips: vec![BakedClip {
                    name: "default".into(),
                    start_frame: 0,
                    events: vec![],
                    frames: vec![vec![BakedNode::Group {
                        children: vec![BakedNode::Shape {
                            id: 1,
                            ratio: 0,
                            transform: AnimTransform::default(),
                        }],
                        filters: vec![],
                        blend_mode: 2,
                    }]],
                }],
            },
        });
    app.world_mut()
        .spawn((VabAssetHandle(asset), VabPlayer::default()));
    let left = capture_camera(&mut app, output.clone(), size);
    assert!(left.chunks_exact(4).all(|p| p[0] > 250 && p[2] < 5));
    app.world_mut()
        .get_mut::<Transform>(camera)
        .unwrap()
        .translation
        .x = 1000.0;
    let right = capture_camera(&mut app, output, size);
    assert!(
        right.chunks_exact(4).all(|p| p[2] > 250 && p[0] < 5),
        "camera motion reused a stale cropped texture"
    );
    let diagnostics = app
        .sub_app(RenderApp)
        .world()
        .resource::<FlashRenderDiagnostics>();
    assert!(
        diagnostics.vab_filter_cache_bytes < 1024 * 1024,
        "root output was not cropped: {diagnostics:?}"
    );
    assert!(
        diagnostics.pooled_texture_bytes < 1024 * 1024,
        "oversized root entered the transient pool: {diagnostics:?}"
    );
}

#[test]
#[ignore = "requires GPU"]
fn vab_instance_cropped_root_keeps_offscreen_glow_input() {
    let mut app = app(PathBuf::from("assets"));
    app.world_mut().insert_resource(VabFilterMsaa::Off);
    let small = target(&mut app, UVec2::splat(64));
    let large = target(&mut app, UVec2::splat(128));
    for output in [&small, &large] {
        app.world_mut().spawn((
            Camera2d,
            Msaa::Off,
            Camera {
                clear_color: ClearColorConfig::Custom(Color::NONE),
                ..default()
            },
            RenderTarget::Image(output.clone().into()),
        ));
    }
    // The source is entirely outside the small camera, but its glow is visible.
    let mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![
            [35.0, -8.0, 0.0],
            [39.0, -8.0, 0.0],
            [35.0, 8.0, 0.0],
            [35.0, 8.0, 0.0],
            [39.0, -8.0, 0.0],
            [39.0, 8.0, 0.0],
        ],
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0; 4]; 6]);
    let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(mesh);
    let asset = app
        .world_mut()
        .resource_mut::<Assets<VabAsset>>()
        .add(VabAsset {
            shape_map: [(1, vec![0])].into(),
            morph_map: default(),
            render_meshes: vec![RenderMeshGroup {
                mesh,
                material: MeshMaterial::Color,
                local_bounds: [35.0, -8.0, 39.0, 8.0],
            }],
            baked: BakedMovie {
                frame_rate: 30.0,
                skins: vec![],
                clips: vec![BakedClip {
                    name: "default".into(),
                    start_frame: 0,
                    events: vec![],
                    frames: vec![vec![BakedNode::Group {
                        children: vec![BakedNode::Shape {
                            id: 1,
                            ratio: 0,
                            transform: AnimTransform::default(),
                        }],
                        filters: vec![AnimFilter::GlowFilter(AnimGlowFilter {
                            flags: 0x21,
                            color_r: 255,
                            color_g: 0,
                            color_b: 0,
                            color_a: 255,
                            blur_x: 20 * 65536,
                            blur_y: 20 * 65536,
                            strength: 256,
                            num_passes: 1,
                        })],
                        blend_mode: 0,
                    }]],
                }],
            },
        });
    app.world_mut()
        .spawn((VabAssetHandle(asset), VabPlayer::default()));
    let cropped = capture_camera(&mut app, small, UVec2::splat(64));
    let reference = capture_camera(&mut app, large, UVec2::splat(128));
    assert!(
        cropped.chunks_exact(4).any(|p| p[3] > 5),
        "offscreen source glow was culled"
    );
    for y in 0..64 {
        for x in 0..64 {
            for channel in 0..4 {
                let a = cropped[(y * 64 + x) * 4 + channel];
                let b = reference[((y + 32) * 128 + x + 32) * 4 + channel];
                assert!(
                    a.abs_diff(b) <= 2,
                    "cropped glow differs at {x},{y} channel {channel}: {a} vs {b}"
                );
            }
        }
    }
}

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
