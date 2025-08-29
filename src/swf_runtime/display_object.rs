use std::collections::btree_map::ValuesMut;
use std::sync::Arc;

use bevy::asset::{Assets, Handle, RenderAssetUsages};
use bevy::image::Image;
use bevy::log::warn_once;
use bevy::math::{IVec2, Vec3};
use bevy::platform::collections::HashMap;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};
use swf::{BlendMode, CharacterId, ColorTransform, Depth, Rectangle, Twips};

use crate::RenderContext;

use super::character::Character;
use super::graphic::Graphic;
use super::matrix::Matrix;
use super::morph_shape::MorphShape;

use super::tag_utils::SwfMovie;

use super::{filter::Filter, movie_clip::MovieClip, transform::Transform};

pub(crate) type FrameNumber = u16;

#[derive(Debug, Clone, Default)]
pub struct ImageCacheInfo {
    width: u16,
    height: u16,
    handle: Handle<Image>,
}

#[derive(Debug, Clone, Default)]
pub struct ImageCache {
    /// 此缓存上次使用的 `Matrix.a` 值
    matrix_a: f32,
    /// 此缓存上次使用的 `Matrix.b` 值
    matrix_b: f32,
    /// 此缓存上次使用的 `Matrix.c` 值
    matrix_c: f32,
    /// 此缓存上次使用的 `Matrix.d` 值
    matrix_d: f32,

    /// 原始图像的宽度，应用滤镜之前。
    source_width: u16,
    /// 原始图像的高度，应用滤镜之前。
    source_height: u16,

    /// 缓存的图像
    image: Option<ImageCacheInfo>,

    /// 用于绘制最终位图的偏移量（即如果滤镜增加了位图大小的情况下）。
    draw_offset: IVec2,
}

impl ImageCache {
    pub fn is_dirty(&self, other: &Matrix, source_width: u16, source_height: u16) -> bool {
        self.matrix_a != other.a
            || self.matrix_b != other.b
            || self.matrix_c != other.c
            || self.matrix_d != other.d
            || self.source_width != source_width
            || self.source_height != source_height
            || self.image.is_none()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        images: &mut Assets<Image>,
        matrix: &Matrix,
        width: u16,
        height: u16,
        actual_width: u64,
        actual_height: u64,
        swf_version: u8,
        draw_offset: IVec2,
    ) {
        self.matrix_a = matrix.a;
        self.matrix_b = matrix.b;
        self.matrix_c = matrix.c;
        self.matrix_d = matrix.d;
        self.source_width = width;
        self.source_height = height;
        self.draw_offset = draw_offset;

        if let Some(current) = self.image.as_mut() {
            if current.width == width && current.height == height {
                // 缓存命中，不需要重新渲染
                return;
            }
        }
        let acceptable_size = if swf_version > 9 {
            let total = actual_width as u32 * actual_height as u32;
            actual_width < 8191 && actual_height < 8191 && total < 16777216
        } else {
            actual_width < 2880 && actual_height < 2880
        };
        if actual_width > 0 && actual_height > 0 && acceptable_size {
            let mut image = Image::new_fill(
                Extent3d {
                    width: actual_width as u32,
                    height: actual_height as u32,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                &[0, 0, 0, 0],
                TextureFormat::Rgba8Unorm,
                RenderAssetUsages::default(),
            );
            image.texture_descriptor.usage = TextureUsages::TEXTURE_BINDING
                | TextureUsages::COPY_DST
                | TextureUsages::RENDER_ATTACHMENT;
            self.image = Some(ImageCacheInfo {
                width,
                height,
                handle: images.add(image),
            })
        } else {
            self.image = None;
        }
    }

    /// 显式清除缓存值并释放所有资源。
    /// 此操作仅应在无法渲染到缓存且需要暂时禁用缓存的情况下使用。
    pub fn clear(&mut self) {
        self.image = None;
    }

    pub fn handle(&self) -> Option<Handle<Image>> {
        self.image.as_ref().map(|i| i.handle.clone())
    }
}

#[derive(Debug, Clone, Default)]
pub struct DisplayObjectBase {
    name: Option<Box<str>>,
    place_frame: FrameNumber,
    depth: Depth,
    transform: Transform,
    filters: Vec<Filter>,
    blend_mode: BlendMode,
    cache: Option<ImageCache>,
}
impl DisplayObjectBase {
    fn set_matrix(&mut self, matrix: Matrix) {
        self.transform.matrix = matrix;
    }

    pub fn set_color_transform(&mut self, color_transform: ColorTransform) {
        self.transform.color_transform = color_transform;
    }

    pub fn set_blend_mode(&mut self, blend_mode: BlendMode) {
        self.blend_mode = blend_mode;
    }

    fn set_filters(&mut self, filters: Vec<Filter>) -> bool {
        if filters != self.filters {
            self.filters = filters;
            self.recheck_cache();
            true
        } else {
            false
        }
    }

    fn recheck_cache(&mut self) {
        if !self.filters.is_empty() && self.cache.is_none() {
            self.cache = Some(ImageCache::default());
        }
    }

    pub fn cache_mut(&mut self) -> Option<&mut ImageCache> {
        self.cache.as_mut()
    }

    fn set_name(&mut self, name: Option<Box<str>>) {
        self.name = name;
    }

    pub fn transform(&self) -> &Transform {
        &self.transform
    }

    pub fn filters(&self) -> Vec<Filter> {
        self.filters.clone()
    }

    pub fn blend_mode(&self) -> BlendMode {
        self.blend_mode
    }
}

pub(crate) trait TDisplayObject: Clone + Into<DisplayObject> {
    fn base(&self) -> &DisplayObjectBase;
    fn base_mut(&mut self) -> &mut DisplayObjectBase;

    fn set_place_frame(&mut self, frame: u16) {
        self.base_mut().place_frame = frame;
    }

    fn set_depth(&mut self, depth: Depth) {
        self.base_mut().depth = depth;
    }

    fn set_matrix(&mut self, matrix: Matrix) {
        self.base_mut().set_matrix(matrix);
    }

    fn set_color_transform(&mut self, color_transform: ColorTransform) {
        self.base_mut().set_color_transform(color_transform);
    }

    fn set_blend_mode(&mut self, blend_mode: BlendMode) {
        self.base_mut().set_blend_mode(blend_mode);
    }

    fn set_filters(&mut self, filters: Vec<Filter>) {
        self.base_mut().set_filters(filters);
    }

    fn set_name(&mut self, name: Option<Box<str>>) {
        self.base_mut().set_name(name);
    }

    fn movie(&self) -> Arc<SwfMovie>;

    fn id(&self) -> CharacterId;

    fn depth(&self) -> Depth {
        self.base().depth
    }

    fn place_frame(&self) -> FrameNumber {
        self.base().place_frame
    }

    fn filters(&self) -> Vec<Filter> {
        self.base().filters()
    }

    fn blend_mode(&self) -> BlendMode {
        self.base().blend_mode()
    }

    fn matrix(&self) -> &Matrix {
        &self.base().transform().matrix
    }

    fn swf_version(&self) -> u8 {
        self.movie().version()
    }

    /// 该对象未经过变换的固有边界框。这些边界不包含子显示对象（DisplayObjects）。
    /// 叶节点显示对象应返回自身的边界。仅包含子对象的复合显示对象应返回 &Default::default ()。
    fn self_bounds(&mut self) -> Rectangle<Twips>;

    /// 获取此对象及其所有子对象的渲染边界。该边界与向 Flash 暴露的边界主要有两点不同：
    /// - 若应用了会增大显示内容尺寸的滤镜，渲染边界可能会更大
    /// - 不考虑滚动矩形
    fn render_bounds_with_transform(
        &mut self,
        matrix: &Matrix,
        include_own_filters: bool,
        scale: Vec3,
    ) -> Rectangle<Twips> {
        let mut bounds = *matrix * self.self_bounds();
        if let Some(children) = self.children_mut() {
            for child in children {
                let matrix = *matrix * *child.matrix();
                bounds = bounds.union(&child.render_bounds_with_transform(
                    &matrix,
                    include_own_filters,
                    scale,
                ));
            }
        }

        if include_own_filters {
            let filters = self.base().filters();
            for mut filter in filters {
                filter.scale(scale.x, scale.y);
                bounds = filter.calculate_dest_rect(bounds);
            }
        }
        bounds
    }

    fn apply_place_object(&mut self, place_object: &swf::PlaceObject, version: u8) {
        if let Some(matrix) = place_object.matrix {
            self.set_matrix(matrix.into());
        }
        if let Some(color_transform) = &place_object.color_transform {
            self.set_color_transform(*color_transform);
        }
        if let Some(ratio) = place_object.ratio {
            if let Some(morph_shape) = self.as_morph_shape() {
                morph_shape.set_ratio(ratio);
            }
        }
        if let Some(blend_mode) = place_object.blend_mode {
            self.set_blend_mode(blend_mode);
        }
        if let Some(_is_bitmap_cached) = place_object.is_bitmap_cached {
            //TODO:
            warn_once!(
                "is_bitmap_cached is not supported. id: {}. TODO!",
                self.id()
            );
        }
        if version >= 11 {
            if let Some(_visible) = place_object.is_visible {
                //TODO:
                warn_once!("visible is not supported. id: {}. TODO!", self.id());
            }
            if let Some(_color) = place_object.background_color {
                //TODO:
                warn_once!(
                    "background_color is not supported. id: {}. TODO!",
                    self.id()
                );
            }
        }
        if let Some(filters) = &place_object.filters {
            self.set_filters(filters.iter().map(Filter::from).collect())
        }
    }

    fn enter_frame(&mut self, _characters: &HashMap<u16, Character>) {}

    fn render_self(
        &mut self,
        _context: &mut RenderContext,
        _blend_mode: BlendMode,
        _shape_depth_layer: String,
    ) {
    }

    fn replace_with(&mut self, _id: CharacterId, _characters: &HashMap<CharacterId, Character>) {}

    fn children_mut(&mut self) -> Option<ValuesMut<'_, u16, DisplayObject>> {
        None
    }

    fn as_morph_shape(&mut self) -> Option<&mut MorphShape> {
        None
    }
}

#[derive(Debug, Clone)]
pub enum DisplayObject {
    Graphic(Graphic),
    MovieClip(MovieClip),
    MorphShape(MorphShape),
}

impl TDisplayObject for DisplayObject {
    fn base(&self) -> &DisplayObjectBase {
        match self {
            Self::Graphic(g) => g.base(),
            Self::MovieClip(m) => m.base(),
            Self::MorphShape(m) => m.base(),
        }
    }

    fn base_mut(&mut self) -> &mut DisplayObjectBase {
        match self {
            Self::Graphic(g) => g.base_mut(),
            Self::MovieClip(m) => m.base_mut(),
            Self::MorphShape(m) => m.base_mut(),
        }
    }

    fn movie(&self) -> Arc<SwfMovie> {
        match self {
            Self::Graphic(g) => g.movie(),
            Self::MovieClip(m) => m.movie(),
            Self::MorphShape(m) => m.movie(),
        }
    }

    fn enter_frame(&mut self, characters: &HashMap<CharacterId, Character>) {
        match self {
            Self::Graphic(g) => g.enter_frame(characters),
            Self::MovieClip(m) => m.enter_frame(characters),
            Self::MorphShape(m) => m.enter_frame(characters),
        }
    }

    fn replace_with(&mut self, id: CharacterId, characters: &HashMap<CharacterId, Character>) {
        match self {
            Self::Graphic(g) => g.replace_with(id, characters),
            Self::MovieClip(m) => m.replace_with(id, characters),
            Self::MorphShape(m) => m.replace_with(id, characters),
        }
    }

    fn self_bounds(&mut self) -> Rectangle<Twips> {
        match self {
            Self::Graphic(g) => g.self_bounds(),
            Self::MovieClip(m) => m.self_bounds(),
            Self::MorphShape(m) => m.self_bounds(),
        }
    }

    fn children_mut(&mut self) -> Option<ValuesMut<'_, u16, DisplayObject>> {
        match self {
            Self::MovieClip(m) => m.children_mut(),
            _ => None,
        }
    }

    fn id(&self) -> CharacterId {
        match self {
            Self::Graphic(g) => g.id(),
            Self::MovieClip(m) => m.id(),
            Self::MorphShape(m) => m.id(),
        }
    }

    fn as_morph_shape(&mut self) -> Option<&mut MorphShape> {
        match self {
            Self::MorphShape(m) => Some(m),
            _ => None,
        }
    }
}
