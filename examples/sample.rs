use bevy::{
    app::{App, Startup, Update},
    asset::{AssetServer, Assets},
    input::ButtonInput,
    math::Vec3,
    prelude::{
        Camera2d, Commands, Entity, EventReader, KeyCode, Msaa, Query, Res, ResMut, Transform,
    },
    DefaultPlugins,
};
use bevy_flash::{
    assets::SwfMovie,
    plugin::{FlashPlugin, SwfInitEvent},
};
use bevy_flash::{
    bundle::FlashAnimation,
    swf::display_object::{
        movie_clip::{MovieClip, NextFrame},
        DisplayObject, TDisplayObject,
    },
};

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, FlashPlugin))
        .add_systems(Startup, setup)
        .add_systems(Update, control)
        .run();
}

fn setup(mut commands: Commands, assert_server: Res<AssetServer>) {
    commands.spawn((Camera2d::default(), Msaa::Sample8));
    commands.spawn((
        FlashAnimation {
            name: Some(String::from("mc")),
            swf_movie: assert_server.load("spirit2724src.swf"),
            ..Default::default()
        },
        Transform::from_translation(Vec3::new(00.0, 00.0, 0.0)).with_scale(Vec3::splat(2.0)),
    ));

    commands.spawn((
        FlashAnimation {
            name: Some(String::from("m")),
            swf_movie: assert_server.load("131381-idle.swf"),
            ..Default::default()
        },
        Transform::from_translation(Vec3::new(-800.0, 200.0, 0.0)).with_scale(Vec3::splat(6.0)),
    ));
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
                if swf_init_event.0 == entity && name.as_deref() == Some("mc") {
                    swf_movie
                        .root_movie_clip
                        .goto_frame(&mut swf_movie.movie_library, 0, true);
                }
                swf_movie.root_movie_clip.set_name(name);
            }
        });
    }

    query.iter_mut().for_each(|(flash_animation, _)| {
        if let Some(swf_movie) = swf_movies.get_mut(flash_animation.swf_movie.id()) {
            if flash_animation.name.as_deref() == Some("mc") {
                if let Some(first_child_movie_clip) =
                    swf_movie.root_movie_clip.first_child_movie_clip()
                {
                    if matches!(
                        first_child_movie_clip.determine_next_frame(),
                        NextFrame::First
                    ) {
                        swf_movie.root_movie_clip.goto_frame(
                            &mut swf_movie.movie_library,
                            20,
                            true,
                        );
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
                        .root_movie_clip
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
