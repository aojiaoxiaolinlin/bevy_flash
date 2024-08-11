use std::sync::Arc;

use bevy::sprite::Mesh2dHandle;
use ruffle_render::{tessellator::ShapeTessellator, transform::Transform};
use swf::{CharacterId, Rectangle, Shape, Twips};

use crate::swf::{library::MovieLibrary, tag_utils::SwfMovie};

use super::{DisplayObjectBase, TDisplayObject};

#[derive(Clone)]
pub struct Graphic {
    pub id: CharacterId,
    shape: Shape,
    bounds: Rectangle<Twips>,
    base: DisplayObjectBase,
    swf_movie: Arc<SwfMovie>,
    mesh: Option<Mesh2dHandle>,
}

impl Graphic {
    pub fn from_swf_tag(shape: Shape, swf_movie: Arc<SwfMovie>) -> Self {
        Self {
            id: shape.id,
            bounds: shape.shape_bounds.clone(),
            shape,
            base: DisplayObjectBase::default(),
            swf_movie,
            mesh: None,
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
    // TODO: Implement render_self
    fn render_self(&mut self, transform: Transform) {}
}
