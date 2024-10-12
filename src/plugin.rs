use crate::assets::{SwfLoader, SwfMovie};
use crate::bundle::{Swf, SwfState};
use crate::render::material::{
    BitmapMaterial, GradientMaterial, GradientUniforms, SwfColorMaterial,
};
use crate::render::tessellator::ShapeTessellator;
use crate::render::FlashRenderPlugin;
use crate::swf::characters::Character;
use crate::swf::display_object::movie_clip::MovieClip;
use crate::swf::display_object::TDisplayObject;
use crate::swf::library::MovieLibrary;
use bevy::app::App;
use bevy::asset::{AssetEvent, Handle};
use bevy::color::{Color, ColorToComponents};
use bevy::log::error;
use bevy::math::{Mat3, Mat4};
use bevy::prelude::{
    Entity, Event, EventReader, EventWriter, Image, IntoSystemConfigs, Mesh, Query, Resource,
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
use ruffle_render::tessellator::DrawType;
use swf::GradientInterpolation;
use wgpu::{Extent3d, PrimitiveTopology};

/// 制作多大得渐变纹理，越大细节越丰富，但是内存占用也越大
const GRADIENT_SIZE: usize = 256;

#[derive(Resource)]
struct PlayerTimer(Timer);

pub struct FlashPlugin;

impl Plugin for FlashPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(FlashRenderPlugin)
            .add_event::<SwfInitEvent>()
            .init_asset::<SwfMovie>()
            .init_asset_loader::<SwfLoader>()
            .insert_resource(PlayerTimer(Timer::from_seconds(
                // TODO: 24fps
                1. / 24.,
                TimerMode::Repeating,
            )))
            .add_systems(Update, (pre_parse, enter_frame).chain());
    }
}

fn enter_frame(
    mut query: Query<(&mut Swf, &Handle<SwfMovie>)>,
    mut swf_movies: ResMut<Assets<SwfMovie>>,
    time: Res<Time>,
    mut timer: ResMut<PlayerTimer>,
) {
    query.iter_mut().for_each(|(mut swf, _)| {
        let target = swf.name.clone().unwrap_or("root".to_string());
        if target != swf.root_movie_clip.name().unwrap_or("root") {
            if let Some(target_movie_clip) = swf.root_movie_clip.query_movie_clip(&target) {
                swf.root_movie_clip = target_movie_clip.clone();
            }
        }
    });
    if timer.0.tick(time.delta()).just_finished() {
        for (mut swf, swf_handle) in query.iter_mut() {
            if let Some(swf_movie) = swf_movies.get_mut(swf_handle.id()) {
                swf.root_movie_clip
                    .enter_frame(&mut swf_movie.movie_library);
                swf.status = SwfState::Ready;
            }
        }
    }
}

#[derive(Clone)]
pub enum ShapeDrawType {
    Color(SwfColorMaterial),
    Gradient(GradientMaterial),
    Bitmap(BitmapMaterial),
}
#[derive(Clone)]
pub struct ShapeMesh {
    pub mesh: Handle<Mesh>,
    pub draw_type: ShapeDrawType,
}
#[derive(Event)]
pub struct SwfInitEvent(pub Entity);

fn pre_parse(
    mut query: Query<(&mut Swf, &Handle<SwfMovie>, Entity)>,
    mut swf_events: EventReader<AssetEvent<SwfMovie>>,
    mut swf_movies: ResMut<Assets<SwfMovie>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut swf_init_events: EventWriter<SwfInitEvent>,
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
                    let library_clone = library.clone();
                    library.characters_mut().iter_mut().for_each(
                        |(_id, character)| match character {
                            Character::Graphic(graphic) => {
                                let mut shape_tessellator = ShapeTessellator::new();
                                let lyon_mesh = shape_tessellator
                                    .tessellate_shape((&graphic.shape).into(), &library_clone);

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

                                for draw in lyon_mesh.draws {
                                    match draw.draw_type {
                                        DrawType::Color => {
                                            let mut positions =
                                                Vec::with_capacity(draw.vertices.len());
                                            let mut colors =
                                                Vec::with_capacity(draw.vertices.len());
                                            for vertex in draw.vertices {
                                                // 平移顶点使得中心点在bevy原点
                                                positions.alloc().init([vertex.x, vertex.y, 0.0]);
                                                let linear_color = Color::srgba_u8(
                                                    vertex.color.r,
                                                    vertex.color.g,
                                                    vertex.color.b,
                                                    vertex.color.a,
                                                )
                                                .to_linear();
                                                colors.alloc().init(linear_color.to_f32_array());
                                            }

                                            let mut mesh = Mesh::new(
                                                PrimitiveTopology::TriangleList,
                                                RenderAssetUsages::default(),
                                            );
                                            mesh.insert_attribute(
                                                Mesh::ATTRIBUTE_POSITION,
                                                positions,
                                            );
                                            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
                                            mesh.insert_indices(Indices::U32(draw.indices));

                                            graphic.add_shape_mesh(ShapeMesh {
                                                mesh: meshes.add(mesh),
                                                draw_type: ShapeDrawType::Color(SwfColorMaterial {
                                                    ..Default::default()
                                                }),
                                            });
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
                                            mesh.insert_indices(Indices::U32(draw.indices.clone()));
                                            let texture =
                                                gradients_texture.get(gradient).unwrap().clone();
                                            graphic.add_shape_mesh(ShapeMesh {
                                                mesh: meshes.add(mesh),
                                                draw_type: ShapeDrawType::Gradient(
                                                    GradientMaterial {
                                                        gradient: GradientUniforms {
                                                            focal_point: texture.1.focal_point,
                                                            interpolation: texture.1.interpolation,
                                                            shape: texture.1.shape,
                                                            repeat: texture.1.repeat,
                                                        },
                                                        texture_transform: Mat4::from_mat3(
                                                            Mat3::from_cols_array_2d(&matrix),
                                                        ),
                                                        texture: Some(images.add(texture.0)),
                                                        ..Default::default()
                                                    },
                                                ),
                                            });
                                        }
                                        DrawType::Bitmap(bitmap) => {
                                            let texture_transform = bitmap.matrix;
                                            if let Some(compressed_bitmap) =
                                                library_clone.get_bitmap(bitmap.bitmap_id)
                                            {
                                                let decoded = match compressed_bitmap.decode() {
                                                    Ok(decoded) => decoded,
                                                    Err(e) => {
                                                        error!("Failed to decode bitmap: {:?}", e);
                                                        continue;
                                                    }
                                                };
                                                let bitmap = decoded.to_rgba();

                                                let bitmap_texture = Image::new(
                                                    Extent3d {
                                                        width: bitmap.width(),
                                                        height: bitmap.height(),
                                                        depth_or_array_layers: 1,
                                                    },
                                                    wgpu::TextureDimension::D2,
                                                    bitmap.data().to_vec(),
                                                    wgpu::TextureFormat::Rgba8Unorm,
                                                    RenderAssetUsages::default(),
                                                );

                                                let mut positions =
                                                    Vec::with_capacity(draw.vertices.len());
                                                for vertex in draw.vertices {
                                                    positions
                                                        .alloc()
                                                        .init([vertex.x, vertex.y, 0.0]);
                                                }
                                                let mut mesh = Mesh::new(
                                                    PrimitiveTopology::TriangleList,
                                                    RenderAssetUsages::default(),
                                                );
                                                mesh.insert_attribute(
                                                    Mesh::ATTRIBUTE_POSITION,
                                                    positions,
                                                );
                                                mesh.insert_indices(Indices::U32(
                                                    draw.indices.clone(),
                                                ));
                                                graphic.add_shape_mesh(ShapeMesh {
                                                    mesh: meshes.add(mesh),
                                                    draw_type: ShapeDrawType::Bitmap(
                                                        BitmapMaterial {
                                                            texture: images.add(bitmap_texture),
                                                            texture_transform: Mat4::from_mat3(
                                                                Mat3::from_cols_array_2d(
                                                                    &texture_transform,
                                                                ),
                                                            ),
                                                            ..Default::default()
                                                        },
                                                    ),
                                                });
                                            }
                                        }
                                    }
                                }
                            }
                            _ => {}
                        },
                    );
                    swf_movie.movie_library = library;
                    if let Some((mut swf, _, entity)) =
                        query.iter_mut().find(|(_, handle, _)| handle.id() == *id)
                    {
                        swf.root_movie_clip = root_movie_clip.clone();
                        swf.status = SwfState::Ready;
                        for _ in 0..5 {
                            // 初始化影片剪辑
                            swf.root_movie_clip
                                .enter_frame(&mut swf_movie.movie_library);
                        }
                        swf_init_events.send(SwfInitEvent(entity));
                    }
                }
            }
            _ => {}
        }
    }
}

/// 线性插值
fn lerp(a: f32, b: f32, factor: f32) -> f32 {
    a + (b - a) * factor
}
