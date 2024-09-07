use std::sync::Arc;

use bevy::{
    asset::Handle,
    log::info,
    prelude::{Image, Mesh},
};
use ruffle_render::transform::Transform;
use swf::{CharacterId, Rectangle, Shape, Twips};

use crate::swf::{library::MovieLibrary, tag_utils::SwfMovie};

use super::{DisplayObjectBase, TDisplayObject};

#[derive(Clone)]
pub enum GraphicStatus {
    Normal,
    Place,
    Replace,
}

#[derive(Clone)]
pub struct Graphic {
    pub id: CharacterId,
    pub shape: Shape,
    pub bounds: Rectangle<Twips>,
    base: DisplayObjectBase,
    swf_movie: Arc<SwfMovie>,
    texture: Option<Handle<Image>>,
    mesh: Option<Handle<Mesh>>,
    status: GraphicStatus,
}

impl Graphic {
    pub fn from_swf_tag(shape: Shape, swf_movie: Arc<SwfMovie>) -> Self {
        info!("Graphic::from_swf_tag:{}", shape.id);
        Self {
            id: shape.id,
            bounds: shape.shape_bounds.clone(),
            shape,
            base: DisplayObjectBase::default(),
            swf_movie,
            texture: None,
            mesh: None,
            status: GraphicStatus::Place,
        }
    }
    pub fn set_texture(&mut self, texture: Handle<Image>) {
        self.texture = Some(texture);
    }

    pub fn set_mesh(&mut self, mesh: Handle<Mesh>) {
        self.mesh = Some(mesh);
    }

    pub fn set_status(&mut self, status: GraphicStatus) {
        self.status = status;
    }

    pub fn mesh(&self) -> Option<Handle<Mesh>> {
        self.mesh.clone()
    }

    pub fn status(&self) -> GraphicStatus {
        self.status.clone()
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
            self.mesh = new_graphic.mesh;
            self.status = GraphicStatus::Replace;
        } else {
            dbg!("PlaceObject: expected Graphic at character ID {}", id);
        }
    }

    fn render_self(&mut self, transform: Transform) {}
}
