use bevy::{
    DefaultPlugins,
    app::{App, Startup, Update},
    asset::{AssetServer, Assets},
    color::Color,
    dev_tools::fps_overlay::FpsOverlayPlugin,
    input::ButtonInput,
    math::Vec3,
    prelude::{
        Camera2d, ClearColor, Commands, Entity, EventReader, KeyCode, Msaa, PluginGroup, Query,
        Res, ResMut, Transform,
    },
    window::{Window, WindowPlugin},
};
use bevy_flash::{
    assets::SwfMovie,
    bundle::FlashAnimation,
    swf::display_object::{TDisplayObject, movie_clip::NextFrame},
    {FlashPlayerTimer, FlashPlugin, SwfInitEvent},
};

fn main() {
    App::new()
        .insert_resource(ClearColor(Color::srgb(
            102.0 / 255.0,
            102.0 / 255.0,
            102.0 / 255.0,
        )))
        .add_plugins((
            DefaultPlugins.set(WindowPlugin {
                primary_window: Some(Window {
                    // present_mode: bevy::window::PresentMode::AutoNoVsync,
                    ..Default::default()
                }),
                ..Default::default()
            }),
            FlashPlugin,
            FpsOverlayPlugin::default(),
        ))
        .add_systems(Startup, setup)
        .add_systems(Update, control)
        .run();
}

fn setup(mut commands: Commands, assert_server: Res<AssetServer>) {
    // 统一flash动画的帧率
    commands.insert_resource(FlashPlayerTimer::from_frame_rate(24.));

    commands.spawn((Camera2d, Msaa::Sample8));
    commands.spawn((
        FlashAnimation {
            name: Some(String::from("mc")),
            swf_movie: assert_server.load("spirit2159src.swf"),
            ignore_root_swf_transform: false,
            ..Default::default()
        },
        Transform::from_translation(Vec3::new(0.0, 0.0, 0.0)).with_scale(Vec3::splat(2.0)),
    ));

    // commands.spawn((
    //     FlashAnimation {
    //         name: Some(String::from("m")),
    //         swf_movie: assert_server.load("131381-idle.swf"),
    //         ..Default::default()
    //     },
    //     Transform::from_translation(Vec3::new(0.0, 0.0, 0.0)).with_scale(Vec3::splat(8.0)),
    // ));

    commands.spawn((
        FlashAnimation {
            name: Some(String::from("m")),
            swf_movie: assert_server.load("leiyi.swf"),
            ignore_root_swf_transform: false,
            ..Default::default()
        },
        Transform::from_translation(Vec3::new(0.0, 0.0, 0.0)).with_scale(Vec3::splat(2.0)),
    ));

    // commands.spawn(FlashAnimation {
    //     name: Some(String::from("blend_add")),
    //     swf_movie: assert_server.load("blend_add.swf"),
    //     ignore_root_swf_transform: false,
    //     ..Default::default()
    // });
    // commands.spawn((
    //     FlashAnimation {
    //         name: Some(String::from("blend_sub")),
    //         swf_movie: assert_server.load("blend_sub.swf"),
    //         ignore_root_swf_transform: false,
    //         ..Default::default()
    //     },
    //     Transform::from_translation(Vec3::new(-400.0, 0.0, 0.0)),
    // ));
    // commands.spawn((
    //     FlashAnimation {
    //         name: Some(String::from("blend_screen")),
    //         swf_movie: assert_server.load("blend_screen.swf"),
    //         ignore_root_swf_transform: false,
    //         ..Default::default()
    //     },
    //     Transform::from_translation(Vec3::new(-800.0, 0.0, 0.0)),
    // ));
}

fn control(
    mut query: Query<(&mut FlashAnimation, Entity)>,
    mut swf_movies: ResMut<Assets<SwfMovie>>,
    keyboard_input: Res<ButtonInput<KeyCode>>,
    mut swf_init_event: EventReader<SwfInitEvent>,
) {
    for swf_init_event in swf_init_event.read() {
        query.iter_mut().for_each(|(flash_animation, entity)| {
            let name = flash_animation.name.clone();
            if let Some(swf_movie) = swf_movies.get_mut(flash_animation.swf_movie.id()) {
                swf_movie.movie_clip.set_name(name);
                if swf_movie.movie_clip.name() == Some("mc") {
                    if swf_init_event.0 == entity {
                        swf_movie
                            .movie_clip
                            .goto_frame(&mut swf_movie.library, 0, true);
                    }
                }
            }
        });
    }

    query.iter_mut().for_each(|(flash_animation, _)| {
        if let Some(swf_movie) = swf_movies.get_mut(flash_animation.swf_movie.id()) {
            if flash_animation.name.as_deref() == Some("mc") {
                if let Some(first_child_movie_clip) = swf_movie.movie_clip.first_child_movie_clip()
                {
                    if matches!(
                        first_child_movie_clip.determine_next_frame(),
                        NextFrame::First
                    ) {
                        swf_movie
                            .movie_clip
                            .goto_frame(&mut swf_movie.library, 20, true);
                    }
                }
            }
        }
    });

    let mut control = |query: &mut Query<'_, '_, (&mut FlashAnimation, Entity)>, frame: u16| {
        query.iter_mut().for_each(|(flash_animation, _)| {
            if let Some(swf_movie) = swf_movies.get_mut(flash_animation.swf_movie.id()) {
                if flash_animation.name.as_deref() == Some("mc") {
                    swf_movie
                        .movie_clip
                        .goto_frame(&mut swf_movie.library, frame, true);
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
}
