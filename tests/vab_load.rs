//! Loader integration test: compile a source SWF into a VAB, load it through
//! Bevy's `AssetServer` headlessly, and assert the resulting asset is
//! structurally complete and reproducible frame by frame.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use bevy::MinimalPlugins;
use bevy::app::App;
use bevy::asset::{AssetApp, AssetPlugin, AssetServer, Assets, Handle, LoadState};
use bevy::image::Image;
use bevy::mesh::Mesh;
use bevy_flash::vab_asset::{CommandList, VabAsset, VabCommand};
use vatf::baked::BakedNode;
use vatf::reader::VabReader;

#[test]
fn native_login_button_loads_named_states_with_shared_registration() {
    use bevy_flash::{
        vab_button::VabButton,
        vab_graphic::{VabAssetLabel, VabGraphic},
    };
    let directory = asset_directory().join("buttons");
    std::fs::create_dir_all(&directory).unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/login.swf");
    vatf::convert_swf_ui_to_vab(&source, &directory.join("login.vab")).unwrap();
    let mut app = build_app(&directory);
    let handle: Handle<VabButton> = app
        .world()
        .resource::<AssetServer>()
        .load(VabAssetLabel::Button("login_button").from_asset("login.vab"));
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        app.update();
        let server = app.world().resource::<AssetServer>();
        if let Some(LoadState::Failed(error)) = server.get_load_state(&handle) {
            panic!("{error}");
        }
        if server.is_loaded_with_dependencies(&handle) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "button dependencies failed to load"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let buttons = app.world().resource::<Assets<VabButton>>();
    let button = buttons.get(&handle).unwrap();
    assert_ne!(button.up, button.over);
    assert_ne!(button.down, *button.hit_test.as_ref().unwrap());
    let graphics = app.world().resource::<Assets<VabGraphic>>();
    let render_assets = app.world().resource::<Assets<VabAsset>>();
    let up = graphics.get(&button.up).unwrap();
    for state in [&button.up, &button.over, &button.down] {
        let graphic = graphics.get(state).unwrap();
        assert_eq!(graphic.frame_count, 1);
        assert_eq!(graphic.source_bounds, up.source_bounds);
        assert_eq!(graphic.visual_bounds, up.visual_bounds);
        let render = render_assets.get(&graphic.render_asset).unwrap();
        assert!(
            count_draws(
                &render
                    .sample(0, 0, &Default::default(), bevy::math::Vec3::ONE)
                    .unwrap()
            ) > 0
        );
        assert_eq!(
            render.render_meshes[0].mesh,
            render_assets.get(&up.render_asset).unwrap().render_meshes[0].mesh
        );
    }
}

#[test]
fn named_ui_graphics_load_without_root_placement_and_share_meshes() {
    use bevy_flash::vab_graphic::{VabAssetLabel, VabGraphic};
    let directory = asset_directory();
    std::fs::create_dir_all(&directory).unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/ui_demo.swf");
    vatf::convert_swf_ui_to_vab(&source, &directory.join("ui.vab")).unwrap();
    let mut app = build_app(&directory);
    let server = app.world().resource::<AssetServer>();
    let button: Handle<VabGraphic> =
        server.load(VabAssetLabel::Graphic("button_background").from_asset("ui.vab"));
    // This public export name previously collided with an internal mesh label.
    let shape: Handle<VabGraphic> = server.load("ui.vab#mesh_0");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        app.update();
        let server = app.world().resource::<AssetServer>();
        for handle in [&button, &shape] {
            if let Some(LoadState::Failed(error)) = server.get_load_state(handle) {
                panic!("{error}");
            }
        }
        if server.is_loaded_with_dependencies(&button) && server.is_loaded_with_dependencies(&shape)
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "UI sub-assets failed to finish loading"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let graphics = app.world().resource::<Assets<VabGraphic>>();
    let button = graphics.get(&button).unwrap();
    let shape = graphics.get(&shape).unwrap();
    assert_eq!(button.size(), bevy::math::Vec2::new(100.0, 40.0));
    let render_assets = app.world().resource::<Assets<VabAsset>>();
    let button = render_assets.get(&button.render_asset).unwrap();
    let shape = render_assets.get(&shape.render_asset).unwrap();
    assert_eq!(button.render_meshes[0].mesh, shape.render_meshes[0].mesh);
    assert_eq!(button.root_frame_count(), 1);
    let BakedNode::Shape { transform, .. } = &button.baked.clips[0].frames[0][0] else {
        panic!("expected centered shape");
    };
    assert_eq!((transform.matrix.tx, transform.matrix.ty), (-60.0, -40.0));
}

/// Collect every shape id referenced anywhere in a baked node tree.
fn collect_shape_ids(nodes: &[BakedNode], out: &mut Vec<u16>) {
    for node in nodes {
        match node {
            BakedNode::Shape { id, .. } => out.push(*id),
            BakedNode::Group { children, .. } => collect_shape_ids(children, out),
            BakedNode::Mask { mask, children } => {
                collect_shape_ids(mask, out);
                collect_shape_ids(children, out);
            }
            // Skin variants live in `movie.skins`; `sample` resolves them there.
            BakedNode::Skin { .. } => {}
        }
    }
}

/// Directory holding the freshly compiled test asset.
fn asset_directory() -> PathBuf {
    std::env::temp_dir().join(format!("bevy_flash_load_{}", std::process::id()))
}

fn fixture_swf() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../bevy_flash/assets/spirit2159src.swf")
}

fn static_text_fixture_swf() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../bevy_flash/assets/123620.swf")
}

/// Compiles the fixture into the asset directory and returns the VAB file name.
fn prepare_asset() -> Option<String> {
    let swf_path = fixture_swf();
    if !swf_path.exists() {
        eprintln!("skipping: fixture not found at {}", swf_path.display());
        return None;
    }

    let directory = asset_directory();
    std::fs::create_dir_all(&directory).unwrap();

    let file_name = "spirit2159src.vab";
    vatf::convert_swf_to_vab(&swf_path, &directory.join(file_name)).unwrap();
    Some(file_name.to_owned())
}

#[test]
fn static_text_fixture_bakes_glyphs_and_loads_headlessly() {
    let swf_path = static_text_fixture_swf();
    if !swf_path.exists() {
        eprintln!("skipping: fixture not found at {}", swf_path.display());
        return;
    }
    let directory = asset_directory();
    std::fs::create_dir_all(&directory).unwrap();
    let file_name = "123620.vab";
    vatf::convert_swf_to_vab(&swf_path, &directory.join(file_name)).unwrap();

    let mut app = build_app(&directory);
    let handle = app
        .world()
        .resource::<AssetServer>()
        .load::<VabAsset>(file_name);
    wait_for_load(&mut app, &handle);

    let assets = app.world().resource::<Assets<VabAsset>>();
    let asset = assets.get(&handle).expect("asset missing after load");
    let clip_names: Vec<_> = asset
        .baked
        .clips
        .iter()
        .map(|clip| clip.name.as_str())
        .collect();
    assert_eq!(
        clip_names,
        [
            "IDLE",
            "STB",
            "BTS",
            "APPEAR",
            "ATTACK",
            "UNDER_ATTACK",
            "BEAT_DOWN",
            "MISS",
            "MAGIC_START",
            "MAGIC_FOCUS",
            "MAGIC_END",
            "DEAD",
            "BLANK",
        ]
    );
    assert!(
        asset.shape_map.keys().any(|id| *id > 336),
        "DefineText glyph should become a synthetic shape"
    );
    let mut referenced = Vec::new();
    for clip in &asset.baked.clips {
        for frame in &clip.frames {
            collect_shape_ids(frame, &mut referenced);
        }
    }
    assert!(
        referenced.iter().any(|id| *id > 336),
        "the baked tree should reference the synthetic text glyph"
    );
    let mut total_draws = 0;
    for (clip, animation) in asset.baked.clips.iter().enumerate() {
        for frame in 0..animation.frames.len() {
            let commands = asset
                .sample(clip, frame, &Default::default(), bevy::math::Vec3::ONE)
                .unwrap();
            total_draws += count_draws(&commands);
        }
    }
    assert!(
        total_draws > 0,
        "fixture should produce renderable commands"
    );
}

fn build_app(asset_directory: &Path) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(AssetPlugin {
            file_path: asset_directory.to_string_lossy().into_owned(),
            ..Default::default()
        })
        .add_plugins(bevy_flash::FlashPlayerPlugin)
        // Types the loader registers as labelled sub-assets.
        .init_asset::<Image>()
        .init_asset::<Mesh>();
    app
}

/// Drives the app until `handle` finishes loading (or panics on failure).
fn wait_for_load(app: &mut App, handle: &Handle<VabAsset>) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        app.update();

        match app.world().resource::<AssetServer>().get_load_state(handle) {
            Some(LoadState::Loaded) => return,
            Some(LoadState::Failed(error)) => panic!("asset failed to load: {error}"),
            _ => {}
        }

        assert!(
            Instant::now() < deadline,
            "timed out waiting for the asset to load",
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn count_draws(commands: &CommandList) -> usize {
    commands
        .commands
        .iter()
        .map(|command| match command {
            VabCommand::RenderShape { .. } => 1,
            VabCommand::ApplyFilter { commands, .. } | VabCommand::Blend(commands, _) => {
                count_draws(commands)
            }
            _ => 0,
        })
        .sum()
}

#[test]
fn asset_server_loads_generated_vab_headlessly() {
    let Some(file_name) = prepare_asset() else {
        return;
    };
    let directory = asset_directory();

    let mut app = build_app(&directory);
    let handle = app
        .world()
        .resource::<AssetServer>()
        .load::<VabAsset>(file_name.clone());
    wait_for_load(&mut app, &handle);

    let assets = app.world().resource::<Assets<VabAsset>>();
    let asset = assets.get(&handle).expect("asset missing after load");

    // Cross-check the loader's output against the file it read.
    let reader = VabReader::open(directory.join(&file_name)).unwrap();
    let shape_meshes = reader.shape_meshes().unwrap();
    let shape_records = reader.shape_records().unwrap();

    assert_eq!(
        asset.render_meshes.len(),
        shape_meshes.len(),
        "one RenderMeshGroup per ShapeMesh",
    );
    assert_eq!(
        asset.shape_map.len(),
        shape_records.len(),
        "one shape_map entry per ShapeRecord",
    );
    assert!(asset.baked.frame_rate > 0.0, "frame rate must be baked");
    assert!(!asset.baked.clips.is_empty());
    for (clip, animation) in asset.baked.clips.iter().enumerate() {
        for frame in 0..animation.frames.len() {
            asset
                .sample(clip, frame, &Default::default(), bevy::math::Vec3::ONE)
                .unwrap();
        }
    }

    // The loader must preserve the file's clip layout exactly.
    let root_frames = asset.root_frame_count();
    assert_eq!(
        root_frames,
        reader
            .baked()
            .clips
            .iter()
            .map(|clip| clip.start_frame as usize + clip.frames.len())
            .max()
            .unwrap_or(0),
        "root timeline frame count must survive the load",
    );
    assert!(root_frames > 1, "fixture should have multiple frames");

    for handles in asset.shape_map.values() {
        for handle in handles {
            assert!(
                *handle < asset.render_meshes.len(),
                "shape handle {handle} out of range",
            );
        }
    }
    for handles in asset.morph_map.values() {
        for handle in handles {
            assert!(
                *handle < asset.render_meshes.len(),
                "morph handle {handle} out of range",
            );
        }
    }

    // Every shape referenced by the baked tree must resolve to something
    // renderable. An id that resolves to nothing means the loader dropped it.
    let mut referenced = Vec::new();
    for clip in &asset.baked.clips {
        for frame in &clip.frames {
            collect_shape_ids(frame, &mut referenced);
        }
    }
    assert!(!referenced.is_empty(), "fixture should reference shapes");
    for id in referenced {
        let resolves = asset.shape_map.contains_key(&id)
            || asset.morph_map.keys().any(|(morph_id, _)| *morph_id == id);
        assert!(
            resolves,
            "shape id {id} in the baked tree resolves to nothing"
        );
    }

    // Every frame must produce draws. Objects persist across frames, so the
    // timeline draws far more than one shape per frame; under the old
    // per-frame-delta compiler bug almost every frame came back empty.
    let scale = bevy::math::Vec3::splat(1.0);
    let mut total_draws = 0;
    for frame_index in 0..root_frames {
        total_draws += count_draws(&asset.build_frame_commands(scale, frame_index));
    }
    assert!(
        total_draws > root_frames,
        "objects must persist across frames ({total_draws} draws over {root_frames} frames)",
    );
}

#[test]
fn loader_reports_corrupt_vab() {
    let directory = asset_directory();
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("corrupt.vab"), b"NOT A VAB FILE").unwrap();

    let mut app = build_app(&directory);
    let handle = app
        .world()
        .resource::<AssetServer>()
        .load::<VabAsset>("corrupt.vab");

    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        app.update();
        match app
            .world()
            .resource::<AssetServer>()
            .get_load_state(&handle)
        {
            Some(LoadState::Failed(_)) => return,
            Some(LoadState::Loaded) => panic!("corrupt file must not load successfully"),
            _ => {}
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the failure"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
