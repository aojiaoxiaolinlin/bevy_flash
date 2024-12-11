use std::sync::Arc;

use bevy::{
    asset::{io::Reader, Asset, AssetLoader, LoadContext},
    reflect::TypePath,
};

use crate::swf::{
    display_object::{movie_clip::MovieClip, TDisplayObject},
    library::MovieLibrary,
    tag_utils,
};

#[derive(Asset, TypePath)]
pub struct SwfMovie {
    pub swf_movie: Arc<tag_utils::SwfMovie>,
    pub movie_library: MovieLibrary,
    pub root_movie_clip: MovieClip,
}

impl SwfMovie {
    pub fn is_target_movie_clip(&self, name: String) -> bool {
        if self.root_movie_clip.name().unwrap_or("root") == name {
            true
        } else {
            false
        }
    }
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
        Ok(SwfMovie {
            swf_movie,
            movie_library: MovieLibrary::new(),
            root_movie_clip: MovieClip::default(),
        })
    }

    fn extensions(&self) -> &[&str] {
        &["swf"]
    }
}
