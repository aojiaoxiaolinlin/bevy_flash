use crate::assets::{SwfLoader, SwfMovie};
use crate::bundle::{FlashAnimation, SwfState};
use crate::render::FlashRenderPlugin;
use crate::render::material::{
    BitmapMaterial, GradientMaterial, GradientUniforms, SwfColorMaterial,
};
use crate::render::tessellator::ShapeTessellator;
use crate::swf::characters::Character;
use crate::swf::display_object::TDisplayObject;
use ::swf::GradientInterpolation;
use bevy::app::App;
use bevy::asset::AssetEvent;
use bevy::color::{Color, ColorToComponents};
use bevy::ecs::schedule::IntoScheduleConfigs;
use bevy::log::error;
use bevy::math::{Mat3, Mat4};
use bevy::prelude::{Entity, Event, EventReader, EventWriter, Image, Query, Resource};
use bevy::render::render_asset::RenderAssetUsages;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::time::{Time, Timer, TimerMode};
use bevy::{
    app::{Plugin, Update},
    asset::{AssetApp, Assets},
    prelude::{Res, ResMut},
};
use copyless::VecHelper;
use ruffle_render::tessellator::DrawType;

pub mod assets;
pub mod bundle;
mod render;
pub mod swf;

/// 制作多大得渐变纹理，越大细节越丰富，但是内存占用也越大
const GRADIENT_SIZE: usize = 256;

#[derive(Resource)]
pub struct FlashPlayerTimer(Timer);

impl FlashPlayerTimer {
    pub fn from_frame_rate(frame_rate: f32) -> Self {
        Self(Timer::from_seconds(1.0 / frame_rate, TimerMode::Repeating))
    }
}

pub struct FlashPlugin;

impl Plugin for FlashPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(FlashRenderPlugin)
            .add_event::<SwfInitEvent>()
            .init_asset::<SwfMovie>()
            .init_asset_loader::<SwfLoader>()
            .add_systems(Update, (pre_parse, enter_frame).chain());
    }
}

fn enter_frame(
    mut query: Query<&mut FlashAnimation>,
    mut swf_movies: ResMut<Assets<SwfMovie>>,
    time: Res<Time>,
    mut timer: ResMut<FlashPlayerTimer>,
) {
    if timer.0.tick(time.delta()).just_finished() {
        for mut flash_animation in query.iter_mut() {
            if let Some(swf_movie) = swf_movies.get_mut(flash_animation.swf_movie.id()) {
                swf_movie.movie_clip.enter_frame(&mut swf_movie.library);
                flash_animation.status = SwfState::Ready;
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
pub struct SwfMesh {
    pub positions: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
    pub colors: Vec<[f32; 4]>,
}

#[derive(Clone)]
pub struct ShapeMesh {
    pub mesh: SwfMesh,
    pub draw_type: ShapeDrawType,
}

#[derive(Event)]
pub struct SwfInitEvent(pub Entity);

fn pre_parse(
    mut query: Query<(&mut FlashAnimation, Entity)>,
    mut swf_events: EventReader<AssetEvent<SwfMovie>>,
    mut swf_movies: ResMut<Assets<SwfMovie>>,
    mut images: ResMut<Assets<Image>>,
    mut swf_init_events: EventWriter<SwfInitEvent>,
) {
    for event in swf_events.read() {
        if let AssetEvent::LoadedWithDependencies { id } = event {
            if let Some(swf_movie) = swf_movies.get_mut(*id) {
                let bitmap_library = swf_movie.library.get_bitmap_characters();
                swf_movie
                    .library
                    .characters_mut()
                    .iter_mut()
                    .for_each(|(_id, character)| {
                        if let Character::Graphic(graphic) = character {
                            let mut shape_tessellator = ShapeTessellator::new();
                            let lyon_mesh = shape_tessellator
                                .tessellate_shape((&graphic.shape).into(), &bitmap_library);

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
                                        for (i, record) in gradient.records.iter().enumerate().rev()
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
                                    Extent3d {
                                        width: GRADIENT_SIZE as u32,
                                        height: 1,
                                        depth_or_array_layers: 1,
                                    },
                                    TextureDimension::D2,
                                    colors,
                                    TextureFormat::Rgba8Unorm,
                                    RenderAssetUsages::default(),
                                );

                                let gradient_uniforms = GradientUniforms::from(gradient);
                                gradients_texture.push((texture.clone(), gradient_uniforms));
                            }

                            for draw in lyon_mesh.draws {
                                match draw.draw_type {
                                    DrawType::Color => {
                                        let mut positions = Vec::with_capacity(draw.vertices.len());
                                        let mut colors = Vec::with_capacity(draw.vertices.len());
                                        for vertex in draw.vertices {
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

                                        graphic.add_shape_mesh(ShapeMesh {
                                            mesh: SwfMesh {
                                                positions,
                                                indices: draw.indices,
                                                colors,
                                            },
                                            draw_type: ShapeDrawType::Color(
                                                SwfColorMaterial::default(),
                                            ),
                                        });
                                    }
                                    DrawType::Gradient { matrix, gradient } => {
                                        let mut positions = Vec::with_capacity(draw.vertices.len());
                                        for vertex in draw.vertices {
                                            positions.alloc().init([vertex.x, vertex.y, 0.0]);
                                        }

                                        let texture =
                                            gradients_texture.get(gradient).unwrap().clone();
                                        graphic.add_shape_mesh(ShapeMesh {
                                            mesh: SwfMesh {
                                                positions,
                                                indices: draw.indices,
                                                colors: Vec::new(),
                                            },
                                            draw_type: ShapeDrawType::Gradient(GradientMaterial {
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
                                            }),
                                        });
                                    }
                                    DrawType::Bitmap(bitmap) => {
                                        let texture_transform = bitmap.matrix;
                                        if let Some(compressed_bitmap) =
                                            bitmap_library.get(&bitmap.bitmap_id)
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
                                                TextureDimension::D2,
                                                bitmap.data().to_vec(),
                                                TextureFormat::Rgba8Unorm,
                                                RenderAssetUsages::default(),
                                            );

                                            let mut positions =
                                                Vec::with_capacity(draw.vertices.len());
                                            for vertex in draw.vertices {
                                                positions.alloc().init([vertex.x, vertex.y, 0.0]);
                                            }

                                            graphic.add_shape_mesh(ShapeMesh {
                                                mesh: SwfMesh {
                                                    positions,
                                                    indices: draw.indices,
                                                    colors: Vec::new(),
                                                },
                                                draw_type: ShapeDrawType::Bitmap(BitmapMaterial {
                                                    texture: images.add(bitmap_texture),
                                                    texture_transform: Mat4::from_mat3(
                                                        Mat3::from_cols_array_2d(
                                                            &texture_transform,
                                                        ),
                                                    ),
                                                    ..Default::default()
                                                }),
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    });
                if let Some((mut flash_animation, entity)) = query
                    .iter_mut()
                    .find(|(flash_animation, _)| flash_animation.swf_movie.id() == *id)
                {
                    if flash_animation.ignore_root_swf_transform {
                        // 这里设置当前影片剪辑的根影片剪辑时，在MovieClip的实例化中就不会应用根影片的变换
                        // 如果后续根影片无其他作用，这里可以更改为更加语义化的方法名
                        swf_movie.movie_clip.set_root();
                    }
                    flash_animation.status = SwfState::Ready;
                    swf_init_events.write(SwfInitEvent(entity));
                }
            }
        }
    }
}

/// 线性插值
fn lerp(a: f32, b: f32, factor: f32) -> f32 {
    a + (b - a) * factor
}
