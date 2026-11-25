//! GPU coverage for filters.
use super::*;

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
fn downsampled_blur_and_glow_preserve_source_and_cut_processed_pixels() {
    use bevy_flash::render::VabFilterQuality;

    let mut app = app(PathBuf::from("assets"));
    let size = UVec2::new(256, 128);
    let output = target(&mut app, size);
    app.world_mut().spawn((
        Camera2d,
        Camera {
            clear_color: ClearColorConfig::Custom(Color::NONE),
            ..default()
        },
        RenderTarget::Image(output.clone().into()),
    ));
    let quad = |x0: f32, y0: f32, x1: f32, y1: f32| {
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![
                [x0, y0, 0.0],
                [x1, y0, 0.0],
                [x0, y1, 0.0],
                [x0, y1, 0.0],
                [x1, y0, 0.0],
                [x1, y1, 0.0],
            ],
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0; 4]; 6])
    };
    let blur_mesh = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(quad(-64.0, -16.0, -32.0, 16.0));
    let glow_mesh = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(quad(32.0, -16.0, 64.0, 16.0));
    let asset = app
        .world_mut()
        .resource_mut::<Assets<VabAsset>>()
        .add(VabAsset {
            shape_map: [(1, vec![0]), (2, vec![1])].into(),
            morph_map: default(),
            render_meshes: vec![
                RenderMeshGroup {
                    mesh: blur_mesh,
                    material: MeshMaterial::Color,
                    local_bounds: [-64.0, -16.0, -32.0, 16.0],
                },
                RenderMeshGroup {
                    mesh: glow_mesh,
                    material: MeshMaterial::Color,
                    local_bounds: [32.0, -16.0, 64.0, 16.0],
                },
            ],
            baked: BakedMovie {
                frame_rate: 30.0,
                skins: vec![],
                clips: vec![BakedClip {
                    name: "default".into(),
                    start_frame: 0,
                    events: vec![],
                    frames: vec![vec![
                        BakedNode::Group {
                            children: vec![BakedNode::Shape {
                                id: 1,
                                ratio: 0,
                                transform: AnimTransform::default(),
                            }],
                            filters: vec![AnimFilter::BlurFilter(AnimBlurFilter {
                                blur_x: 13 * 65536,
                                blur_y: 13 * 65536,
                                num_passes: 2,
                            })],
                            blend_mode: 0,
                        },
                        BakedNode::Group {
                            children: vec![BakedNode::Shape {
                                id: 2,
                                ratio: 0,
                                transform: AnimTransform::default(),
                            }],
                            filters: vec![AnimFilter::GlowFilter(AnimGlowFilter {
                                flags: 0x22,
                                color_r: 255,
                                color_g: 0,
                                color_b: 0,
                                color_a: 255,
                                blur_x: 13 * 65536,
                                blur_y: 13 * 65536,
                                strength: 512,
                                num_passes: 2,
                            })],
                            blend_mode: 0,
                        },
                    ]],
                }],
            },
        });
    let mut player = VabPlayer::default();
    player.pause();
    app.world_mut().spawn((VabAssetHandle(asset), player));

    *app.world_mut().resource_mut::<VabFilterQuality>() = VabFilterQuality::full();
    let _full_pixels = capture_camera(&mut app, output.clone(), size);
    let saved_budget = app.world().resource::<VabFilterCacheSettings>().max_bytes;
    app.world_mut()
        .resource_mut::<VabFilterCacheSettings>()
        .max_bytes = 0;
    app.update();
    app.world_mut()
        .resource_mut::<VabFilterCacheSettings>()
        .max_bytes = saved_budget;
    *app.world_mut().resource_mut::<VabFilterQuality>() = VabFilterQuality::full();
    app.update();
    let (full_processed, full_cache_bytes) = {
        let diag = app
            .sub_app(RenderApp)
            .world()
            .resource::<FlashRenderDiagnostics>();
        (
            diag.vab_filter_processed_pixels,
            diag.vab_filter_cache_bytes,
        )
    };
    app.world_mut()
        .resource_mut::<VabFilterCacheSettings>()
        .max_bytes = 0;
    app.update();
    app.world_mut()
        .resource_mut::<VabFilterCacheSettings>()
        .max_bytes = saved_budget;
    *app.world_mut().resource_mut::<VabFilterQuality>() = VabFilterQuality::half_blur();
    app.update();
    let (half_processed, half_cache_bytes) = {
        let diag = app
            .sub_app(RenderApp)
            .world()
            .resource::<FlashRenderDiagnostics>();
        (
            diag.vab_filter_processed_pixels,
            diag.vab_filter_cache_bytes,
        )
    };
    let half_pixels = capture_camera(&mut app, output.clone(), size);
    assert!(
        full_processed > 0 && half_processed > 0,
        "expected non-zero filter work: full={full_processed}, half={half_processed}"
    );
    assert!(
        half_processed * 2 < full_processed,
        "half_blur should cut processed pixels by >50%: full={full_processed}, half={half_processed}"
    );
    assert!(
        half_cache_bytes < full_cache_bytes,
        "half_blur should shrink cached pure-Blur VRAM: full={full_cache_bytes}, half={half_cache_bytes}"
    );

    let pixel = |x: usize, y: usize| {
        let o = (y * 256 + x) * 4;
        <[u8; 4]>::try_from(&half_pixels[o..o + 4]).unwrap()
    };
    // Blur square at local x=[-64..-32] -> screen x=[64..96], y=[48..80].
    assert!(pixel(80, 64)[3] > 200, "downsampled blur lost center");
    assert!(pixel(59, 64)[3] > 0, "downsampled blur lost outer halo");
    // Glow square at local x=[32..64] -> screen x=[160..192], y=[48..80].
    let glow_center = pixel(176, 64);
    assert!(
        glow_center[0] > 245
            && glow_center[1] > 245
            && glow_center[2] > 245
            && glow_center[3] > 245,
        "glow foreground must remain crisp white: {glow_center:?}"
    );
    let glow_halo = pixel(196, 64);
    assert!(
        glow_halo[0] > 100 && glow_halo[1] < 40 && glow_halo[3] > 50,
        "glow outer red halo missing: {glow_halo:?}"
    );
}

#[test]
#[ignore = "requires GPU"]
fn complex_blend_modes_inside_isolated_layer_match_flash_equations() {
    use bevy::mesh::VertexAttributeValues;
    use bevy_flash::vab_asset::{ATTRIBUTE_VAB_COLOR_SRGB, ATTRIBUTE_VAB_POSITION};
    use vatf::animation::AnimColorTransform;

    let mut app = app(PathBuf::from("assets"));
    let size = UVec2::new(256, 64);
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

    let quad = |x0: f32, y0: f32, x1: f32, y1: f32, rgba: [u8; 4]| {
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(
            ATTRIBUTE_VAB_POSITION,
            vec![[x0, y0], [x1, y0], [x0, y1], [x0, y1], [x1, y0], [x1, y1]],
        )
        .with_inserted_attribute(
            ATTRIBUTE_VAB_COLOR_SRGB,
            VertexAttributeValues::Unorm8x4(vec![rgba; 6]),
        )
    };
    let dst_mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(quad(
        -10.0,
        -10.0,
        10.0,
        10.0,
        [204, 153, 51, 255],
    ));
    let src_mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(quad(
        -10.0,
        -10.0,
        10.0,
        10.0,
        [77, 204, 179, 255],
    ));
    // Covers only the right half so Alpha verifies both partial-alpha and outside-zeroing.
    let half_alpha_mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(quad(
        0.0,
        -10.0,
        10.0,
        10.0,
        [255, 255, 255, 128],
    ));

    let translate = |x: f32| AnimTransform {
        matrix: AnimMatrix {
            tx: x,
            ..AnimMatrix::IDENTITY
        },
        color_transform: AnimColorTransform::IDENTITY,
    };

    // Each mode sits inside an isolated parent Layer (blend_mode: 2) so complex
    // destination-sampling shaders run against the parent TextureTarget.
    let specs: [(f32, u8, u16); 9] = [
        (-112.0, 3, 2), // Multiply
        (-84.0, 5, 2),  // Lighten (exact)
        (-56.0, 6, 2),  // Darken
        (-28.0, 7, 2),  // Difference
        (0.0, 10, 2),   // Invert
        (28.0, 11, 3),  // Alpha
        (56.0, 12, 3),  // Erase
        (84.0, 13, 2),  // Overlay
        (112.0, 14, 2), // HardLight
    ];
    let nodes: Vec<BakedNode> = specs
        .iter()
        .map(|&(x, blend_mode, src_id)| BakedNode::Group {
            blend_mode: 2,
            filters: vec![],
            children: vec![
                BakedNode::Shape {
                    id: 1,
                    ratio: 0,
                    transform: translate(x),
                },
                BakedNode::Group {
                    blend_mode,
                    filters: vec![],
                    children: vec![BakedNode::Shape {
                        id: src_id,
                        ratio: 0,
                        transform: translate(x),
                    }],
                },
            ],
        })
        .collect();

    let asset = app
        .world_mut()
        .resource_mut::<Assets<VabAsset>>()
        .add(VabAsset {
            shape_map: [(1, vec![0]), (2, vec![1]), (3, vec![2])].into(),
            morph_map: default(),
            render_meshes: vec![
                RenderMeshGroup {
                    mesh: dst_mesh,
                    material: MeshMaterial::Color,
                    local_bounds: [-10.0, -10.0, 10.0, 10.0],
                },
                RenderMeshGroup {
                    mesh: src_mesh,
                    material: MeshMaterial::Color,
                    local_bounds: [-10.0, -10.0, 10.0, 10.0],
                },
                RenderMeshGroup {
                    mesh: half_alpha_mesh,
                    material: MeshMaterial::Color,
                    local_bounds: [0.0, -10.0, 10.0, 10.0],
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

    let pixels = capture_camera(&mut app, output, size);
    let at = |local_x: i32| {
        let sx = (128 + local_x) as usize;
        let o = (32 * 256 + sx) * 4;
        <[u8; 4]>::try_from(&pixels[o..o + 4]).unwrap()
    };
    let assert_near = |label: &str, got: [u8; 4], expected: [u8; 4]| {
        for c in 0..4 {
            assert!(
                got[c].abs_diff(expected[c]) <= 4,
                "{label} channel {c}: got {got:?}, expected {expected:?}"
            );
        }
    };

    // dst = (204, 153, 51, 255) = (0.8, 0.6, 0.2)
    // src = (77, 204, 179, 255) = (0.302, 0.8, 0.702)
    assert_near("Multiply", at(-112), [62, 122, 36, 255]);
    assert_near("Lighten", at(-84), [204, 204, 179, 255]);
    assert_near("Darken", at(-56), [77, 153, 51, 255]);
    assert_near("Difference", at(-28), [127, 51, 128, 255]);
    assert_near("Invert", at(0), [51, 102, 204, 255]);
    // Alpha: left half outside half_alpha_mesh is cleared to 0; right half is dst * 0.502.
    assert_near("Alpha outside", at(24), [0, 0, 0, 0]);
    assert_near("Alpha inside", at(32), [102, 77, 26, 128]);
    // Erase: left half stays full dst; right half is dst * (1 - 0.502).
    assert_near("Erase outside", at(52), [204, 153, 51, 255]);
    assert_near("Erase inside", at(60), [102, 76, 25, 127]);
    // Overlay vs HardLight differ strongly on R (184 vs 123) and B (72 vs 133).
    assert_near("Overlay", at(84), [184, 214, 72, 255]);
    assert_near("HardLight", at(112), [123, 214, 133, 255]);

    let diag = app
        .sub_app(RenderApp)
        .world()
        .resource::<FlashRenderDiagnostics>();
    assert_eq!(
        diag.unsupported_vab_layers, 0,
        "all complex blends nested in isolated layers must be supported"
    );
}

#[test]
#[ignore = "requires GPU"]
fn filter_symbol_stamps_batch_and_match_direct_rendering_visually() {
    use bevy::mesh::VertexAttributeValues;
    use bevy_flash::render::VabFilterQuality;
    use bevy_flash::vab_asset::{ATTRIBUTE_VAB_COLOR_SRGB, ATTRIBUTE_VAB_POSITION};
    use vatf::animation::AnimColorTransform;

    let mut app = app(PathBuf::from("assets"));
    let size = UVec2::new(256, 200);
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

    let quad = |x0: f32, y0: f32, x1: f32, y1: f32, rgba: [u8; 4]| {
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(
            ATTRIBUTE_VAB_POSITION,
            vec![[x0, y0], [x1, y0], [x0, y1], [x0, y1], [x1, y0], [x1, y1]],
        )
        .with_inserted_attribute(
            ATTRIBUTE_VAB_COLOR_SRGB,
            VertexAttributeValues::Unorm8x4(vec![rgba; 6]),
        )
    };
    let base_mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(quad(
        -10.0,
        -10.0,
        10.0,
        10.0,
        [255, 0, 0, 255],
    ));
    let accent_mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(quad(
        4.0,
        -6.0,
        10.0,
        6.0,
        [255, 255, 0, 255],
    ));

    let glow = vec![AnimFilter::GlowFilter(AnimGlowFilter {
        flags: 0x22,
        color_r: 0,
        color_g: 0,
        color_b: 255,
        color_a: 255,
        blur_x: 9 * 65536,
        blur_y: 9 * 65536,
        strength: 512,
        num_passes: 2,
    })];

    let place_symbol = |tx: f32, ty: f32, angle_deg: f32, scale: f32| {
        let (sin, cos) = angle_deg.to_radians().sin_cos();
        let matrix = AnimMatrix {
            a: cos * scale,
            b: sin * scale,
            c: -sin * scale,
            d: cos * scale,
            tx,
            ty,
        };
        let transform = AnimTransform {
            matrix,
            color_transform: AnimColorTransform::IDENTITY,
        };
        BakedNode::Group {
            blend_mode: 0,
            filters: glow.clone(),
            children: vec![
                BakedNode::Shape {
                    id: 1,
                    ratio: 0,
                    transform,
                },
                BakedNode::Shape {
                    id: 2,
                    ratio: 0,
                    transform,
                },
            ],
        }
    };

    // Frame 0: 16 copies of the 2-shape glowing symbol at distinct positions & rotations.
    let mut frame0 = Vec::new();
    let mut frame1 = Vec::new();
    let xs = [-90.0, -30.0, 30.0, 90.0];
    let ys = [-54.0, -18.0, 18.0, 54.0];
    let mut idx = 0usize;
    for &y in &ys {
        for &x in &xs {
            let angle = (idx % 4) as f32 * 90.0;
            frame0.push(place_symbol(x, y, angle, 1.0));
            frame1.push(place_symbol(x + 6.0, y + 4.0, angle + 30.0, 1.0));
            idx += 1;
        }
    }
    // Frame 2: two differently scaled copies (1.0x and 0.75x) to test opt-in scaled stamp reuse.
    let frame2 = vec![
        place_symbol(-40.0, 0.0, 0.0, 1.0),
        place_symbol(40.0, 0.0, 45.0, 0.75),
    ];

    let asset = app
        .world_mut()
        .resource_mut::<Assets<VabAsset>>()
        .add(VabAsset {
            shape_map: [(1, vec![0]), (2, vec![1])].into(),
            morph_map: default(),
            render_meshes: vec![
                RenderMeshGroup {
                    mesh: base_mesh,
                    material: MeshMaterial::Color,
                    local_bounds: [-10.0, -10.0, 10.0, 10.0],
                },
                RenderMeshGroup {
                    mesh: accent_mesh,
                    material: MeshMaterial::Color,
                    local_bounds: [4.0, -6.0, 10.0, 6.0],
                },
            ],
            baked: BakedMovie {
                frame_rate: 30.0,
                skins: vec![],
                clips: vec![BakedClip {
                    name: "default".into(),
                    start_frame: 0,
                    events: vec![],
                    frames: vec![frame0, frame1, frame2],
                }],
            },
        });
    let mut player = VabPlayer::default();
    player.pause();
    let entity = app.world_mut().spawn((VabAssetHandle(asset), player)).id();

    // Render Frame 0 and inspect pixel accuracy + batching.
    let pixels0 = capture_camera(&mut app, output.clone(), size);
    let diag0 = app
        .sub_app(RenderApp)
        .world()
        .resource::<FlashRenderDiagnostics>();
    assert_eq!(
        diag0.vab_draw_packets, 1,
        "all 16 identical rotated glowing symbols should batch into 1 instanced quad packet: {diag0:?}"
    );
    assert_eq!(diag0.vab_filter_layers, 16);
    assert_eq!(diag0.vab_filter_cache_entries, 1);

    let at = |pixels: &[u8], x: usize, y: usize| {
        let o = (y * 256 + x) * 4;
        <[u8; 4]>::try_from(&pixels[o..o + 4]).unwrap()
    };
    // Symbol 0 at (-90, -54) -> screen (38, 46), angle 0°: accent bar on right (45, 46).
    let s0_body = at(&pixels0, 38, 46);
    let s0_accent = at(&pixels0, 45, 46);
    let s0_halo = at(&pixels0, 25, 46);
    assert!(
        s0_body[0] > 240 && s0_body[1] < 20 && s0_body[2] < 20,
        "s0 body={s0_body:?}"
    );
    assert!(
        s0_accent[0] > 240 && s0_accent[1] > 240 && s0_accent[2] < 20,
        "s0 accent={s0_accent:?}"
    );
    assert!(
        s0_halo[2] > 60 && s0_halo[0] < 40 && s0_halo[3] > 40,
        "s0 halo={s0_halo:?}"
    );

    // Symbol 1 at (-30, -54) -> screen (98, 46), angle 90° CW: accent bar rotates to bottom (98, 53).
    let s1_body = at(&pixels0, 98, 46);
    let s1_right = at(&pixels0, 105, 46);
    let s1_accent = at(&pixels0, 98, 53);
    let s1_halo = at(&pixels0, 98, 33);
    assert!(s1_body[0] > 240 && s1_body[1] < 20, "s1 body={s1_body:?}");
    assert!(
        s1_right[0] > 240 && s1_right[1] < 20,
        "s1 right should be red after 90° rotation: {s1_right:?}"
    );
    assert!(
        s1_accent[0] > 240 && s1_accent[1] > 240 && s1_accent[2] < 20,
        "s1 accent={s1_accent:?}"
    );
    assert!(
        s1_halo[2] > 60 && s1_halo[0] < 40 && s1_halo[3] > 40,
        "s1 halo={s1_halo:?}"
    );

    // Switch to Frame 1: all 16 symbols move and rotate by +30° -> 100% stamp cache hit, 0 GPU filter passes!
    app.world_mut()
        .get_mut::<VabPlayer>(entity)
        .unwrap()
        .current_frame = 1;
    app.update();
    let diag1 = app
        .sub_app(RenderApp)
        .world()
        .resource::<FlashRenderDiagnostics>();
    assert_eq!(
        diag1.vab_filter_cache_misses, 0,
        "moving+rotating symbols must hit stamp cache: {diag1:?}"
    );
    assert_eq!(diag1.vab_filter_cache_hits, 16);
    assert_eq!(diag1.vab_filter_passes, 0);
    assert_eq!(diag1.vab_draw_packets, 1);

    // Switch to Frame 2 (1.0x + 0.75x) with opt-in scaled stamps -> shares 1 octave stamp & 1 packet!
    *app.world_mut().resource_mut::<VabFilterQuality>() =
        VabFilterQuality::full().with_scaled_stamps(true);
    app.world_mut()
        .get_mut::<VabPlayer>(entity)
        .unwrap()
        .current_frame = 2;
    let pixels2 = capture_camera(&mut app, output, size);
    let diag2 = app
        .sub_app(RenderApp)
        .world()
        .resource::<FlashRenderDiagnostics>();
    assert_eq!(
        diag2.vab_draw_packets, 1,
        "1.0x and 0.75x glowing symbols should share 1 octave stamp when reuse_scaled_stamps is enabled: {diag2:?}"
    );
    assert_eq!(diag2.vab_filter_layers, 2);
    // Both symbols rendered cleanly at (-40, 0) -> (88, 100) and (40, 0) -> (168, 100).
    assert!(at(&pixels2, 88, 100)[0] > 240);
    assert!(at(&pixels2, 168, 100)[0] > 240);
}
