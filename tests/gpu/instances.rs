//! GPU coverage for instances.
use super::*;

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
