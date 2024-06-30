use bevy::{
    asset::{Asset, AssetLoader, AsyncReadExt, Handle},
    reflect::TypePath,
};
use swf::HeaderExt;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum FlashLoadError {
    #[error("加载文件:{0}")]
    IOError(#[from] std::io::Error),
}
#[derive(Error, Debug)]
pub enum FlashParseError {
    #[error("无法读取SWF: {0}")]
    InvalidSwf(#[from] swf::error::Error),
}
#[derive(Asset, Debug, TypePath)]
pub struct SwfMovie {
    pub header: HeaderExt,

    /// Uncompressed SWF data.
    ///
    pub data: Vec<u8>,

    /// The suggest encoding for this SWF.
    pub encoding: &'static swf::Encoding,

    /// The compressed length of the entire data stream
    pub compressed_len: usize,
}
#[derive(Default)]
pub(crate) struct SwfLoader;

impl AssetLoader for SwfLoader {
    type Asset = SwfMovie;

    type Settings = ();

    type Error = FlashLoadError;

    fn load<'a>(
        &'a self,
        reader: &'a mut bevy::asset::io::Reader,
        _settings: &'a Self::Settings,
        _load_context: &'a mut bevy::asset::LoadContext,
    ) -> bevy::utils::BoxedFuture<'a, Result<Self::Asset, Self::Error>> {
        Box::pin(async move {
            let mut swf_data = Vec::new();
            reader.read_to_end(&mut swf_data).await?;
            let compressed_len = swf_data.len();
            let swf_buf = swf::read::decompress_swf(&swf_data[..]).expect("解析失败");
            let encoding = swf::SwfStr::encoding_for_version(swf_buf.header.version());
            Ok(SwfMovie {
                header: swf_buf.header,
                data: swf_buf.data,
                encoding,
                compressed_len,
            })
        })
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
