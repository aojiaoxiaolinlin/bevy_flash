use bitflags::bitflags;
use morph_shape::MorphShape;
use ruffle_render::{
    bitmap::{BitmapHandle, BitmapInfo},
    blend::ExtendedBlendMode,
    filters::Filter,
    matrix::Matrix,
    transform::Transform,
};
use swf::{CharacterId, Color, ColorTransform, Depth, Point};

use super::library::MovieLibrary;
pub mod graphic;
pub mod morph_shape;
pub mod movie_clip;

bitflags! {
    /// Bit flags used by `DisplayObject`.
    #[derive(Clone, Copy)]
    struct DisplayObjectFlags: u16 {
        /// Whether this object has been removed from the display list.
        /// Necessary in AVM1 to throw away queued actions from removed movie clips.
        const AVM1_REMOVED             = 1 << 0;

        /// If this object is visible (`_visible` property).
        const VISIBLE                  = 1 << 1;

        /// Whether the `_xscale`, `_yscale` and `_rotation` of the object have been calculated and cached.
        const SCALE_ROTATION_CACHED    = 1 << 2;

        /// Whether this object has been transformed by ActionScript.
        /// When this flag is set, changes from SWF `PlaceObject` tags are ignored.
        const TRANSFORMED_BY_SCRIPT    = 1 << 3;

        /// Whether this object has been placed in a container by ActionScript 3.
        /// When this flag is set, changes from SWF `RemoveObject` tags are ignored.
        const PLACED_BY_SCRIPT         = 1 << 4;

        /// Whether this object has been instantiated by a SWF tag.
        /// When this flag is set, attempts to change the object's name from AVM2 throw an exception.
        const INSTANTIATED_BY_TIMELINE = 1 << 5;

        /// Whether this object is a "root", the top-most display object of a loaded SWF or Bitmap.
        /// Used by `MovieClip.getBytesLoaded` in AVM1 and `DisplayObject.root` in AVM2.
        const IS_ROOT                  = 1 << 6;

        /// Whether this object has `_lockroot` set to true, in which case
        /// it becomes the _root of itself and of any children
        const LOCK_ROOT                = 1 << 7;

        /// Whether this object will be cached to bitmap.
        const CACHE_AS_BITMAP          = 1 << 8;

        /// Whether this object has a scroll rectangle applied.
        const HAS_SCROLL_RECT          = 1 << 9;

        /// Whether this object has an explicit name.
        const HAS_EXPLICIT_NAME        = 1 << 10;

        /// Flag set when we should skip running our next 'enterFrame'
        /// for ourself and our children.
        /// This is set for objects constructed from ActionScript,
        /// which are observed to lag behind objects placed by the timeline
        /// (even if they are both placed in the same frame)
        const SKIP_NEXT_ENTER_FRAME    = 1 << 11;

        /// If this object has already had `invalidate_cached_bitmap` called this frame
        const CACHE_INVALIDATED        = 1 << 12;

        /// If this AVM1 object is pending removal (will be removed on the next frame).
        const AVM1_PENDING_REMOVAL     = 1 << 13;
    }
}

#[derive(Clone, Debug, Default)]
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
    opaque_background: Option<Color>,
    filters: Vec<Filter>,
    cache: Option<BitmapCache>,
}
impl Default for DisplayObjectBase {
    fn default() -> Self {
        Self {
            place_frame: Default::default(),
            depth: Default::default(),
            clip_depth: Default::default(),
            name: Default::default(),
            transform: Default::default(),
            blend_mode: Default::default(),
            flags: DisplayObjectFlags::VISIBLE,
            opaque_background: Default::default(),
            filters: Default::default(),
            cache: Default::default(),
        }
    }
}
impl DisplayObjectBase {
    fn set_blend_mode(&mut self, value: ExtendedBlendMode) -> bool {
        let changed = self.blend_mode != value;
        self.blend_mode = value;
        changed
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
    fn set_filters(&mut self, filters: Vec<Filter>) -> bool {
        // if filters != self.filters {
        //     self.filters = filters;
        //     self.recheck_cache_as_bitmap();
        //     true
        // } else {
        //     false
        // }
        true
    }
    fn set_bitmap_cached_preference(&mut self, value: bool) {
        self.flags.set(DisplayObjectFlags::CACHE_AS_BITMAP, value);
        self.recheck_cache_as_bitmap();
    }
    fn recheck_cache_as_bitmap(&mut self) {
        // let should_cache = self.is_bitmap_cached_preference() || !self.filters.is_empty();
        // if should_cache && self.cache.is_none() {
        //     self.cache = Some(Default::default());
        // } else if !should_cache && self.cache.is_some() {
        //     self.cache = None;
        // }
    }
    fn visible(&self) -> bool {
        self.flags.contains(DisplayObjectFlags::VISIBLE)
    }
    fn is_bitmap_cached_preference(&self) -> bool {
        self.flags.contains(DisplayObjectFlags::CACHE_AS_BITMAP)
    }
}

pub trait TDisplayObject: Send + Sync {
    fn base(&self) -> &DisplayObjectBase;
    fn base_mut(&mut self) -> &mut DisplayObjectBase;

    fn character_id(&self) -> CharacterId;

    fn depth(&self) -> Depth {
        self.base().depth
    }

    fn enter_frame(&mut self, _library: &mut MovieLibrary) {}

    fn set_name(&mut self, name: Option<String>) {
        self.base_mut().name = name;
    }
    fn set_depth(&mut self, depth: Depth) {
        self.base_mut().depth = depth;
    }
    fn set_clip_depth(&mut self, depth: Depth) {
        self.base_mut().clip_depth = depth;
    }
    fn set_place_frame(&mut self, frame: u16) {
        self.base_mut().place_frame = frame;
    }
    fn set_matrix(&mut self, matrix: Matrix) {
        self.base_mut().transform.matrix = matrix;
    }
    fn set_color_transform(&mut self, color_transform: ColorTransform) {
        self.base_mut().transform.color_transform = color_transform.into();
    }
    fn set_blend_mode(&mut self, blend_mode: ExtendedBlendMode) {
        self.base_mut().set_blend_mode(blend_mode);
    }
    fn set_visible(&mut self, visible: bool) {
        self.base_mut().set_visible(visible);
    }
    fn set_opaque_background(&mut self, color: Option<Color>) {
        self.base_mut().set_opaque_background(color);
    }
    fn set_filters(&mut self, filters: Vec<Filter>) {
        self.base_mut().set_filters(filters);
    }
    fn set_bitmap_cached_preference(&mut self, value: bool) {
        self.base_mut().set_bitmap_cached_preference(value);
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
        if let Some(is_bitmap_cached) = place_object.is_bitmap_cached {
            self.set_bitmap_cached_preference(is_bitmap_cached);
        }
        if let Some(blend_mode) = place_object.blend_mode {
            self.set_blend_mode(blend_mode.into());
        }
        if version >= 11 {
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

    fn as_morph_shape(&mut self) -> Option<&mut MorphShape> {
        None
    }
}
