use std::sync::Arc;

use bevy::log::error;
use bevy::platform::collections::HashMap;
use swf::{CharacterId, DefineBitsLossless, PlaceObject};

use crate::swf_runtime::display_object::FrameNumber;

use super::display_object::{DisplayObject, TDisplayObject};
use super::tag_utils::{SwfMovie, SwfSlice};

use super::decoder::error::Error;
use super::decoder::{Bitmap, decode_define_bits_jpeg, decode_define_bits_lossless};
use super::graphic::Graphic;
use super::morph_shape::MorphShape;
use super::movie_clip::MovieClip;

#[derive(Clone)]
pub enum Character {
    MovieClip(MovieClip),
    Graphic(Graphic),
    MorphShape(MorphShape),
}

impl From<Character> for DisplayObject {
    fn from(value: Character) -> Self {
        match value {
            Character::MovieClip(movie_clip) => DisplayObject::MovieClip(movie_clip),
            Character::Graphic(graphic) => DisplayObject::Graphic(graphic),
            Character::MorphShape(morph_shape) => DisplayObject::MorphShape(morph_shape),
        }
    }
}

pub type BitmapLibrary = HashMap<CharacterId, CompressedBitmap>;

#[derive(Clone)]
pub enum CompressedBitmap {
    Jpeg {
        data: Vec<u8>,
        alpha: Option<Vec<u8>>,
        width: u16,
        height: u16,
    },
    Lossless(DefineBitsLossless<'static>),
}

impl CompressedBitmap {
    pub fn size(&self) -> BitmapSize {
        match self {
            CompressedBitmap::Jpeg { width, height, .. } => BitmapSize {
                width: *width,
                height: *height,
            },
            CompressedBitmap::Lossless(DefineBitsLossless { width, height, .. }) => BitmapSize {
                width: *width,
                height: *height,
            },
        }
    }

    pub fn decode(&self) -> Result<Bitmap, Error> {
        match self {
            CompressedBitmap::Jpeg { data, alpha, .. } => {
                decode_define_bits_jpeg(data, alpha.as_deref())
            }
            CompressedBitmap::Lossless(define_bits_lossless) => {
                decode_define_bits_lossless(define_bits_lossless)
            }
        }
    }
}

pub struct BitmapSize {
    pub width: u16,
    pub height: u16,
}

pub fn instantiate_by_id(
    id: CharacterId,
    characters: &HashMap<CharacterId, Character>,
    place_object: &PlaceObject,
    movie: Arc<SwfMovie>,
    swf_slice: &SwfSlice,
    place_frame: FrameNumber,
) -> Option<DisplayObject> {
    match characters.get(&id).cloned() {
        Some(child) => {
            let mut child: DisplayObject = child.into();
            if let Some(name) = &place_object.name {
                let name = name.to_str_lossy(movie.encoding());
                child.set_name(Some(name.as_ref().into()));
            }
            child.set_depth(place_object.depth);
            child.set_place_frame(place_frame);
            child.apply_place_object(&place_object, swf_slice.version());
            // 运行第一帧
            child.enter_frame(characters);
            Some(child)
        }
        None => {
            error!("Unable to instantiate display node id {}", id);
            None
        }
    }
}
