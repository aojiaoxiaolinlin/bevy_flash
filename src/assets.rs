use std::sync::Arc;

use bevy::{
    asset::{io::Reader, Asset, AssetLoader, AsyncReadExt, LoadContext},
    reflect::TypePath,
};

use crate::swf::{display_object::movie_clip::MovieClip, library::MovieLibrary, tag_utils};

#[derive(Asset, TypePath)]
pub struct SwfMovie {
    pub library: MovieLibrary,
    pub root_movie_clip: MovieClip,
}
impl SwfMovie {}
#[derive(Default)]
pub(crate) struct SwfLoader;

impl AssetLoader for SwfLoader {
    type Asset = SwfMovie;

    type Settings = ();

    type Error = tag_utils::Error;
    async fn load<'a>(
        &'a self,
        reader: &'a mut Reader<'_>,
        _settings: &'a (),
        _load_context: &'a mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut swf_data = Vec::new();
        reader.read_to_end(&mut swf_data).await?;
        let swf_movie = Arc::new(tag_utils::SwfMovie::from_data(&swf_data[..])?);
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

#[derive(Debug, Clone, Copy)]
pub enum FlashRunFrameStatus {
    Running,
    Stop,
}
