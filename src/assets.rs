use std::sync::Arc;

use bevy::{
    asset::{io::Reader, Asset, AssetLoader, AsyncReadExt, LoadContext},
    color::{Color, ColorToComponents, Srgba},
    prelude::{Image, Mesh},
    reflect::TypePath,
    render::{
        mesh::{Indices, VertexAttributeValues},
        render_asset::RenderAssetUsages,
    },
    utils::dbg,
};
use copyless::VecHelper;
use glam::Vec2;
use ruffle_render::tessellator::DrawType;
use swf::GradientInterpolation;
use wgpu::PrimitiveTopology;

use crate::{
    render::tessellator::ShapeTessellator,
    swf::{
        characters::Character, display_object::movie_clip::MovieClip, library::MovieLibrary,
        tag_utils,
    },
};

/// 制作多大得渐变纹理，越大细节越丰富，但是内存占用也越大
const GRADIENT_SIZE: usize = 256;

#[derive(Asset, TypePath)]
pub struct SwfMovie {
    pub library: MovieLibrary,
    pub root_movie_clip: MovieClip,
}

impl SwfMovie {
    pub fn register_shape<'a>(library: &mut MovieLibrary, load_context: &'a mut LoadContext<'_>) {
        library
            .characters_mut()
            .iter_mut()
            .for_each(|(_, character)| match character {
                Character::Graphic(graphic) => {
                    let mut shape_tessellator = ShapeTessellator::new();
                    let lyon_mesh = shape_tessellator.tessellate_shape((&graphic.shape).into());

                    let gradients = lyon_mesh.gradients;
                    for gradient in gradients {
                        let colors = if gradient.records.is_empty() {
                            vec![0; GRADIENT_SIZE * 4]
                        } else {
                            dbg!(gradient.records.len());
                            let mut colors = vec![0; GRADIENT_SIZE * 4];
                            let convert =
                                if gradient.interpolation == GradientInterpolation::LinearRgb {
                                    |color| Srgba::gamma_function(color / 255.0) * 255.0
                                } else {
                                    |color| color
                                };

                            for t in 0..GRADIENT_SIZE {
                                let mut last = 0;
                                let mut next = 0;
                                for (i, record) in gradient.records.iter().enumerate().rev() {
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
                                        / (next_record.ratio as f32 - last_record.ratio as f32)
                                };

                                colors[t * 4] = lerp(
                                    convert(last_record.color.r as f32),
                                    convert(next_record.color.r as f32),
                                    factor,
                                ) as u8;
                                colors[(t * 4) + 1] = lerp(
                                    convert(last_record.color.g as f32),
                                    convert(next_record.color.g as f32),
                                    factor,
                                ) as u8;
                                colors[(t * 4) + 2] = lerp(
                                    convert(last_record.color.b as f32),
                                    convert(next_record.color.b as f32),
                                    factor,
                                ) as u8;
                                colors[(t * 4) + 3] = lerp(
                                    last_record.color.a as f32,
                                    next_record.color.a as f32,
                                    factor,
                                ) as u8;
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
                        graphic.set_texture(
                            load_context.add_labeled_asset(String::from("texture"), texture),
                        );
                    }

                    let center: Vec2 = Vec2::new(
                        (graphic.bounds.x_max + graphic.bounds.x_min).to_pixels() as f32 / 2.,
                        (graphic.bounds.y_max + graphic.bounds.y_min).to_pixels() as f32 / 2.,
                    );

                    for draw in lyon_mesh.draws {
                        dbg("draw");
                        if matches!(draw.draw_type, DrawType::Color) {
                            let mut positions = Vec::with_capacity(draw.vertices.len());
                            let mut colors = Vec::with_capacity(draw.vertices.len());

                            for vertex in draw.vertices {
                                // 平移顶点使得中心点在bevy原点
                                positions.alloc().init([
                                    vertex.x - center.x,
                                    vertex.y - center.y,
                                    0.0,
                                ]);

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
                            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
                            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
                            mesh.insert_indices(Indices::U32(draw.indices));
                            flip_mesh_vertically(&mut mesh);
                            graphic.set_mesh(
                                load_context.add_labeled_asset(String::from("mesh"), mesh),
                            );
                        }
                    }
                }

                _ => {}
            });
    }
}

/// Bevy 有一个不同的y轴原点，所以我们需要翻转y坐标
fn flip_mesh_vertically(mesh: &mut Mesh) {
    if let Some(VertexAttributeValues::Float32x3(positions)) =
        mesh.attribute_mut(Mesh::ATTRIBUTE_POSITION)
    {
        for position in positions.iter_mut() {
            // Invert the y-coordinate to flip the mesh vertically
            position[1] = -position[1];
        }
    }
}
/// 线性插值
fn lerp(a: f32, b: f32, factor: f32) -> f32 {
    a + (b - a) * factor
}
#[derive(Default)]
pub(crate) struct SwfLoader;

impl AssetLoader for SwfLoader {
    type Asset = SwfMovie;

    type Settings = ();

    type Error = tag_utils::Error;
    async fn load<'a>(
        &'a self,
        reader: &'a mut Reader<'_>,
        _settings: &'a (),
        load_context: &'a mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut swf_data = Vec::new();
        reader.read_to_end(&mut swf_data).await?;
        let swf_movie = Arc::new(tag_utils::SwfMovie::from_data(&swf_data[..])?);
        let mut root_movie_clip: MovieClip = MovieClip::new(swf_movie.clone());
        let mut library = MovieLibrary::new();
        root_movie_clip.parse_swf(&mut library);
        SwfMovie::register_shape(&mut library, load_context);
        Ok(SwfMovie {
            library,
            root_movie_clip,
        })
    }

    fn extensions(&self) -> &[&str] {
        &["swf"]
    }
}

#[derive(Debug, Clone, Copy)]
pub enum FlashRunFrameStatus {
    Running,
    Stop,
}
