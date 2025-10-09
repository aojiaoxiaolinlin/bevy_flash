use std::collections::btree_map::ValuesMut;
use std::sync::Arc;

use bevy::asset::{Assets, Handle, RenderAssetUsages};
use bevy::image::Image;
use bevy::log::warn_once;
use bevy::math::{IVec2, UVec2};
use bevy::platform::collections::HashMap;
use bevy::render::render_resource::TextureFormat;
use swf::{BlendMode, CharacterId, ColorTransform, Depth, Rectangle, Twips};

use crate::RenderContext;

use super::character::Character;
use super::graphic::Graphic;
use super::matrix::Matrix;
use super::morph_shape::MorphShape;

use super::tag_utils::SwfMovie;

use super::{filter::Filter, movie_clip::MovieClip, transform::Transform};

pub(crate) type FrameNumber = u16;

type ImageCaches = HashMap<(u16, u16), Handle<Image>>;

#[derive(Debug, Clone, Default)]
pub struct ImageCacheInfo {
    width: u16,
    height: u16,
    caches: ImageCaches,
}
impl ImageCacheInfo {
    pub fn handle(&self) -> Handle<Image> {
        self.caches.get(&(self.width, self.height)).unwrap().clone()
    }
    pub fn size(&self) -> UVec2 {
        UVec2::new(self.width as u32, self.height as u32)
    }
    pub fn has_cache(&mut self, width: u16, height: u16) -> bool {
        if self.caches.contains_key(&(width, height)) {
            // 缓存命中
            self.width = width;
            self.height = height;
            true
        } else {
            false
        }
    }
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

    dirty: bool,
}

impl ImageCache {
    pub fn is_dirty(&mut self, other: &Matrix, source_width: u16, source_height: u16) -> bool {
        self.dirty = self.matrix_a != other.a
            || self.matrix_b != other.b
            || self.matrix_c != other.c
            || self.matrix_d != other.d
            || self.source_width != source_width
            || self.source_height != source_height
            || self.image.is_none();
        self.dirty
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        images: &mut Assets<Image>,
        matrix: &Matrix,
        width: u16,
        height: u16,
        actual_width: u16,
        actual_height: u16,
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

        if let Some(current) = self.image.as_mut()
            && current.has_cache(actual_width, actual_height)
        {
            // 缓存命中，不需要重新渲染
            return;
        }
        let acceptable_size = if swf_version > 9 {
            let total = actual_width as u32 * actual_height as u32;
            actual_width < 8191 && actual_height < 8191 && total < 16777216
        } else {
            actual_width < 2880 && actual_height < 2880
        };
        if actual_width > 0 && actual_height > 0 && acceptable_size {
            let mut image = Image::new_target_texture(
                actual_width as u32,
                actual_height as u32,
                TextureFormat::Rgba8Unorm,
            );
            image.asset_usage = RenderAssetUsages::RENDER_WORLD;
            if let Some(image_cache_info) = &mut self.image {
                image_cache_info.width = actual_width;
                image_cache_info.height = actual_height;
                image_cache_info
                    .caches
                    .insert((actual_width, actual_height), images.add(image));
            } else {
                let mut caches = HashMap::new();
                caches.insert((actual_width, actual_height), images.add(image));
                self.image = Some(ImageCacheInfo {
                    width: actual_width,
                    height: actual_height,
                    caches,
                });
            }
        } else {
            self.image = None;
        }
    }

    /// 显式清除缓存值并释放所有资源。
    /// 此操作仅应在无法渲染到缓存且需要暂时禁用缓存的情况下使用。
    pub fn clear(&mut self) {
        self.image = None;
    }

    pub fn image_info(&self) -> Option<ImageCacheInfo> {
        self.image.clone()
    }
}

#[derive(Debug, Clone, Default)]
pub struct DisplayObjectBase {
    name: Option<Box<str>>,
    place_frame: FrameNumber,
    depth: Depth,
    clip_depth: Depth,
    transform: Transform,
    filters: Vec<Filter>,
    blend_mode: BlendMode,
    as_bitmap_cached: bool,
    cache_dirty: bool,
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
            true
        } else {
            false
        }
    }

    fn recheck_cache(&self, id: CharacterId, image_caches: &mut HashMap<CharacterId, ImageCache>) {
        if !self.filters.is_empty() && image_caches.get(&id).is_none() || self.as_bitmap_cached {
            image_caches.insert(id, ImageCache::default());
        }
    }

    fn invalidate_cached_bitmap(&mut self) {
        self.cache_dirty = true;
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

    fn set_clip_depth(&mut self, clip_depth: Depth) {
        self.base_mut().clip_depth = clip_depth;
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
        if self.base_mut().set_filters(filters) {
            self.invalidate_cached_bitmap();
        }
    }

    fn invalidate_cached_bitmap(&mut self) {
        self.base_mut().invalidate_cached_bitmap();
    }

    fn recheck_cache(&self, id: CharacterId, image_caches: &mut HashMap<CharacterId, ImageCache>) {
        self.base().recheck_cache(id, image_caches);
    }

    fn set_name(&mut self, name: Option<Box<str>>) {
        self.base_mut().set_name(name);
    }

    fn movie(&self) -> Arc<SwfMovie>;

    fn id(&self) -> CharacterId;

    fn depth(&self) -> Depth {
        self.base().depth
    }

    fn clip_depth(&self) -> Depth {
        self.base().clip_depth
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

    fn cache_dirty(&self) -> bool {
        self.base().cache_dirty
    }

    /// 该对象未经过变换的固有边界框。这些边界不包含子显示对象（DisplayObjects）。
    /// 叶节点显示对象应返回自身的边界。仅包含子对象的复合显示对象应返回 &Default::default ()。
    fn self_bounds(&mut self, context: &mut RenderContext) -> Rectangle<Twips>;

    /// 获取此对象及其所有子对象的渲染边界。该边界与向 Flash 暴露的边界主要有两点不同：
    /// - 若应用了会增大显示内容尺寸的滤镜，渲染边界可能会更大
    /// - 不考虑滚动矩形
    fn render_bounds_with_transform(
        &mut self,
        matrix: &Matrix,
        include_own_filters: bool,
        context: &mut RenderContext,
    ) -> Rectangle<Twips> {
        let scale = context.scale;
        let mut bounds = *matrix * self.self_bounds(context);
        if let Some(children) = self.children_mut() {
            for child in children {
                let matrix = *matrix * *child.matrix();
                bounds = bounds.union(&child.render_bounds_with_transform(&matrix, true, context));
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
        if let Some(ratio) = place_object.ratio
            && let Some(morph_shape) = self.as_morph_shape()
        {
            morph_shape.set_ratio(ratio);
        }
        if let Some(blend_mode) = place_object.blend_mode {
            self.set_blend_mode(blend_mode);
        }
        if let Some(clip_depth) = place_object.clip_depth {
            self.set_clip_depth(clip_depth);
        }
        if let Some(is_bitmap_cached) = place_object.is_bitmap_cached {
            self.base_mut().as_bitmap_cached = is_bitmap_cached;
        }
        if version >= 11 {
            if let Some(_visible) = place_object.is_visible {
                //TODO:
                warn_once!("visible is not supported. id: {}. TODO!", self.id());
            }
            if let Some(_color) = place_object.background_color {}
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

    fn allow_as_mask(&self) -> bool {
        true
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

    fn self_bounds(&mut self, context: &mut RenderContext) -> Rectangle<Twips> {
        match self {
            Self::Graphic(g) => g.self_bounds(context),
            Self::MovieClip(m) => m.self_bounds(context),
            Self::MorphShape(m) => m.self_bounds(context),
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
