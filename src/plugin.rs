use crate::assets::{SwfLoader, SwfMovie};
use crate::bundle::Swf;
use crate::render::material::{GradientMaterial, GradientUniforms};
use crate::render::tessellator::ShapeTessellator;
use crate::render::FlashRenderPlugin;
use crate::swf::characters::Character;
use crate::swf::display_object::movie_clip::MovieClip;
use crate::swf::display_object::TDisplayObject;
use crate::swf::library::MovieLibrary;
use bevy::app::App;
use bevy::asset::{AssetEvent, Handle};
use bevy::color::{Color, ColorToComponents};
use bevy::log::info;
use bevy::prelude::{
    Commands, Entity, EventReader, Image, IntoSystemConfigs, Mesh, Query, Resource,
};
use bevy::render::mesh::Indices;
use bevy::render::render_asset::RenderAssetUsages;
use bevy::time::{Time, Timer, TimerMode};
use bevy::{
    app::{Plugin, Update},
    asset::{AssetApp, Assets},
    prelude::{Res, ResMut},
};
use copyless::VecHelper;
use glam::{Mat3, Mat4};
use ruffle_render::tessellator::DrawType;
use swf::GradientInterpolation;
use wgpu::PrimitiveTopology;

/// 制作多大得渐变纹理，越大细节越丰富，但是内存占用也越大
const GRADIENT_SIZE: usize = 256;

#[derive(Resource)]
struct PlayerTimer(Timer);

pub struct FlashPlugin;

impl Plugin for FlashPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(FlashRenderPlugin)
            .init_asset::<SwfMovie>()
            .init_asset_loader::<SwfLoader>()
            .insert_resource(PlayerTimer(Timer::from_seconds(
                // TODO: 24fps
                30.0 / 1000.0,
                TimerMode::Repeating,
            )))
            .add_systems(Update, (pre_parse, enter_frame).chain());
    }
}

fn pre_parse(
    mut commands: Commands,
    query: Query<(Entity, &Handle<SwfMovie>)>,
    mut swf_events: EventReader<AssetEvent<SwfMovie>>,
    mut swf_movies: ResMut<Assets<SwfMovie>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut gradient_materials: ResMut<Assets<GradientMaterial>>,
) {
    for event in swf_events.read() {
        match event {
            AssetEvent::LoadedWithDependencies { id } => {
                if let Some(swf_movie) = swf_movies.get_mut(*id) {
                    let swf_movie_data = swf_movie.swf_movie.clone();
                    let mut root_movie_clip = MovieClip::new(swf_movie_data);
                    let mut library = MovieLibrary::new();
                    root_movie_clip.parse_swf(&mut library);
                    root_movie_clip.current_frame = 0;
                    info!(
                        "root movie clip total frame:{}",
                        root_movie_clip.total_frames
                    );
                    library.characters_mut().iter_mut().for_each(
                        |(_id, character)| match character {
                            Character::Graphic(graphic) => {
                                let mut shape_tessellator = ShapeTessellator::new();
                                let lyon_mesh =
                                    shape_tessellator.tessellate_shape((&graphic.shape).into());

                                let gradients = lyon_mesh.gradients;

                                let mut gradients_texture = Vec::new();

                                for gradient in gradients {
                                    let colors = if gradient.records.is_empty() {
                                        vec![0; GRADIENT_SIZE * 4]
                                    } else {
                                        let mut colors = vec![0; GRADIENT_SIZE * 4];
                                        let convert = if gradient.interpolation
                                            == GradientInterpolation::LinearRgb
                                        {
                                            |color| color
                                        } else {
                                            |color| color
                                        };

                                        for t in 0..GRADIENT_SIZE {
                                            let mut last = 0;
                                            let mut next = 0;
                                            for (i, record) in
                                                gradient.records.iter().enumerate().rev()
                                            {
                                                if (record.ratio as usize) < t {
                                                    last = i;
                                                    next = (i + 1).min(gradient.records.len() - 1);
                                                    break;
                                                }
                                            }
                                            assert!(last == next || last + 1 == next);
                                            let last_record = &gradient.records[last];
                                            let next_record = &gradient.records[next];
                                            let factor = if next == last {
                                                0.0
                                            } else {
                                                (t as f32 - last_record.ratio as f32)
                                                    / (next_record.ratio as f32
                                                        - last_record.ratio as f32)
                                            };

                                            colors[t * 4] = lerp(
                                                convert(last_record.color.r as f32),
                                                convert(next_record.color.r as f32),
                                                factor,
                                            )
                                                as u8;
                                            colors[(t * 4) + 1] = lerp(
                                                convert(last_record.color.g as f32),
                                                convert(next_record.color.g as f32),
                                                factor,
                                            )
                                                as u8;
                                            colors[(t * 4) + 2] = lerp(
                                                convert(last_record.color.b as f32),
                                                convert(next_record.color.b as f32),
                                                factor,
                                            )
                                                as u8;
                                            colors[(t * 4) + 3] = lerp(
                                                last_record.color.a as f32,
                                                next_record.color.a as f32,
                                                factor,
                                            )
                                                as u8;
                                        }
                                        colors
                                    };

                                    let texture = Image::new(
                                        wgpu::Extent3d {
                                            width: GRADIENT_SIZE as u32,
                                            height: 1,
                                            depth_or_array_layers: 1,
                                        },
                                        wgpu::TextureDimension::D2,
                                        colors,
                                        wgpu::TextureFormat::Rgba8Unorm,
                                        RenderAssetUsages::default(),
                                    );

                                    let gradient_uniforms = GradientUniforms::from(gradient);
                                    gradients_texture.push((texture.clone(), gradient_uniforms));
                                }

                                let mut result_positions = Vec::new();
                                let mut result_colors = Vec::new();
                                let mut result_indices = Vec::new();
                                let mut vertex_num = 0;
                                for draw in lyon_mesh.draws {
                                    match draw.draw_type {
                                        DrawType::Color => {
                                            let current_vertex_num = draw.vertices.len() as u32;
                                            for vertex in draw.vertices {
                                                result_positions.push([vertex.x, vertex.y, 0.0]);
                                                let linear_color = Color::srgba_u8(
                                                    vertex.color.r,
                                                    vertex.color.g,
                                                    vertex.color.b,
                                                    vertex.color.a,
                                                )
                                                .to_linear();
                                                result_colors.push(linear_color.to_f32_array());
                                            }
                                            draw.indices.iter().for_each(|index| {
                                                result_indices.push(*index + vertex_num);
                                            });
                                            vertex_num += current_vertex_num;
                                        }
                                        DrawType::Gradient { matrix, gradient } => {
                                            let mut positions =
                                                Vec::with_capacity(draw.vertices.len());
                                            for vertex in draw.vertices {
                                                positions.alloc().init([vertex.x, vertex.y, 0.0]);
                                            }
                                            let mut mesh = Mesh::new(
                                                PrimitiveTopology::TriangleList,
                                                RenderAssetUsages::default(),
                                            );
                                            mesh.insert_attribute(
                                                Mesh::ATTRIBUTE_POSITION,
                                                positions,
                                            );
                                            mesh.insert_indices(Indices::U32(draw.indices));
                                            let texture =
                                                gradients_texture.get(gradient).unwrap().clone();
                                            graphic.add_gradient_mesh(
                                                meshes.add(mesh),
                                                gradient_materials.add(GradientMaterial {
                                                    gradient: texture.1,
                                                    texture_transform: Mat4::from_mat3(
                                                        Mat3::from_cols_array_2d(&matrix),
                                                    ),
                                                    texture: Some(images.add(texture.0)),
                                                    ..Default::default()
                                                }),
                                            );
                                        }
                                        _ => {}
                                    }
                                }
                                let mut mesh = Mesh::new(
                                    PrimitiveTopology::TriangleList,
                                    RenderAssetUsages::default(),
                                );
                                mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, result_positions);
                                mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, result_colors);
                                mesh.insert_indices(Indices::U32(result_indices));

                                // flip_mesh_vertically(&mut mesh);
                                let mesh_handle = meshes.add(mesh);
                                graphic.set_mesh(mesh_handle);
                            }
                            _ => {}
                        },
                    );
                    swf_movie.movie_library = library;
                    for (entity, ..) in query.iter().filter(|(_, handle)| handle.id() == *id) {
                        commands.entity(entity).insert(Swf {
                            root_movie_clip: root_movie_clip.clone(),
                        });
                    }
                }
            }
            _ => {}
        }
    }
}

fn enter_frame(
    mut query: Query<(&mut Swf, &Handle<SwfMovie>)>,
    mut swf_movies: ResMut<Assets<SwfMovie>>,
    time: Res<Time>,
    mut timer: ResMut<PlayerTimer>,
) {
    if timer.0.tick(time.delta()).just_finished() {
        for (mut swf, swf_handle) in query.iter_mut() {
            if let Some(swf_movie) = swf_movies.get_mut(swf_handle.id()) {
                swf.root_movie_clip
                    .enter_frame(&mut swf_movie.movie_library);
            }
        }
    }
}

/// 线性插值
fn lerp(a: f32, b: f32, factor: f32) -> f32 {
    a + (b - a) * factor
}
