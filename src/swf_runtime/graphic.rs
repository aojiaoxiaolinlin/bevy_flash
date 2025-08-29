use std::sync::Arc;

use bevy::platform::collections::HashMap;
use swf::{BlendMode, CharacterId, Rectangle, Twips};

use crate::RenderContext;
use crate::swf_runtime::character::Character;

use super::tag_utils::SwfMovie;

use super::display_object::{DisplayObject, DisplayObjectBase, TDisplayObject};

#[derive(Debug, Clone)]
pub struct Graphic {
    id: CharacterId,
    base: DisplayObjectBase,
    shape: swf::Shape,
    bounds: Rectangle<Twips>,
    movie: Arc<SwfMovie>,
}

impl Graphic {
    pub fn from_swf_tag(shape: swf::Shape, movie: Arc<SwfMovie>) -> Self {
        Self {
            id: shape.id,
            base: Default::default(),
            bounds: shape.shape_bounds.clone(),
            shape,
            movie,
        }
    }

    pub fn id(&self) -> CharacterId {
        self.id
    }

    pub fn shape(&self) -> &swf::Shape {
        &self.shape
    }

    pub fn shape_mut(&mut self) -> &mut swf::Shape {
        &mut self.shape
    }
}

impl TDisplayObject for Graphic {
    fn base(&self) -> &DisplayObjectBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut DisplayObjectBase {
        &mut self.base
    }

    fn movie(&self) -> Arc<SwfMovie> {
        self.movie.clone()
    }

    fn replace_with(
        &mut self,
        id: CharacterId,
        characters: &HashMap<CharacterId, super::character::Character>,
    ) {
        if let Some(Character::Graphic(graphic)) = characters.get(&id) {
            self.id = graphic.id;
            self.shape = graphic.shape.clone();
            self.bounds = graphic.bounds.clone();
            self.movie = graphic.movie.clone();
        }
    }

    fn self_bounds(&mut self) -> Rectangle<Twips> {
        self.bounds.clone()
    }

    fn id(&self) -> CharacterId {
        self.id
    }

    fn render_self(
        &mut self,
        context: &mut RenderContext,
        blend_mode: BlendMode,
        shape_depth_layer: String,
    ) {
        context.render_shape(
            self.id(),
            context.transform_stack.transform(),
            shape_depth_layer,
            blend_mode.into(),
        );
    }
}

impl From<Graphic> for DisplayObject {
    fn from(graphic: Graphic) -> Self {
        Self::Graphic(graphic)
    }
}
