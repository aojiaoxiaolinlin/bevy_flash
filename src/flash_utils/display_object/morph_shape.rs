use std::sync::{Arc, RwLock};

use swf::{CharacterId, Rectangle, Twips};

use crate::flash_utils::tag_utils::SwfMovie;

use super::TDisplayObject;

struct Frame {
    // shape_handle: Option<ShapeHandle>,
    shape: swf::Shape,
    bounds: Rectangle<Twips>,
}
#[derive(Clone)]
pub struct MorphShape {
    ratio: u16,
    id: CharacterId,
    start: swf::MorphShape,
    end: swf::MorphShape,
    frames: Arc<RwLock<fnv::FnvHashMap<u16, Frame>>>,
    movie: Arc<SwfMovie>,
}

impl MorphShape {
    pub fn set_ratio(&mut self, ratio: u16) {
        self.ratio = ratio;
    }
}

impl TDisplayObject for MorphShape {
    fn base_mut(&mut self) -> &mut super::DisplayObjectBase {
        todo!()
    }
    fn as_morph_shape(&mut self) -> Option<&mut MorphShape> {
        Some(self)
    }

    fn base(&self) -> &super::DisplayObjectBase {
        todo!()
    }

    fn character_id(&self) -> CharacterId {
        todo!()
    }

    fn movie(&self) -> Arc<SwfMovie> {
        self.movie.clone()
    }
}
