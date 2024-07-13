use std::sync::Arc;

use swf::{CharacterId, Rectangle, Shape, Twips};

use crate::flash_utils::{library::MovieLibrary, tag_utils::SwfMovie};

use super::{DisplayObjectBase, TDisplayObject};

#[derive(Clone)]
pub struct Graphic {
    pub id: CharacterId,
    shape: Shape,
    bounds: Rectangle<Twips>,
    base: DisplayObjectBase,
    swf_movie: Arc<SwfMovie>,
}

impl Graphic {
    pub fn from_swf_tag(shape: Shape, swf_movie: Arc<SwfMovie>) -> Self {
        Self {
            id: shape.id,
            bounds: shape.shape_bounds.clone(),
            shape,
            base: DisplayObjectBase::default(),
            swf_movie,
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
        self.swf_movie.clone()
    }

    fn replace_with(&mut self, id: CharacterId, library: &mut MovieLibrary) {
        if let Some(new_graphic) = library.get_graphic(id) {
            self.id = new_graphic.id;
            self.shape = new_graphic.shape;
            self.bounds = new_graphic.bounds;
            self.base = new_graphic.base;
        } else {
            dbg!("PlaceObject: expected Graphic at character ID {}", id);
        }
    }
}
