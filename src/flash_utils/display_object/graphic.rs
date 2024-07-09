use std::sync::Arc;

use swf::{CharacterId, Rectangle, Shape, Twips};

use crate::flash_utils::tag_utils::SwfMovie;

use super::{DisplayObjectBase, TDisplayObject};

#[derive(Clone)]
pub struct Graphic {
    base: DisplayObjectBase,
    id: CharacterId,
    shape: Shape,
    bounds: Rectangle<Twips>,
    movie: Arc<SwfMovie>,
}

impl Graphic {
    pub fn from_swf_tag(shape: Shape, movie: Arc<SwfMovie>) -> Self {
        Self {
            base: DisplayObjectBase::default(),
            id: shape.id,
            bounds: shape.shape_bounds.clone(),
            shape,
            movie,
        }
    }
}

impl TDisplayObject for Graphic {
    fn base_mut(&mut self) -> &mut DisplayObjectBase {
        &mut self.base
    }

    fn base(&self) -> &DisplayObjectBase {
        &self.base
    }

    fn character_id(&self) -> CharacterId {
        self.id
    }

    fn movie(&self) -> Arc<SwfMovie> {
        self.movie.clone()
    }
}
