//! Reference host for `FlashPlayerPlugin`.
//!
//! Plays one VAB instance and lists every clip the asset exposes. A clip can be
//! started once by clicking its row or by moving the cursor and pressing Enter.
//! The first clip is the looping fallback; explicit loop, hold and terminal
//! playback plus pause, speed, seek and frame-rate override are on the keyboard.
//! The instance is placed and framed from measured frame bounds, so no constants
//! have to be tuned per asset.

use bevy::{camera::ScalingMode, prelude::*};
use bevy_flash::{
    FlashPlayerPlugin,
    sampling::VabSkin,
    vab_asset::{CommandList, VabAsset, VabAssetHandle, VabCommand},
    vab_player::{VabCompleteEvent, VabFrameEvent, VabPlayer},
};

const ASSET_PATH: &str = "Dra_LeShan3_battle.vab";

/// Output-pixel scale for the instance.
const SPIRIT_SCALE: f32 = 2.0;

/// How much room the camera leaves around the instance. The extra width is what
/// the clip list sits in.
const VIEW_WIDTH_FACTOR: f32 = 2.1;
const VIEW_HEIGHT_FACTOR: f32 = 1.35;

/// Shift the content centre right to leave room for the controls on the left.
const CONTENT_RIGHT_OFFSET: f32 = 0.22;

const FONT_SIZE: f32 = 16.0;
const PANEL_PADDING: f32 = 8.0;
const ROW_PADDING: f32 = 2.0;

const CURSOR_COLOR: Color = Color::srgb(1.0, 0.85, 0.3);
const PLAYING_COLOR: Color = Color::srgb(0.45, 1.0, 0.5);
const FALLBACK_COLOR: Color = Color::srgb(0.35, 0.85, 1.0);
const IDLE_COLOR: Color = Color::srgb(0.82, 0.82, 0.82);
const CURSOR_BACKGROUND: Color = Color::srgba(1.0, 0.85, 0.3, 0.22);

const SPEED_STEP: f64 = 0.25;
const MIN_SPEED: f64 = 0.25;
const MAX_SPEED: f64 = 4.0;

const RATE_STEP: f32 = 5.0;
const MIN_RATE: f32 = 5.0;
const MAX_RATE: f32 = 120.0;

#[derive(Component)]
struct Spirit;

/// Set on the instance while it still needs post-load initialisation.
#[derive(Component)]
struct StartWhenLoaded;

/// Panel title, written once the asset is known.
#[derive(Component)]
struct TitleText;

/// Column that the clip rows are added to.
#[derive(Component)]
struct ClipRows;

/// One clickable row of the clip list, holding its clip index.
#[derive(Component)]
struct ClipRow(usize);

/// Live status line plus the key hints.
#[derive(Component)]
struct PanelInfo;

/// The instance's frame bounds in asset-local units, measured from the asset.
/// `content_size` stays zero until the measuring system has run.
#[derive(Resource, Default)]
struct VabLayout {
    content_center: Vec2,
    content_size: Vec2,
}

/// Keyboard cursor into the clip list, and whether the rows exist yet.
#[derive(Resource, Default)]
struct ClipList {
    cursor: usize,
    built: bool,
    fallback: Option<usize>,
}

fn main() {
    App::new()
        .insert_resource(ClearColor(Color::srgb_u8(102, 102, 102)))
        .init_resource::<VabLayout>()
        .init_resource::<ClipList>()
        .add_plugins((DefaultPlugins, FlashPlayerPlugin))
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (
                start_when_loaded,
                build_clip_list,
                place_spirit,
                frame_camera,
                clip_list_control,
                playback_control,
                update_panel,
                log_playback_events,
            )
                .chain(),
        )
        .run();
}

fn panel_text(content: impl Into<String>) -> (Text, TextFont, TextLayout) {
    (
        Text::new(content),
        TextFont {
            font_size: FontSize::Px(FONT_SIZE),
            ..default()
        },
        TextLayout::no_wrap(),
    )
}

fn setup(mut commands: Commands, asset_server: Res<AssetServer>) {
    // `Camera2d` requires a 2D orthographic projection; `frame_camera` replaces
    // it with a fitted one as soon as the asset's bounds are known.
    commands.spawn((Camera2d, CompositingSpace::Srgb));

    commands.spawn((
        Spirit,
        Name::new("spirit"),
        StartWhenLoaded,
        VabAssetHandle(asset_server.load(ASSET_PATH)),
        VabPlayer::default(),
        Transform::from_scale(Vec3::splat(SPIRIT_SCALE)),
    ));

    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            top: px(12),
            left: px(12),
            flex_direction: FlexDirection::Column,
            padding: UiRect::all(px(PANEL_PADDING)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6)),
        children![
            (
                TitleText,
                panel_text("loading..."),
                TextColor(Color::WHITE),
                Node {
                    margin: UiRect::bottom(px(6.0)),
                    ..default()
                },
            ),
            (
                ClipRows,
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: px(1),
                    ..default()
                },
            ),
            (
                PanelInfo,
                panel_text(""),
                TextColor(Color::srgb(0.62, 0.62, 0.62)),
                Node {
                    margin: UiRect::top(px(6.0)),
                    ..default()
                },
            ),
        ],
    ));
}

/// Waits for the asset, then measures the layout and starts playback. The
/// marker is consumed exactly once.
fn start_when_loaded(
    mut commands: Commands,
    assets: Res<Assets<VabAsset>>,
    mut layout: ResMut<VabLayout>,
    mut list: ResMut<ClipList>,
    mut players: Query<(Entity, &VabAssetHandle, &mut VabPlayer), With<StartWhenLoaded>>,
) {
    for (entity, handle, mut player) in &mut players {
        let Some(asset) = assets.get(&handle.0) else {
            continue;
        };
        match measure_content(asset) {
            Ok((center, size)) => {
                info!("{ASSET_PATH}: measured content centre {center}, size {size}");
                layout.content_center = center;
                layout.content_size = size;
            }
            Err(error) => warn!("{ASSET_PATH}: cannot measure content bounds: {error}"),
        }
        match asset.baked.clips.first() {
            Some(clip) => {
                info!(
                    "{ASSET_PATH}: using {} as fallback and playing it by default",
                    clip.name
                );
                if let Err(error) = player.set_fallback_loop(asset, &clip.name) {
                    warn!("{} cannot be used as fallback: {error}", clip.name);
                } else if let Err(error) = player.play_loop(asset, &clip.name) {
                    warn!("{} is unavailable: {error}", clip.name);
                } else {
                    list.fallback = Some(0);
                }
            }
            None => warn!("{ASSET_PATH} exposes no clips"),
        }
        commands.entity(entity).remove::<StartWhenLoaded>();
    }
}

/// Adds one clickable row per clip. Rows are built lazily because the clip names
/// are only known once the asset has loaded.
fn build_clip_list(
    mut commands: Commands,
    assets: Res<Assets<VabAsset>>,
    spirit: Query<&VabAssetHandle, With<Spirit>>,
    mut list: ResMut<ClipList>,
    titles: Query<Entity, (With<TitleText>, Without<ClipRow>)>,
    columns: Query<Entity, (With<ClipRows>, Without<ClipRow>)>,
) {
    if list.built {
        return;
    }
    let Ok(handle) = spirit.single() else {
        return;
    };
    let Some(vab) = assets.get(&handle.0) else {
        return;
    };
    let (Ok(title), Ok(column)) = (titles.single(), columns.single()) else {
        return;
    };
    let clips = &vab.baked.clips;
    if clips.is_empty() {
        return;
    }

    info!(
        "{ASSET_PATH}: {} clips ({})",
        clips.len(),
        clips
            .iter()
            .map(|clip| clip.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    commands.entity(title).insert(Text::new(format!(
        "{ASSET_PATH}\n{} clips   {:.1} fps",
        clips.len(),
        vab.playback_frame_rate()
    )));
    commands.entity(column).with_children(|column| {
        for (index, clip) in clips.iter().enumerate() {
            column.spawn((
                ClipRow(index),
                panel_text(clip_row_text(index, &clip.name, clip.frames.len())),
                TextColor(IDLE_COLOR),
                Button,
                Node {
                    padding: UiRect::axes(px(6.0), px(ROW_PADDING)),
                    ..default()
                },
            ));
        }
    });
    list.built = true;
}

fn clip_row_text(index: usize, name: &str, frames: usize) -> String {
    format!("{:>2}  {name:<6} {frames:>4} frames", index + 1)
}

/// Places the instance so the content centre lands on the world origin.
///
/// The renderer composes `world_from_local = GlobalTransform * VIEW_MATRIX * local`
/// and `VIEW_MATRIX` flips Y (`src/render/instance.rs`), so an asset-local point
/// `p` reaches `(t.x + scale * p.x, t.y - scale * p.y)`.
fn place_spirit(layout: Res<VabLayout>, mut spirit: Query<&mut Transform, With<Spirit>>) {
    if layout.content_size.x <= 0.0 || layout.content_size.y <= 0.0 {
        return;
    }
    let view_width = SPIRIT_SCALE * layout.content_size.x * VIEW_WIDTH_FACTOR;
    let x = -SPIRIT_SCALE * layout.content_center.x + view_width * CONTENT_RIGHT_OFFSET;
    let y = SPIRIT_SCALE * layout.content_center.y;
    for mut transform in &mut spirit {
        if (transform.translation.x - x).abs() > f32::EPSILON
            || (transform.translation.y - y).abs() > f32::EPSILON
        {
            transform.translation = Vec3::new(x, y, 0.0);
        }
    }
}

/// Frames the instance once its bounds are known. `AutoMin` keeps the aspect
/// ratio while guaranteeing the requested world extent stays visible.
fn frame_camera(
    layout: Res<VabLayout>,
    mut applied: Local<Vec2>,
    mut cameras: Query<&mut Projection, With<Camera2d>>,
) {
    if layout.content_size.x <= 0.0 || layout.content_size.y <= 0.0 {
        return;
    }
    let extent = Vec2::new(
        SPIRIT_SCALE * layout.content_size.x * VIEW_WIDTH_FACTOR,
        SPIRIT_SCALE * layout.content_size.y * VIEW_HEIGHT_FACTOR,
    );
    if (extent.x - applied.x).abs() < 0.5 && (extent.y - applied.y).abs() < 0.5 {
        return;
    }
    *applied = extent;
    for mut projection in &mut cameras {
        *projection = Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::AutoMin {
                min_width: extent.x,
                min_height: extent.y,
            },
            ..OrthographicProjection::default_2d()
        });
    }
}

/// Moves the list cursor and starts clips. Both the arrow keys and a click on a
/// row land here, so the two input paths cannot drift apart.
fn clip_list_control(
    keyboard: Res<ButtonInput<KeyCode>>,
    assets: Res<Assets<VabAsset>>,
    mut list: ResMut<ClipList>,
    rows: Query<(&ClipRow, &Interaction), Changed<Interaction>>,
    mut spirit: Query<(&VabAssetHandle, &mut VabPlayer), With<Spirit>>,
) {
    let Ok((handle, mut player)) = spirit.single_mut() else {
        return;
    };
    let Some(asset) = assets.get(&handle.0) else {
        return;
    };
    let count = asset.baked.clips.len();
    if count == 0 {
        return;
    }
    list.cursor = list.cursor.min(count - 1);

    if keyboard.just_pressed(KeyCode::ArrowUp) {
        list.cursor = list.cursor.saturating_sub(1);
    }
    if keyboard.just_pressed(KeyCode::ArrowDown) {
        list.cursor = (list.cursor + 1).min(count - 1);
    }

    enum Request {
        Once,
        Loop,
        Hold,
        Terminal,
    }

    let mut requested = None;
    if keyboard.just_pressed(KeyCode::Enter) || keyboard.just_pressed(KeyCode::Space) {
        requested = Some((list.cursor, Request::Once));
    }
    if keyboard.just_pressed(KeyCode::KeyL) {
        requested = Some((list.cursor, Request::Loop));
    }
    if keyboard.just_pressed(KeyCode::KeyH) {
        requested = Some((list.cursor, Request::Hold));
    }
    if keyboard.just_pressed(KeyCode::KeyT) {
        requested = Some((list.cursor, Request::Terminal));
    }
    for (row, interaction) in &rows {
        if *interaction == Interaction::Pressed && row.0 < count {
            list.cursor = row.0;
            requested = Some((row.0, Request::Once));
        }
    }

    if keyboard.just_pressed(KeyCode::KeyF) {
        let fallback = asset.baked.clips[list.cursor].name.clone();
        match player.set_fallback_loop(asset, &fallback) {
            Ok(_) => {
                list.fallback = Some(list.cursor);
                info!("fallback animation changed to {fallback}");
            }
            Err(error) => warn!("{fallback} cannot be used as fallback: {error}"),
        }
    }

    if keyboard.just_pressed(KeyCode::KeyU) && player.is_terminal() {
        player.reset_terminal();
        if let Some(fallback) = list.fallback.and_then(|index| asset.baked.clips.get(index))
            && let Err(error) = player.play_loop(asset, &fallback.name)
        {
            warn!("{} is unavailable: {error}", fallback.name);
        }
    }

    if let Some((index, request)) = requested {
        let name = asset.baked.clips[index].name.clone();
        let result = match request {
            Request::Once => player.play_once(asset, &name),
            Request::Loop => player.play_loop(asset, &name),
            Request::Hold => player.play_once_and_hold(asset, &name),
            Request::Terminal => player.play_terminal(asset, &name),
        };
        if let Err(error) = result {
            warn!("{name} is unavailable: {error}");
        }
    }
}

/// The rest of the playback API: pause, speed, single-frame stepping and the
/// frame-rate override.
fn playback_control(
    keyboard: Res<ButtonInput<KeyCode>>,
    assets: Res<Assets<VabAsset>>,
    mut spirit: Query<(&VabAssetHandle, &mut VabPlayer), With<Spirit>>,
) {
    let Ok((handle, mut player)) = spirit.single_mut() else {
        return;
    };
    let Some(asset) = assets.get(&handle.0) else {
        return;
    };

    if keyboard.just_pressed(KeyCode::KeyP) {
        if player.playing {
            player.pause();
        } else {
            player.resume();
        }
    }

    let speed = player.speed();
    if keyboard.just_pressed(KeyCode::Comma) {
        set_speed(&mut player, speed - SPEED_STEP);
    }
    if keyboard.just_pressed(KeyCode::Period) {
        set_speed(&mut player, speed + SPEED_STEP);
    }

    let frames = asset
        .baked
        .clips
        .get(player.clip())
        .map_or(0, |clip| clip.frames.len());
    let step = if keyboard.just_pressed(KeyCode::ArrowLeft) {
        -1
    } else if keyboard.just_pressed(KeyCode::ArrowRight) {
        1
    } else {
        0
    };
    if step != 0 && frames > 0 {
        // `advance_vab_animations` keeps moving the frame every tick, so a
        // single step only reads as one frame if playback is paused from here
        // on. `P` resumes.
        player.pause();
        let target = (player.current_frame as isize + step).clamp(0, frames as isize - 1);
        if let Err(error) = player.seek(asset, target as usize) {
            warn!("seek to frame {target} failed: {error}");
        }
    }

    let rate = player
        .frame_rate_override
        .unwrap_or_else(|| asset.playback_frame_rate());
    if keyboard.just_pressed(KeyCode::BracketLeft) {
        player.frame_rate_override = Some((rate - RATE_STEP).max(MIN_RATE));
    }
    if keyboard.just_pressed(KeyCode::BracketRight) {
        player.frame_rate_override = Some((rate + RATE_STEP).min(MAX_RATE));
    }
    if keyboard.just_pressed(KeyCode::Backslash) {
        player.frame_rate_override = None;
    }
}

fn set_speed(player: &mut VabPlayer, speed: f64) {
    let clamped = speed.clamp(MIN_SPEED, MAX_SPEED);
    if let Err(error) = player.set_speed(clamped) {
        warn!("speed {clamped} rejected: {error}");
    }
}

/// Highlights the cursor and the playing clip, and rewrites the status line.
fn update_panel(
    assets: Res<Assets<VabAsset>>,
    list: Res<ClipList>,
    spirit: Query<(&VabAssetHandle, &VabPlayer), With<Spirit>>,
    mut info: Query<&mut Text, (With<PanelInfo>, Without<ClipRow>)>,
    mut rows: Query<(&ClipRow, &mut TextColor, &mut BackgroundColor)>,
) {
    let Ok((handle, player)) = spirit.single() else {
        return;
    };
    let Some(asset) = assets.get(&handle.0) else {
        return;
    };
    let clips = &asset.baked.clips;

    for (row, mut color, mut background) in &mut rows {
        let playing = row.0 == player.clip();
        let cursor = row.0 == list.cursor;
        let fallback = Some(row.0) == list.fallback;
        let wanted = if playing {
            PLAYING_COLOR
        } else if cursor {
            CURSOR_COLOR
        } else if fallback {
            FALLBACK_COLOR
        } else {
            IDLE_COLOR
        };
        if color.0 != wanted {
            color.0 = wanted;
        }
        let wanted_background = if cursor {
            CURSOR_BACKGROUND
        } else {
            Color::NONE
        };
        if background.0 != wanted_background {
            background.0 = wanted_background;
        }
    }

    let Ok(mut info) = info.single_mut() else {
        return;
    };
    let status = match clips.get(player.clip()) {
        Some(clip) => {
            let state = if player.is_terminal() && player.is_complete() {
                "terminal complete"
            } else if player.is_terminal() {
                "terminal"
            } else if player.is_complete() {
                "complete"
            } else if player.playing {
                "playing"
            } else {
                "paused"
            };
            let looping = if player.is_looping() { "loop" } else { "once" };
            let (rate, source) = match player.frame_rate_override {
                Some(rate) => (rate, "override"),
                None => (asset.playback_frame_rate(), "asset"),
            };
            format!(
                "{state} {}   frame {}/{}   {looping}   {:.2}x   rate {rate:.1} fps ({source})",
                clip.name,
                player.current_frame,
                clip.frames.len(),
                player.speed(),
            )
        }
        None => "no clip".to_owned(),
    };
    let fallback = list
        .fallback
        .and_then(|index| clips.get(index))
        .map_or("none", |clip| clip.name.as_str());
    let text = format!(
        "{status}   fallback {fallback}\n\n\
         up/down  move in the list      enter/click  play once, then fallback\n\
         F  set selected as fallback    L  loop selected\n\
         H  play once and hold\n\
         T  terminal selected           U  unlock and return to fallback\n\
         P  pause / resume\n\
         , .  speed down / up           left right  step one frame (pauses)\n\
         [ ]  frame rate down / up      backslash  back to the asset frame rate"
    );
    if info.0 != text {
        info.0 = text;
    }
}

fn log_playback_events(
    mut frames: MessageReader<VabFrameEvent>,
    mut completions: MessageReader<VabCompleteEvent>,
) {
    for event in frames.read() {
        info!(
            "frame event: animation={} frame={} name={}",
            event.animation, event.frame, event.name
        );
    }
    for event in completions.read() {
        info!("animation={} complete", event.animation);
    }
}

/// Measures the asset's frame bounds using only public API: sample a few frames
/// of the first clip at scale 1 (so units are source SWF units), then walk the
/// command list and union every shape's transformed `local_bounds`.
fn measure_content(asset: &VabAsset) -> Result<(Vec2, Vec2), String> {
    let frame_count = asset
        .baked
        .clips
        .first()
        .map(|clip| clip.frames.len())
        .unwrap_or(0);
    if frame_count == 0 {
        return Err("first clip has no frames".to_owned());
    }

    let mut min = Vec2::splat(f32::INFINITY);
    let mut max = Vec2::splat(f32::NEG_INFINITY);
    let mut measured = false;
    for frame in [0, frame_count / 4, frame_count / 2, frame_count * 3 / 4] {
        let commands = asset
            .sample(0, frame, &VabSkin::default(), Vec3::ONE)
            .map_err(|error| format!("frame {frame}: {error}"))?;
        measured |= accumulate_bounds(asset, &commands, &mut min, &mut max);
    }
    info!("measured bounds: min {min}, max {max}");
    if !measured {
        return Err("no shapes in the sampled frames".to_owned());
    }
    Ok(((min + max) * 0.5, max - min))
}

fn accumulate_bounds(
    asset: &VabAsset,
    commands: &CommandList,
    min: &mut Vec2,
    max: &mut Vec2,
) -> bool {
    let mut measured = false;
    for command in &commands.commands {
        match command {
            VabCommand::RenderShape { handle, transform } => {
                let Some(mesh) = asset.render_meshes.get(*handle) else {
                    continue;
                };
                let [x0, y0, x1, y1] = mesh.local_bounds;
                let matrix = transform.matrix;
                for (x, y) in [(x0, y0), (x1, y0), (x0, y1), (x1, y1)] {
                    let point = Vec2::new(
                        matrix.a * x + matrix.c * y + matrix.tx,
                        matrix.b * x + matrix.d * y + matrix.ty,
                    );
                    *min = min.min(point);
                    *max = max.max(point);
                    measured = true;
                }
            }
            VabCommand::ApplyFilter { commands, .. } => {
                measured |= accumulate_bounds(asset, commands, min, max);
            }
            VabCommand::Blend(commands, _) => {
                measured |= accumulate_bounds(asset, commands, min, max);
            }
            VabCommand::PushMask
            | VabCommand::ActivateMask
            | VabCommand::DeactivateMask
            | VabCommand::PopMask => {}
        }
    }
    measured
}
