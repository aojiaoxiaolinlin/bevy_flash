use std::sync::Arc;

use bevy::{
    asset::{Asset, AssetLoader, LoadContext, io::Reader},
    reflect::TypePath,
};

use crate::swf::{display_object::movie_clip::MovieClip, library::MovieLibrary, tag_utils};

#[derive(Asset, TypePath)]
pub struct SwfMovie {
    pub library: MovieLibrary,
    pub movie_clip: MovieClip,
}

#[derive(Default)]
pub(crate) struct SwfLoader;

impl AssetLoader for SwfLoader {
    type Asset = SwfMovie;

    type Settings = ();

    type Error = tag_utils::Error;
    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &(),
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut swf_data = Vec::new();
        reader.read_to_end(&mut swf_data).await?;
        let swf_movie = Arc::new(tag_utils::SwfMovie::from_data(&swf_data[..])?);
        let mut movie_clip = MovieClip::new(swf_movie.clone());
        let mut library = MovieLibrary::new();
        movie_clip.parse_swf(&mut library);
        movie_clip.current_frame = 0;
        Ok(SwfMovie {
            library,
            movie_clip,
        })
    }

    fn extensions(&self) -> &[&str] {
        &["swf"]
    }
}
