use std::sync::Arc;

use bevy::{
    asset::{io::Reader, Asset, AssetLoader, AsyncReadExt, LoadContext},
    reflect::TypePath,
};

use crate::swf::{library::MovieLibrary, tag_utils};

#[derive(Asset, TypePath)]
pub struct SwfMovie {
    pub swf_movie: Arc<tag_utils::SwfMovie>,
    // pub movie_libraries: PtrWeakKeyHashMap<Weak<SwfMovie>, MovieLibrary>,
    pub movie_library: MovieLibrary,
}

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
        Ok(SwfMovie {
            swf_movie,
            movie_library: MovieLibrary::new(),
        })
    }

    fn extensions(&self) -> &[&str] {
        &["swf"]
    }
}
