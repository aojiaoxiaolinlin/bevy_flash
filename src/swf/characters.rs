use swf::DefineBitsLossless;

use super::display_object::{graphic::Graphic, movie_clip::MovieClip};

#[derive(Clone)]
pub enum Character {
    MovieClip(MovieClip),
    Graphic(Graphic),
    Bitmap(CompressedBitmap),
}

#[derive(Copy, Clone, Debug)]
pub struct BitmapSize {
    pub width: u16,
    pub height: u16,
}

#[derive(Clone, Debug)]
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

    pub fn decode(
        &self,
    ) -> Result<crate::render::utils::bitmap::Bitmap, crate::render::utils::error::Error> {
        match self {
            CompressedBitmap::Jpeg { data, alpha, .. } => {
                crate::render::utils::bitmap::decode_define_bits_jpeg(data, alpha.as_deref())
            }
            CompressedBitmap::Lossless(define_bits_lossless) => {
                crate::render::utils::bitmap::decode_define_bits_lossless(define_bits_lossless)
            }
        }
    }
}
