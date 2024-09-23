use std::sync::Arc;

use bevy::log::error;
use swf::{CharacterId, Rectangle, Shape, Twips};

use crate::{
    plugin::ShapeMesh,
    swf::{library::MovieLibrary, tag_utils::SwfMovie},
};

use super::{DisplayObjectBase, TDisplayObject};

#[derive(Clone)]
pub struct Graphic {
    pub id: CharacterId,
    pub shape: Shape,
    pub bounds: Rectangle<Twips>,
    base: DisplayObjectBase,
    swf_movie: Arc<SwfMovie>,
    shape_mesh: Vec<ShapeMesh>,
}

impl Graphic {
    pub fn from_swf_tag(shape: Shape, swf_movie: Arc<SwfMovie>) -> Self {
        Self {
            id: shape.id,
            bounds: shape.shape_bounds.clone(),
            shape,
            base: DisplayObjectBase::default(),
            swf_movie,
            shape_mesh: Vec::new(),
        }
    }

    pub fn add_shape_mesh(&mut self, shape_mesh: ShapeMesh) {
        self.shape_mesh.push(shape_mesh);
    }

    pub fn shape_mesh(&mut self) -> &mut Vec<ShapeMesh> {
        &mut self.shape_mesh
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
            self.shape_mesh = new_graphic.shape_mesh;
        } else {
            error!("PlaceObject: expected Graphic at character ID {}", id);
        }
    }
}
