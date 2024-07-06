use swf::{CharacterId, Rectangle, Shape, Twips};

use super::{DisplayObjectBase, TDisplayObject};

#[derive(Clone)]
pub struct Graphic {
    base: DisplayObjectBase,
    id: CharacterId,
    shape: Shape,
    bounds: Rectangle<Twips>,
}

impl Graphic {
    pub fn from_swf_tag(shape: Shape) -> Self {
        Self {
            base: DisplayObjectBase::default(),
            id: shape.id,
            bounds: shape.shape_bounds.clone(),
            shape,
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
}
