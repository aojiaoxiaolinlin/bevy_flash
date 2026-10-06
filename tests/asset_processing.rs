//! Real Bevy processing, cache reuse and output-loader/sub-asset integration.
#![cfg(feature = "asset_processor")]
use bevy::{
    asset::{
        AssetApp, AssetMode, AssetPlugin, AssetServer, Assets, Handle, LoadState,
        meta::{AssetAction, AssetMeta, AssetMetaDyn},
        processor::{AssetProcessor, FileTransactionLogFactory, ProcessorState},
    },
    image::Image,
    mesh::Mesh,
    prelude::*,
    tasks::block_on,
};
use bevy_flash::{
    FlashPlayerPlugin,
    asset_processing::{
        SwfCompileMode, SwfCompileSettings, SwfToVabProcessor, VabAssetProcessorPlugin,
        vab_processed_asset_path,
    },
    vab_asset::VabAsset,
    vab_button::VabButton,
    vab_graphic::VabGraphic,
};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

fn directory(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vab_processing_{}_{name}", std::process::id()));
    fs::create_dir_all(dir.join("source")).unwrap();
    dir
}

fn metadata(path: &Path, mode: SwfCompileMode) {
    let meta = AssetMeta::<(), SwfToVabProcessor>::new(AssetAction::Process {
        processor: <SwfToVabProcessor as bevy::reflect::TypePath>::type_path().into(),
        settings: SwfCompileSettings { mode },
    });
    fs::write(path, meta.serialize()).unwrap();
}

fn app(source: &Path, output: &Path, log: &Path, compile: bool) -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin {
            mode: AssetMode::Processed,
            file_path: source.to_string_lossy().into_owned(),
            processed_file_path: output.to_string_lossy().into_owned(),
            watch_for_changes_override: Some(false),
            use_asset_processor_override: Some(compile),
            ..default()
        },
        FlashPlayerPlugin,
        VabAssetProcessorPlugin,
    ))
    .init_asset::<Image>()
    .init_asset::<Mesh>();
    if compile {
        app.world()
            .resource::<AssetProcessor>()
            .data()
            .set_log_factory(Box::new(FileTransactionLogFactory {
                file_path: log.to_owned(),
            }))
            .unwrap();
    }
    app
}

fn load<A: bevy::asset::Asset>(app: &mut App, path: &str) -> Handle<A> {
    let handle = app
        .world()
        .resource::<AssetServer>()
        .load::<A>(path.to_owned());
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        app.update();
        let server = app.world().resource::<AssetServer>();
        if let Some(LoadState::Failed(error)) = server.get_load_state(&handle) {
            panic!("{path}: {error}");
        }
        if server.is_loaded_with_dependencies(&handle) {
            return handle;
        }
        assert!(Instant::now() < deadline, "timed out loading {path}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn finished(app: &mut App) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        app.update();
        if block_on(app.world().resource::<AssetProcessor>().get_state())
            == ProcessorState::Finished
        {
            return;
        }
        assert!(Instant::now() < deadline, "processing did not finish");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn processed_ui_cache_reuses_and_invalidates_source_and_settings() {
    let root = directory("cache");
    let source = root.join("source");
    let output = vab_processed_asset_path(root.join("cache"));
    let bytes = fs::read("assets/ui_demo.swf").unwrap();
    let input = source.join("ui.swf");
    fs::write(&input, &bytes).unwrap();
    metadata(&source.join("ui.swf.meta"), SwfCompileMode::StaticUi);
    let mut first = app(&source, &output, &root.join("first.log"), true);
    let graphic = load::<VabGraphic>(&mut first, "ui.swf#button_background");
    assert_eq!(
        first
            .world()
            .resource::<Assets<VabGraphic>>()
            .get(&graphic)
            .unwrap()
            .frame_count,
        1
    );
    finished(&mut first);
    let cached = output.join("ui.swf");
    let timestamp = fs::metadata(&cached).unwrap().modified().unwrap();
    let meta = fs::read(output.join("ui.swf.meta")).unwrap();
    assert_eq!(
        fs::read(&cached).unwrap(),
        vatf::compile_swf(
            &bytes,
            &SwfCompileSettings {
                mode: SwfCompileMode::StaticUi
            }
        )
        .unwrap()
        .bytes
    );
    drop(first);

    let mut second = app(&source, &output, &root.join("second.log"), true);
    load::<VabGraphic>(&mut second, "ui.swf#button_background");
    finished(&mut second);
    assert_eq!(
        fs::metadata(&cached).unwrap().modified().unwrap(),
        timestamp,
        "unchanged source must not rewrite output"
    );
    assert_eq!(fs::read(output.join("ui.swf.meta")).unwrap(), meta);
    drop(second);

    // A legal SWF version change alters its hash even when visual output is identical.
    let mut changed = bytes;
    changed[3] += 1;
    fs::write(&input, &changed).unwrap();
    let mut third = app(&source, &output, &root.join("third.log"), true);
    load::<VabGraphic>(&mut third, "ui.swf#button_background");
    finished(&mut third);
    let changed_meta = fs::read(output.join("ui.swf.meta")).unwrap();
    assert_ne!(
        changed_meta, meta,
        "source changes must invalidate the processing hash"
    );
    drop(third);

    metadata(&source.join("ui.swf.meta"), SwfCompileMode::Animation);
    let mut fourth = app(&source, &output, &root.join("fourth.log"), true);
    let animation = load::<VabAsset>(&mut fourth, "ui.swf");
    assert!(
        fourth
            .world()
            .resource::<Assets<VabAsset>>()
            .get(&animation)
            .unwrap()
            .baked
            .clips
            .iter()
            .all(|clip| clip.frames.iter().all(Vec::is_empty))
    );
    finished(&mut fourth);
    assert_ne!(fs::read(output.join("ui.swf.meta")).unwrap(), changed_meta);
    assert!(
        !vatf::reader::VabReader::open(&cached)
            .unwrap()
            .has_chunk(b"UIGR")
    );
    drop(fourth);

    // A published app needs only processed bytes + generated load metadata.
    let mut shipped = app(
        &root.join("absent_source"),
        &output,
        &root.join("unused.log"),
        false,
    );
    assert!(!shipped.world().contains_resource::<AssetProcessor>());
    load::<VabAsset>(&mut shipped, "ui.swf");
}

#[test]
fn animation_dynamic_ui_and_button_use_the_same_output_loader() {
    let root = directory("modes");
    let source = root.join("source");
    let output = vab_processed_asset_path(root.join("cache"));
    for (name, mode) in [
        ("animated_ui", SwfCompileMode::AnimatedUi),
        ("login", SwfCompileMode::StaticUi),
    ] {
        fs::copy(
            format!("assets/{name}.swf"),
            source.join(format!("{name}.swf")),
        )
        .unwrap();
        metadata(&source.join(format!("{name}.swf.meta")), mode);
    }
    // No metadata: the default processor treats SWF as an ordinary animation.
    fs::copy(
        "../bevy_flash/assets/spirit2159src.swf",
        source.join("spirit.swf"),
    )
    .unwrap();
    let mut app = app(&source, &output, &root.join("log"), true);
    let ui = load::<VabGraphic>(&mut app, "animated_ui.swf#sparkles");
    assert!(
        app.world()
            .resource::<Assets<VabGraphic>>()
            .get(&ui)
            .unwrap()
            .frame_count
            > 1
    );
    let button = load::<VabButton>(&mut app, "login.swf#login_button");
    let buttons = app.world().resource::<Assets<VabButton>>();
    let button = buttons.get(&button).unwrap();
    assert_ne!(button.up, button.down);
    let animation = load::<VabAsset>(&mut app, "spirit.swf");
    let animations = app.world().resource::<Assets<VabAsset>>();
    assert!(!animations.get(&animation).unwrap().baked.clips.is_empty());
    finished(&mut app);
    drop(app);
    let mut shipped = self::app(
        &root.join("absent_source"),
        &output,
        &root.join("unused.log"),
        false,
    );
    load::<VabGraphic>(&mut shipped, "animated_ui.swf#sparkles");
    load::<VabButton>(&mut shipped, "login.swf#login_button");
    load::<VabAsset>(&mut shipped, "spirit.swf");
}

#[test]
fn compiler_byte_and_file_apis_match_all_modes() {
    let root = directory("bytes");
    for (name, mode) in [
        ("ui_demo", SwfCompileMode::StaticUi),
        ("animated_ui", SwfCompileMode::AnimatedUi),
        ("ui_demo", SwfCompileMode::Animation),
    ] {
        let input = PathBuf::from(format!("assets/{name}.swf"));
        let settings = SwfCompileSettings { mode };
        let compiled = vatf::compile_swf(&fs::read(&input).unwrap(), &settings).unwrap();
        let output = root.join(format!("{mode:?}.vab"));
        let report = vatf::convert_swf(&input, &output, &settings).unwrap();
        assert_eq!(compiled.bytes, fs::read(output).unwrap());
        assert_eq!(compiled.pruning, report);
        vatf::reader::VabReader::from_bytes(&compiled.bytes).unwrap();
    }
    assert!(vatf::compile_swf(b"not SWF", &SwfCompileSettings::default()).is_err());
}

#[test]
fn invalid_source_fails_processing_and_example_metadata_is_valid() {
    for name in ["ui_demo", "animated_ui", "login"] {
        let meta = fs::read(format!("examples/processed_assets/{name}.swf.meta")).unwrap();
        AssetMeta::<(), SwfToVabProcessor>::deserialize(&meta).unwrap();
    }
    let root = directory("invalid");
    fs::write(root.join("source/broken.swf"), b"not a SWF").unwrap();
    let output = vab_processed_asset_path(root.join("cache"));
    let mut app = app(&root.join("source"), &output, &root.join("log"), true);
    let handle = app
        .world()
        .resource::<AssetServer>()
        .load::<VabAsset>("broken.swf");
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        app.update();
        match app
            .world()
            .resource::<AssetServer>()
            .get_load_state(&handle)
        {
            Some(LoadState::Failed(_)) => break,
            Some(LoadState::Loaded) => panic!("invalid SWF must not load"),
            _ => {}
        }
        assert!(
            Instant::now() < deadline,
            "failed processing was not reported to the loader"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn watcher_events_reprocess_and_reload_existing_subasset_handles() {
    use bevy::asset::io::{AssetSourceBuilder, AssetSourceEvent, AssetSourceId, AssetWatcher};
    use bevy::mesh::VertexAttributeValues;
    use std::sync::{Arc, Mutex};
    struct ManualWatcher;
    impl AssetWatcher for ManualWatcher {}
    let root = directory("reload");
    let source = root.join("source");
    let output = vab_processed_asset_path(root.join("cache"));
    let mut bytes = fs::read("assets/ui_demo.swf").unwrap();
    fs::write(source.join("ui.swf"), &bytes).unwrap();
    metadata(&source.join("ui.swf.meta"), SwfCompileMode::StaticUi);
    // Inject real Bevy watcher events deterministically instead of timing OS notifications.
    let source_events = Arc::new(Mutex::new(Vec::new()));
    let processed_events = Arc::new(Mutex::new(Vec::new()));
    let source_sender = source_events.clone();
    let processed_sender = processed_events.clone();
    let builder = AssetSourceBuilder::platform_default(
        &source.to_string_lossy(),
        Some(&output.to_string_lossy()),
    )
    .with_watcher(move |sender| {
        source_sender.lock().unwrap().push(sender);
        Some(Box::new(ManualWatcher))
    })
    .with_processed_watcher(move |sender| {
        processed_sender.lock().unwrap().push(sender);
        Some(Box::new(ManualWatcher))
    });
    let mut app = App::new();
    app.register_asset_source(AssetSourceId::Default, builder)
        .add_plugins((
            MinimalPlugins,
            AssetPlugin {
                mode: AssetMode::Processed,
                use_asset_processor_override: Some(true),
                watch_for_changes_override: Some(true),
                ..default()
            },
            FlashPlayerPlugin,
            VabAssetProcessorPlugin,
        ))
        .init_asset::<Image>()
        .init_asset::<Mesh>();
    app.world()
        .resource::<AssetProcessor>()
        .data()
        .set_log_factory(Box::new(FileTransactionLogFactory {
            file_path: root.join("log"),
        }))
        .unwrap();
    let graphic = load::<VabGraphic>(&mut app, "ui.swf#button_background");
    finished(&mut app);
    let render = app
        .world()
        .resource::<Assets<VabGraphic>>()
        .get(&graphic)
        .unwrap()
        .render_asset
        .clone();
    let mesh = app
        .world()
        .resource::<Assets<VabAsset>>()
        .get(&render)
        .unwrap()
        .render_meshes[0]
        .mesh
        .clone();
    fn red(app: &App, mesh: &Handle<Mesh>) -> f32 {
        match app
            .world()
            .resource::<Assets<Mesh>>()
            .get(mesh)
            .unwrap()
            .attribute(Mesh::ATTRIBUTE_COLOR)
            .unwrap()
        {
            VertexAttributeValues::Float32x4(values) => values[0][0],
            _ => panic!("expected decoded vertex colors"),
        }
    }
    let initial_red = red(&app, &mesh);
    let index = bytes
        .windows(4)
        .position(|rgba| rgba == [230, 130, 40, 255])
        .unwrap();
    bytes[index] = 50;
    fs::write(source.join("ui.swf"), &bytes).unwrap();
    source_events.lock().unwrap()[0]
        .try_send(AssetSourceEvent::ModifiedAsset("ui.swf".into()))
        .unwrap();
    let expected = vatf::compile_swf(
        &bytes,
        &SwfCompileSettings {
            mode: SwfCompileMode::StaticUi,
        },
    )
    .unwrap()
    .bytes;
    let deadline = Instant::now() + Duration::from_secs(60);
    while fs::read(output.join("ui.swf")).ok().as_deref() != Some(expected.as_slice()) {
        app.update();
        assert!(
            Instant::now() < deadline,
            "watcher event did not recompile source"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    finished(&mut app);
    processed_events.lock().unwrap()[0]
        .try_send(AssetSourceEvent::ModifiedAsset("ui.swf".into()))
        .unwrap();
    while red(&app, &mesh) >= initial_red / 2.0 {
        app.update();
        assert!(
            Instant::now() < deadline,
            "processed output did not reload mesh"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        app.world()
            .resource::<Assets<VabGraphic>>()
            .get(&graphic)
            .unwrap()
            .render_asset,
        render
    );
    assert_eq!(
        app.world()
            .resource::<Assets<VabAsset>>()
            .get(&render)
            .unwrap()
            .render_meshes[0]
            .mesh,
        mesh
    );
}
