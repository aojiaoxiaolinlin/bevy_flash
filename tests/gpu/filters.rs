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
