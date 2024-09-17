use std::sync::Arc;

use bevy::{asset::Handle, prelude::Mesh};
use swf::{CharacterId, Rectangle, Shape, Twips};

use crate::{
    render::material::{BitmapMaterial, GradientMaterial},
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
    gradient_mesh: Vec<(Handle<Mesh>, Handle<GradientMaterial>)>,
    mesh: Option<Handle<Mesh>>,
    bitmap_mesh: Vec<(Handle<Mesh>, Handle<BitmapMaterial>)>,
}

impl Graphic {
    pub fn from_swf_tag(shape: Shape, swf_movie: Arc<SwfMovie>) -> Self {
        Self {
            id: shape.id,
            bounds: shape.shape_bounds.clone(),
            shape,
            base: DisplayObjectBase::default(),
            swf_movie,
            gradient_mesh: Vec::new(),
            mesh: None,
            bitmap_mesh: Vec::new(),
        }
    }
    pub fn add_gradient_mesh(
        &mut self,
        mesh: Handle<Mesh>,
        gradient_material: Handle<GradientMaterial>,
    ) {
        self.gradient_mesh.push((mesh, gradient_material));
    }

    pub fn add_bitmap_mesh(&mut self, bitmap_mesh: (Handle<Mesh>, Handle<BitmapMaterial>)) {
        self.bitmap_mesh.push(bitmap_mesh);
    }

    pub fn set_mesh(&mut self, mesh: Handle<Mesh>) {
        self.mesh = Some(mesh);
    }

    pub fn mesh(&self) -> Option<Handle<Mesh>> {
        self.mesh.clone()
    }

    pub fn gradient_mesh(&self) -> &Vec<(Handle<Mesh>, Handle<GradientMaterial>)> {
        &self.gradient_mesh
    }

    pub fn bitmap_mesh(&self) -> &Vec<(Handle<Mesh>, Handle<BitmapMaterial>)> {
        &self.bitmap_mesh
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
            self.gradient_mesh = new_graphic.gradient_mesh;
            self.bitmap_mesh = new_graphic.bitmap_mesh;
        } else {
            dbg!("PlaceObject: expected Graphic at character ID {}", id);
        }
    }
}
