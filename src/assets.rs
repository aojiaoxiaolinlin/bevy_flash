use std::sync::Arc;

use bevy::{
    asset::{io::Reader, Asset, AssetLoader, AsyncReadExt, Handle, LoadContext},
    reflect::TypePath,
};
use thiserror::Error;

use crate::flash_utils::{display_object::movie_clip::MovieClip, library::MovieLibrary};

#[derive(Error, Debug)]
pub enum FlashLoadError {
    #[error("加载文件:{0}")]
    IOError(#[from] std::io::Error),
    #[error("无法读取SWF: {0}")]
    InvalidSwf(#[from] swf::error::Error),
}
#[derive(Error, Debug)]
pub enum FlashParseError {
    #[error("无法读取SWF: {0}")]
    InvalidSwf(#[from] swf::error::Error),
}
#[derive(Asset, TypePath)]
pub struct SwfMovie {
    pub library: MovieLibrary,
    pub root_movie_clip: MovieClip,
}
#[derive(Default)]
pub(crate) struct SwfLoader;

impl AssetLoader for SwfLoader {
    type Asset = SwfMovie;

    type Settings = ();

    type Error = FlashLoadError;
    async fn load<'a>(
        &'a self,
        reader: &'a mut Reader<'_>,
        _settings: &'a (),
        _load_context: &'a mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut swf_data = Vec::new();
        reader.read_to_end(&mut swf_data).await?;
        let compressed_len = swf_data.len();
        let swf_buf = swf::read::decompress_swf(&swf_data[..])?;
        let encoding = swf::SwfStr::encoding_for_version(swf_buf.header.version());
        let swf_movie = Arc::new(crate::flash_utils::util::SwfMovie {
            header: swf_buf.header,
            data: swf_buf.data,
            encoding,
            compressed_len,
        });
        let mut root_movie_clip: MovieClip = MovieClip::new(swf_movie.clone());
        let mut library = MovieLibrary::new();
        root_movie_clip.parse_swf(&mut library);

        Ok(SwfMovie {
            library,
            root_movie_clip,
        })
    }

    fn extensions(&self) -> &[&str] {
        &["swf"]
    }
}

#[derive(Asset, TypePath)]
pub struct FlashData {
    pub swf_movie: Handle<SwfMovie>,
}

impl FlashData {
    pub fn new_from_binary_data(swf_movie: Handle<SwfMovie>) -> Self {
        Self { swf_movie }
    }
}