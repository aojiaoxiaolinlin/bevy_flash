use std::sync::Arc;

use bevy::{
    asset::{Asset, AssetLoader, AssetPath, Handle, LoadContext, RenderAssetUsages, io::Reader},
    color::{Color, ColorToComponents},
    image::Image,
    log::error,
    math::{Mat3, Mat4},
    mesh::{Indices, Mesh, PrimitiveTopology},
    platform::collections::HashMap,
    reflect::TypePath,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use copyless::VecHelper;
use swf::{CharacterId, GradientInterpolation};

use crate::{
    render::material::{BitmapMaterial, ColorMaterial, GradientMaterial, GradientUniforms},
    swf_runtime::{
        character::{BitmapLibrary, Character},
        display_object::FrameNumber,
        graphic::Graphic,
        movie_clip::MovieClip,
        tag_utils::{self, SwfMovie},
        tessellator::{DrawType, Gradient, ShapeTessellator},
    },
};

/// 制作多大得渐变纹理，越大细节越丰富，但是内存占用也越大
const GRADIENT_SIZE: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwfAssetLabel {
    MC(CharacterId),
}

impl std::fmt::Display for SwfAssetLabel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SwfAssetLabel::MC(id) => f.write_str(&format!("MC{id}")),
        }
    }
}

impl SwfAssetLabel {
    pub fn from_asset(&self, path: impl Into<AssetPath<'static>>) -> AssetPath<'static> {
        path.into().with_label(self.to_string())
    }
}

#[derive(Asset, TypePath)]
pub struct SwfMC {}

/// SWF 资产结构体，包含了 SWF 文件的相关信息。
#[derive(Asset, TypePath)]
pub struct Swf {
    pub shape_mesh_materials: HashMap<CharacterId, Vec<(ShapeMaterialType, Handle<Mesh>)>>,
    pub library: MovieLibrary,
    /// 动画名称，以及动画的起始帧和总帧长
    pub animations: HashMap<Box<str>, (FrameNumber, FrameNumber)>,
    pub frame_events: HashMap<FrameNumber, Box<str>>,
    pub swf_movie: Arc<SwfMovie>,
}

impl Swf {
    pub fn animations(&self) -> &HashMap<Box<str>, (FrameNumber, FrameNumber)> {
        &self.animations
    }

    pub fn frame_events(&self) -> &HashMap<FrameNumber, Box<str>> {
        &self.frame_events
    }

    pub fn characters(&self) -> &HashMap<CharacterId, Character> {
        &self.library.characters
    }
}

#[derive(Default, Asset, TypePath)]
pub struct MovieLibrary {
    characters: HashMap<CharacterId, Character>,
    export_characters: HashMap<String, CharacterId>,
}

impl MovieLibrary {
    pub fn characters_mut(&mut self) -> &mut HashMap<CharacterId, Character> {
        &mut self.characters
    }
    pub fn export_characters_mut(&mut self) -> &mut HashMap<String, CharacterId> {
        &mut self.export_characters
    }
}

#[derive(Default)]
pub(crate) struct SwfLoader;

impl AssetLoader for SwfLoader {
    type Asset = Swf;

    type Settings = ();

    type Error = tag_utils::Error;
    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &(),
        load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut swf_data = Vec::new();
        reader.read_to_end(&mut swf_data).await?;
        let swf_movie = Arc::new(SwfMovie::from_data(&swf_data)?);
        let mut root = MovieClip::new(swf_movie.clone());
        let mut library = MovieLibrary::default();

        // 解析定义的资源
        let mut bitmaps = HashMap::new();
        let mut jpeg_tables = None;
        root.preload(&mut library, &mut bitmaps, &mut jpeg_tables);

        let mut mesh_index = 0;
        let mut image_index = 0;
        let mut material_index = 0;
        let mut shape_mesh_materials = HashMap::new();
        library.characters.values_mut().for_each(|v| {
            if let Character::Graphic(graphic) = v {
                shape_mesh_materials.insert(
                    graphic.id(),
                    load_shape_mesh(
                        load_context,
                        graphic,
                        &bitmaps,
                        &mut image_index,
                        &mut mesh_index,
                        &mut material_index,
                    ),
                );
                // 生成Mesh 后清除图形记录数据，后续不在需要。
                graphic.shape_mut().shape.clear();
            }
        });
        // 加载子资源
        library.export_characters.values().for_each(|v| {
            if let Character::MovieClip(mc) = library.characters.get(v).unwrap() {}
        });

        let mut animations = <HashMap<_, _>>::default();
        let mut frame_events = <HashMap<_, _>>::default();
        root.frame_labels().iter().for_each(|(k, v)| {
            if let Some(anim_name) = k.strip_prefix("anim_") {
                animations.insert(anim_name.into(), (*v, 0));
            } else if let Some(event_name) = k.strip_prefix("event_") {
                frame_events.insert(*v, event_name.into());
            } else {
                animations.insert(k.clone(), (*v, 0));
            }
        });
        // 根据animations 的 起始帧v.0 的值，使用第一个大于当前项的v.0减去当前项的v.0，得到动画的长度。
        if !animations.is_empty() {
            let mut anim_frames = animations.values_mut().collect::<Vec<_>>();
            anim_frames.sort_by_key(|(start, _)| *start);
            for i in 0..anim_frames.len() - 1 {
                let (start, _) = *anim_frames[i];
                let (end, _) = *anim_frames[i + 1];
                let len = end - start;
                anim_frames[i].1 = len;
            }
            let last: usize = anim_frames.len() - 1;
            anim_frames[last].1 = root.total_frames() - anim_frames[last].0;
        }
        Ok(Swf {
            shape_mesh_materials,
            library,
            animations,
            frame_events,
            swf_movie,
        })
    }

    fn extensions(&self) -> &[&str] {
        &["swf"]
    }
}

fn load_shape_mesh(
    load_context: &mut LoadContext,
    graphic: &Graphic,
    bitmaps: &BitmapLibrary,
    image_index: &mut usize,
    mesh_index: &mut usize,
    _material_index: &mut usize,
) -> Vec<(ShapeMaterialType, Handle<Mesh>)> {
    let mut tessellator = ShapeTessellator::default();
    let shape = graphic.shape();
    let lyon_mesh = tessellator.tessellate_shape(shape.into(), bitmaps);

    let gradient_texture = load_gradient_textures(lyon_mesh.gradients, load_context, image_index);

    let mut shape_mesh_material = Vec::new();
    let draws = lyon_mesh.draws;
    for draw in draws {
        match &draw.draw_type {
            DrawType::Color => {
                let mut positions = Vec::with_capacity(draw.vertices.len());
                let mut colors = Vec::with_capacity(draw.vertices.len());
                for vertex in &draw.vertices {
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

                let mesh = Mesh::new(
                    PrimitiveTopology::TriangleList,
                    RenderAssetUsages::default(),
                )
                .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
                .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
                .with_inserted_indices(Indices::U32(draw.indices.into_iter().collect()));
                let mesh = load_context.add_labeled_asset(format!("mesh_{mesh_index}"), mesh);
                *mesh_index += 1;
                // let material = load_context.add_labeled_asset(
                //     format!("material_{}", material_index),
                //     ColorMaterial::default(),
                // );
                // *material_index += 1;
                // shape_mesh_material.push((ShapeMaterialType::Color(material), mesh));
                shape_mesh_material
                    .push((ShapeMaterialType::Color(ColorMaterial::default()), mesh));
            }
            DrawType::Gradient { matrix, gradient } => {
                let Some((handle, gradient)) = gradient_texture.get(*gradient).cloned() else {
                    continue;
                };
                let mut positions = Vec::with_capacity(draw.vertices.len());
                for vertex in &draw.vertices {
                    positions.alloc().init([vertex.x, vertex.y, 0.0]);
                }
                let mesh = Mesh::new(
                    PrimitiveTopology::TriangleList,
                    RenderAssetUsages::default(),
                )
                .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
                .with_inserted_indices(Indices::U32(draw.indices.into_iter().collect()));
                let mesh = load_context.add_labeled_asset(format!("mesh_{mesh_index}"), mesh);
                *mesh_index += 1;

                // let material = load_context.add_labeled_asset(
                //     format!("material_{}", material_index),
                //     GradientMaterial {
                //         gradient,
                //         texture: handle,
                //         texture_transform: Mat4::from_mat3(Mat3::from_cols_array_2d(&matrix)),
                //         ..Default::default()
                //     },
                // );
                // *material_index += 1;
                // shape_mesh_material.push((ShapeMaterialType::Gradient(material), mesh));
                shape_mesh_material.push((
                    ShapeMaterialType::Gradient(GradientMaterial {
                        gradient,
                        texture: handle,
                        texture_transform: Mat4::from_mat3(Mat3::from_cols_array_2d(matrix)),
                        ..Default::default()
                    }),
                    mesh,
                ));
            }
            DrawType::Bitmap(bitmap) => {
                let mut positions = Vec::with_capacity(draw.vertices.len());
                for vertex in &draw.vertices {
                    positions.alloc().init([vertex.x, vertex.y, 0.0]);
                }

                let mesh = Mesh::new(
                    PrimitiveTopology::TriangleList,
                    RenderAssetUsages::default(),
                )
                .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
                .with_inserted_indices(Indices::U32(draw.indices.into_iter().collect()));
                let mesh = load_context.add_labeled_asset(format!("mesh_{mesh_index}"), mesh);
                *mesh_index += 1;

                let texture_transform = bitmap.matrix;
                if let Some(compressed_bitmap) = bitmaps.get(&bitmap.bitmap_id) {
                    let decoded = match compressed_bitmap.decode() {
                        Ok(decoded) => decoded,
                        Err(e) => {
                            error!("Failed to decode bitmap: {:?}", e);
                            continue;
                        }
                    };
                    let bitmap = decoded.into_rgba();
                    let texture = Image::new(
                        Extent3d {
                            width: bitmap.width(),
                            height: bitmap.height(),
                            depth_or_array_layers: 1,
                        },
                        TextureDimension::D2,
                        bitmap.data().to_vec(),
                        TextureFormat::Rgba8UnormSrgb,
                        RenderAssetUsages::RENDER_WORLD,
                    );
                    let texture =
                        load_context.add_labeled_asset(format!("texture_{image_index}"), texture);
                    *image_index += 1;
                    // let material = load_context.add_labeled_asset(
                    //     format!("material_{}", material_index),
                    //     BitmapMaterial {
                    //         texture,
                    //         texture_transform: Mat4::from_mat3(Mat3::from_cols_array_2d(
                    //             &texture_transform,
                    //         )),
                    //         ..Default::default()
                    //     },
                    // );
                    // *material_index += 1;
                    // shape_mesh_material.push((ShapeMaterialType::Bitmap(material), mesh));
                    shape_mesh_material.push((
                        ShapeMaterialType::Bitmap(BitmapMaterial {
                            texture,
                            texture_transform: Mat4::from_mat3(Mat3::from_cols_array_2d(
                                &texture_transform,
                            )),
                            ..Default::default()
                        }),
                        mesh,
                    ));
                }
            }
        }
    }
    shape_mesh_material
}

fn load_gradient_textures(
    gradients: Vec<Gradient>,
    load_context: &mut LoadContext,
    i: &mut usize,
) -> Vec<(Handle<Image>, GradientUniforms)> {
    let mut gradient_textures = Vec::new();
    for (texture, gradient_uniforms) in create_gradient_textures(gradients) {
        let handle = load_context.add_labeled_asset(format!("gradient_{i}"), texture);
        *i += 1;
        gradient_textures.push((handle, gradient_uniforms));
    }
    gradient_textures
}

pub fn create_gradient_textures(gradients: Vec<Gradient>) -> Vec<(Image, GradientUniforms)> {
    let mut gradient_textures = Vec::new();
    for gradient in gradients {
        let colors = if gradient.records.is_empty() {
            vec![0; GRADIENT_SIZE * 4]
        } else {
            let mut colors = vec![0; GRADIENT_SIZE * 4];
            let convert = if gradient.interpolation == GradientInterpolation::LinearRgb {
                println!("线性");
                |color| color
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
            Extent3d {
                width: GRADIENT_SIZE as u32,
                height: 1,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            colors,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD,
        );
        let gradient_uniforms = GradientUniforms::from(gradient);
        gradient_textures.push((texture, gradient_uniforms));
    }
    gradient_textures
}

/// 线性插值
fn lerp(a: f32, b: f32, factor: f32) -> f32 {
    a + (b - a) * factor
}

/// TODO: 只有实现Skew组件这里才能使用Handle应用材质。
/// 不然当Shape被多次引用时，会导致SwfTransform修改同一个材质。
#[derive(Debug, Clone, Asset, TypePath)]
pub enum ShapeMaterialType {
    // 待实现Skew组件后这里使用Handle处理
    // Color(Handle<ColorMaterial>),
    // Gradient(Handle<GradientMaterial>),
    // Bitmap(Handle<BitmapMaterial>),
    /// 颜色
    Color(ColorMaterial),
    /// 渐变
    Gradient(GradientMaterial),
    /// 位图
    Bitmap(BitmapMaterial),
}
