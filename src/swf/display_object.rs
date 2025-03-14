pub(crate) mod graphic;
pub mod movie_clip;
use std::sync::Arc;

use bitflags::bitflags;
use graphic::Graphic;
use movie_clip::MovieClip;
use ruffle_macros::enum_trait_object;
use ruffle_render::{
    backend::RenderBackend,
    bitmap::{BitmapHandle, BitmapInfo},
    blend::ExtendedBlendMode,
    filters::Filter,
    matrix::Matrix,
    transform::Transform,
};
use swf::{CharacterId, Color, ColorTransform, Depth, Point, Rectangle, Twips};

use super::{library::MovieLibrary, tag_utils::SwfMovie};

bitflags! {
    /// Bit flags used by `DisplayObject`.
    #[derive(Clone, Copy)]
    struct DisplayObjectFlags: u16 {
        /// If this object is visible (`_visible` property).
        const VISIBLE                  = 1 << 0;

        /// Whether this object is a "root", the top-most display object of a loaded SWF or Bitmap.
        /// Used by `MovieClip.getBytesLoaded` in AVM1 and `DisplayObject.root` in AVM2.
        const IS_ROOT                  = 1 << 1;

        /// Whether this object will be cached to bitmap.
        const CACHE_AS_BITMAP          = 1 << 2;

        /// If this object has already had `invalidate_cached_bitmap` called this frame
        const CACHE_INVALIDATED        = 1 << 3;
    }
}
#[derive(Clone, Debug, Default)]
#[allow(dead_code)]
pub struct BitmapCache {
    /// The `Matrix.a` value that was last used with this cache
    matrix_a: f32,
    /// The `Matrix.b` value that was last used with this cache
    matrix_b: f32,
    /// The `Matrix.c` value that was last used with this cache
    matrix_c: f32,
    /// The `Matrix.d` value that was last used with this cache
    matrix_d: f32,

    /// The width of the original bitmap, pre-filters
    source_width: u16,

    /// The height of the original bitmap, pre-filters
    source_height: u16,

    /// The offset used to draw the final bitmap (i.e. if a filter increases the size)
    draw_offset: Point<i32>,

    /// The current contents of the cache, if any. Values are post-filters.
    bitmap: Option<BitmapInfo>,

    /// Whether we warned that this bitmap was too large to be cached
    warned_for_oversize: bool,
}

#[allow(dead_code)]
impl BitmapCache {
    /// Forcefully make this BitmapCache invalid and require regeneration.
    /// This should be used for changes that aren't automatically detected, such as children.
    pub fn make_dirty(&mut self) {
        // Setting the old transform to something invalid is a cheap way of making it invalid,
        // without reserving an extra field for.
        self.matrix_a = f32::NAN;
    }

    fn is_dirty(&self, other: &Matrix, source_width: u16, source_height: u16) -> bool {
        self.matrix_a != other.a
            || self.matrix_b != other.b
            || self.matrix_c != other.c
            || self.matrix_d != other.d
            || self.source_width != source_width
            || self.source_height != source_height
            || self.bitmap.is_none()
    }

    /// Clears any dirtiness and ensure there's an appropriately sized texture allocated
    #[allow(clippy::too_many_arguments)]
    fn update(
        &mut self,
        renderer: &mut dyn RenderBackend,
        matrix: Matrix,
        source_width: u16,
        source_height: u16,
        actual_width: u16,
        actual_height: u16,
        draw_offset: Point<i32>,
        swf_version: u8,
    ) {
        self.matrix_a = matrix.a;
        self.matrix_b = matrix.b;
        self.matrix_c = matrix.c;
        self.matrix_d = matrix.d;
        self.source_width = source_width;
        self.source_height = source_height;
        self.draw_offset = draw_offset;
        if let Some(current) = &mut self.bitmap {
            if current.width == actual_width && current.height == actual_height {
                return; // No need to resize it
            }
        }
        let acceptable_size = if swf_version > 9 {
            let total = actual_width as u32 * actual_height as u32;
            actual_width < 8191 && actual_height < 8191 && total < 16777215
        } else {
            actual_width < 2880 && actual_height < 2880
        };
        if renderer.is_offscreen_supported()
            && actual_width > 0
            && actual_height > 0
            && acceptable_size
        {
            let handle = renderer.create_empty_texture(actual_width as u32, actual_height as u32);
            self.bitmap = handle.ok().map(|handle| BitmapInfo {
                width: actual_width,
                height: actual_height,
                handle,
            });
        } else {
            self.bitmap = None;
        }
    }

    /// Explicitly clears the cached value and drops any resources.
    /// This should only be used in situations where you can't render to the cache and it needs to be
    /// temporarily disabled.
    fn clear(&mut self) {
        self.bitmap = None;
    }

    fn handle(&self) -> Option<BitmapHandle> {
        self.bitmap.as_ref().map(|b| b.handle.clone())
    }
}

#[derive(Clone)]
pub struct DisplayObjectBase {
    place_frame: u16,
    depth: Depth,
    clip_depth: Depth,
    name: Option<String>,
    transform: Transform,
    blend_mode: ExtendedBlendMode,
    flags: DisplayObjectFlags,
    #[allow(unused)]
    scaling_grid: Rectangle<Twips>,
    opaque_background: Option<Color>,
    filters: Vec<Filter>,
    cache: Option<BitmapCache>,
}
unsafe impl Send for DisplayObjectBase {}
unsafe impl Sync for DisplayObjectBase {}

impl Default for DisplayObjectBase {
    fn default() -> Self {
        Self {
            place_frame: Default::default(),
            depth: Default::default(),
            clip_depth: Default::default(),
            name: None,
            transform: Default::default(),
            blend_mode: Default::default(),
            opaque_background: Default::default(),
            flags: DisplayObjectFlags::VISIBLE,
            filters: Default::default(),
            cache: None,
            scaling_grid: Default::default(),
        }
    }
}
impl DisplayObjectBase {
    pub fn transform(&self) -> &Transform {
        &self.transform
    }

    fn blend_mode(&self) -> ExtendedBlendMode {
        self.blend_mode
    }

    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    pub fn matrix(&self) -> &Matrix {
        &self.transform.matrix
    }

    fn filters(&self) -> Vec<Filter> {
        self.filters.clone()
    }

    #[allow(unused)]
    fn clip_depth(&self) -> Depth {
        self.clip_depth
    }

    fn visible(&self) -> bool {
        self.flags.contains(DisplayObjectFlags::VISIBLE)
    }

    pub fn set_name(&mut self, name: Option<String>) {
        self.name = name;
    }

    pub fn set_transform(&mut self, transform: Transform) {
        self.transform = transform;
    }
    pub fn set_root(&mut self) {
        self.flags.set(DisplayObjectFlags::IS_ROOT, true);
    }

    fn set_depth(&mut self, depth: Depth) {
        self.depth = depth;
    }

    fn set_matrix(&mut self, matrix: Matrix) {
        self.transform.matrix = matrix;
    }

    pub fn set_color_transform(&mut self, color_transform: ColorTransform) {
        self.transform.color_transform = color_transform;
    }

    fn set_bitmap_cached_preference(&mut self, value: bool) {
        self.flags.set(DisplayObjectFlags::CACHE_AS_BITMAP, value);
        self.recheck_cache_as_bitmap();
    }

    fn is_bitmap_cached_preference(&self) -> bool {
        self.flags.contains(DisplayObjectFlags::CACHE_AS_BITMAP)
    }

    fn recheck_cache_as_bitmap(&mut self) {
        let should_cache = self.is_bitmap_cached_preference() || !self.filters.is_empty();
        if should_cache && self.cache.is_none() {
            self.cache = Some(Default::default());
        } else if !should_cache && self.cache.is_some() {
            self.cache = None;
        }
    }
    fn set_blend_mode(&mut self, value: ExtendedBlendMode) -> bool {
        let changed = self.blend_mode != value;
        self.blend_mode = value;
        changed
    }
    fn set_filters(&mut self, filters: Vec<Filter>) -> bool {
        if filters != self.filters {
            self.filters = filters;
            self.recheck_cache_as_bitmap();
            true
        } else {
            false
        }
    }

    fn set_visible(&mut self, visible: bool) -> bool {
        let changed = self.visible() != visible;
        self.flags.set(DisplayObjectFlags::VISIBLE, visible);
        changed
    }

    fn set_opaque_background(&mut self, value: Option<Color>) -> bool {
        let value = value.map(|mut color| {
            color.a = 255;
            color
        });
        let changed = self.opaque_background != value;
        self.opaque_background = value;
        changed
    }

    fn set_place_frame(&mut self, frame: u16) {
        self.place_frame = frame;
    }

    fn invalidate_cached_bitmap(&mut self) -> bool {
        if self.flags.contains(DisplayObjectFlags::CACHE_INVALIDATED) {
            return false;
        }
        if let Some(cache) = &mut self.cache {
            cache.make_dirty();
        }
        self.flags.insert(DisplayObjectFlags::CACHE_INVALIDATED);
        true
    }

    pub fn clear_invalidate_flag(&mut self) {
        self.flags.remove(DisplayObjectFlags::CACHE_INVALIDATED);
    }

    fn set_clip_depth(&mut self, depth: Depth) {
        self.clip_depth = depth;
    }

    fn is_root(&self) -> bool {
        self.flags.contains(DisplayObjectFlags::IS_ROOT)
    }

    #[allow(unused)]
    fn bitmap_cache_mut(&mut self) -> Option<&mut BitmapCache> {
        self.cache.as_mut()
    }
}

#[enum_trait_object(
    #[derive(Clone)]
    pub enum DisplayObject {
        MovieClip(MovieClip),
        Graphic(Graphic),
    }
)]

pub trait TDisplayObject: Clone + Into<DisplayObject> {
    fn base(&self) -> &DisplayObjectBase;
    fn base_mut(&mut self) -> &mut DisplayObjectBase;
    fn movie(&self) -> Arc<SwfMovie>;
    fn character_id(&self) -> CharacterId;
    fn depth(&self) -> Depth {
        self.base().depth
    }

    fn place_frame(&self) -> u16 {
        self.base().place_frame
    }

    fn is_bitmap_cached(&self) -> bool {
        self.base().cache.is_some()
    }
    fn is_root(&self) -> bool {
        self.base().is_root()
    }
    fn filters(&self) -> Vec<Filter> {
        self.base().filters()
    }
    fn opaque_background(&self) -> Option<Color> {
        self.base().opaque_background
    }

    fn allow_as_mask(&self) -> bool {
        true
    }
    fn visible(&self) -> bool {
        self.base().visible()
    }
    fn name(&self) -> Option<&str> {
        self.base().name()
    }

    fn blend_mode(&self) -> ExtendedBlendMode {
        self.base().blend_mode()
    }

    fn set_name(&mut self, name: Option<String>) {
        self.base_mut().set_name(name);
    }
    fn set_clip_depth(&mut self, depth: Depth) {
        self.base_mut().set_clip_depth(depth);
    }
    fn set_matrix(&mut self, matrix: Matrix) {
        self.base_mut().set_matrix(matrix);
    }
    fn set_color_transform(&mut self, color_transform: ColorTransform) {
        self.base_mut().set_color_transform(color_transform);
    }
    fn set_bitmap_cached_preference(&mut self, value: bool) {
        self.base_mut().set_bitmap_cached_preference(value);
    }
    fn set_blend_mode(&mut self, blend_mode: ExtendedBlendMode) {
        self.base_mut().set_blend_mode(blend_mode);
    }
    fn set_opaque_background(&mut self, value: Option<Color>) {
        if self.base_mut().set_opaque_background(value) {
            self.invalidate_cached_bitmap();
        }
    }
    fn set_filters(&mut self, filters: Vec<Filter>) {
        self.base_mut().set_filters(filters);
    }
    fn set_visible(&mut self, visible: bool) {
        self.base_mut().set_visible(visible);
    }
    fn set_place_frame(&mut self, frame: u16) {
        self.base_mut().set_place_frame(frame);
    }
    fn set_depth(&mut self, depth: Depth) {
        self.base_mut().set_depth(depth);
    }
    fn set_root(&mut self) {
        self.base_mut().set_root();
    }

    fn invalidate_cached_bitmap(&mut self) {
        if self.base_mut().invalidate_cached_bitmap() {
            // Don't inform ancestors if we've already done so this frame
        }
    }

    fn set_default_instance_name(&mut self, library: &mut MovieLibrary) {
        if self.base().name.is_none() {
            let name = format!("instance{}", library.instance_count);
            self.set_name(Some(name));
            library.instance_count = library.instance_count.wrapping_add(1);
        }
    }

    fn post_instantiation(&mut self, library: &mut MovieLibrary) {
        self.set_default_instance_name(library);
    }

    fn enter_frame(&mut self, _library: &mut MovieLibrary) {}

    fn replace_with(&mut self, _id: CharacterId, _library: &mut MovieLibrary) {}

    fn as_movie(&mut self) -> Option<MovieClip> {
        None
    }

    fn apply_place_object(&mut self, place_object: &swf::PlaceObject, swf_version: u8) {
        if let Some(matrix) = place_object.matrix {
            self.set_matrix(matrix.into());
        }
        if let Some(color_transform) = &place_object.color_transform {
            self.set_color_transform(*color_transform);
        }
        if let Some(is_bitmap_cached) = place_object.is_bitmap_cached {
            self.set_bitmap_cached_preference(is_bitmap_cached);
        }
        if let Some(blend_mode) = place_object.blend_mode {
            self.set_blend_mode(blend_mode.into());
        }
        if swf_version >= 11 {
            if let Some(visible) = place_object.is_visible {
                self.set_visible(visible);
            }
            if let Some(mut color) = place_object.background_color {
                let color = if color.a > 0 {
                    color.a = 255;
                    Some(color)
                } else {
                    None
                };
                self.set_opaque_background(color);
            }
        }
        if let Some(filters) = &place_object.filters {
            self.set_filters(filters.iter().map(Filter::from).collect())
        }
    }
}
