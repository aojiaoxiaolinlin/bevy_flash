use bevy::{
    app::{App, Startup, Update},
    asset::{AssetServer, Assets, Handle},
    color::{palettes::css::GOLD, Color},
    diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin},
    gizmos::gizmos,
    input::ButtonInput,
    log::info,
    prelude::{
        Camera2dBundle, Commands, Component, Gizmos, KeyCode, Msaa, Query, Res, ResMut,
        SpatialBundle, TextBundle, Transform, With,
    },
    text::{Text, TextSection, TextStyle},
    DefaultPlugins,
};
use bevy_flash::swf::display_object::{movie_clip::MovieClip, DisplayObject, TDisplayObject};
use bevy_flash::{
    assets::SwfMovie,
    bundle::{Swf, SwfBundle},
    plugin::FlashPlugin,
};
use glam::{Vec2, Vec3};

#[derive(Component)]
struct FpsText;

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, FrameTimeDiagnosticsPlugin, FlashPlugin))
        .insert_resource(Msaa::Sample8)
        .add_systems(Startup, setup)
        .add_systems(Update, (control, text_update_system, draw_grid))
        .run();
}

fn setup(mut commands: Commands, assert_server: Res<AssetServer>) {
    commands.spawn(Camera2dBundle::default());
    commands.spawn(SwfBundle {
        // swf_handle: assert_server.load("sprite.swf"),
        // swf_handle: assert_server.load("scale.swf"),
        // swf_handle: assert_server.load("rotate.swf"),
        // swf_handle: assert_server.load("head_scale2.swf"),
        // swf_handle: assert_server.load("head-animation.swf"),
        // swf_handle: assert_server.load("head.swf"),
        // swf_handle: assert_server.load("spirit2159src.swf"),
        swf_handle: assert_server.load("spirit2724src.swf"),
        // swf_handle: assert_server.load("spirit2256src.swf"),
        // swf_handle: assert_server.load("double_ref2.swf"),
        // swf_handle: assert_server.load("effect1209.swf"),
        // swf_handle: assert_server.load("frames.swf"),
        // swf_handle: assert_server.load("bitmap_test.swf"),
        // swf_handle: assert_server.load("miaomiao.swf"),
        // swf_handle: assert_server.load("tou.swf"),
        // swf_handle: assert_server.load("123680-idle.swf"),
        // swf_handle: assert_server.load("frame_animation.swf"),
        // swf_handle: assert_server.load("gradient.swf"),
        // swf_handle: assert_server.load("weiba.swf"),
        // swf_handle: assert_server.load("spirit1src.swf"),
        // swf_handle: assert_server.load("32.swf"),
        // swf_handle: assert_server.load("30.swf"),
        swf: Swf {
            name: Some(String::from("_mc")),
            ..Default::default()
        },
        // TODO: X、Y坐标变换会引起渲染显示异常，不显示或渐变纹理没有填充，该异常还与窗口大小有关系
        // 问题可能是这个发生平移变换后，检测已经不再可渲染窗口内部了，之所以能看到图形，是因为shader对顶点进行了变换
        spatial: SpatialBundle {
            transform: Transform::from_translation(Vec3::new(-3000.0, 900.0, 0.0))
                .with_scale(Vec3::new(4.0, -4.0, 1.0)),
            ..Default::default()
        },
        ..Default::default()
    });

    commands.spawn((
        TextBundle::from_sections([
            TextSection::new(
                "FPS",
                TextStyle {
                    font_size: 40.0,
                    ..Default::default()
                },
            ),
            TextSection::from_style(TextStyle {
                color: GOLD.into(),
                ..Default::default()
            }),
        ]),
        FpsText,
    ));
}

fn draw_grid(mut gizmos: Gizmos) {
    gizmos.line_2d(Vec2::new(-500.0, 0.0), Vec2::new(500.0, 0.0), Color::WHITE);
    gizmos.line_2d(Vec2::new(0.0, -500.0), Vec2::new(0.0, 500.0), Color::WHITE);
}

fn control(
    mut query: Query<(&mut Swf, &Handle<SwfMovie>)>,
    mut swf_movies: ResMut<Assets<SwfMovie>>,
    keyboard_input: Res<ButtonInput<KeyCode>>,
) {
    let mut control = |query: &mut Query<'_, '_, (&mut Swf, &Handle<SwfMovie>)>, frame: u16| {
        query.iter_mut().for_each(|(mut swf, handle_swf_movie)| {
            if let Some(swf_movie) = swf_movies.get_mut(handle_swf_movie.id()) {
                if swf.is_target_movie_clip() {
                    swf.root_movie_clip
                        .goto_frame(&mut swf_movie.movie_library, frame, true);
                }
            }
        });
    };

    if keyboard_input.just_released(KeyCode::KeyW) {
        control(&mut query, 0);
    }

    if keyboard_input.just_released(KeyCode::KeyA) {
        control(&mut query, 10);
    }

    if keyboard_input.just_released(KeyCode::KeyS) {
        control(&mut query, 20);
    }

    if keyboard_input.just_released(KeyCode::KeyD) {
        control(&mut query, 30);
    }

    if keyboard_input.just_released(KeyCode::KeyF) {
        control(&mut query, 40);
    }

    if keyboard_input.just_released(KeyCode::KeyH) {
        control(&mut query, 50);
    }

    if keyboard_input.just_released(KeyCode::KeyJ) {
        control(&mut query, 60);
    }

    if keyboard_input.just_released(KeyCode::KeyK) {
        control(&mut query, 70);
    }

    if keyboard_input.just_released(KeyCode::KeyL) {
        control(&mut query, 80);
    }

    if keyboard_input.just_released(KeyCode::KeyM) {
        control(&mut query, 90);
    }

    if keyboard_input.just_released(KeyCode::KeyN) {
        control(&mut query, 100);
    }

    if keyboard_input.just_released(KeyCode::KeyO) {
        control(&mut query, 110);
    }

    // query.iter().for_each(|(swf, _)| {
    //     let movie_clip = &swf.root_movie_clip;
    //     println!("MovieClip:{}", movie_clip.character_id());
    //     let space = 0;
    //     show(movie_clip, space);
    // });

    // println!("-------------end----------------------");
}

fn show(movie_clip: &MovieClip, mut space: i32) {
    space += 2;
    let render_list = movie_clip.raw_container().render_list();
    let display_objects = movie_clip.raw_container().display_objects();
    render_list.iter().for_each(|display_id| {
        let display_object = display_objects.get(&display_id).unwrap();
        match display_object {
            DisplayObject::MovieClip(movie_clip) => {
                for _ in 0..space {
                    print!(" ");
                }
                println!(
                    "MovieClip:{} depth:{}",
                    movie_clip.character_id(),
                    movie_clip.depth()
                );
                show(movie_clip, space);
            }
            DisplayObject::Graphic(graphic) => {
                for _ in 0..space {
                    print!(" ");
                }
                println!("Graphic:{:?}", graphic.character_id());
            }
        }
    });
}

fn text_update_system(
    diagnostics: Res<DiagnosticsStore>,
    mut query: Query<&mut Text, With<FpsText>>,
) {
    for mut text in &mut query {
        if let Some(fps) = diagnostics.get(&FrameTimeDiagnosticsPlugin::FPS) {
            if let Some(value) = fps.smoothed() {
                // Update the value of the second section
                text.sections[1].value = format!("{value:.2}");
            }
        }
    }
}
