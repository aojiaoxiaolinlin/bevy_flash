use std::sync::Arc;

use super::decoder;
use bevy::prelude::{error, warn};
use swf::{HeaderExt, TagCode, read::Reader};
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct SwfMovie {
    /// 从数据流解析的SWF头
    header: HeaderExt,

    /// 未解压的SWF数据
    data: Vec<u8>,

    /// SWF建议编码
    encoding: &'static swf::Encoding,
}

impl SwfMovie {
    pub fn from_data(swf_data: &[u8]) -> Result<Self, Error> {
        let swf_buf = swf::read::decompress_swf(swf_data)?;
        let encoding = swf::SwfStr::encoding_for_version(swf_buf.header.version());
        Ok(Self {
            header: swf_buf.header,
            data: swf_buf.data,
            encoding,
        })
    }

    pub fn encoding(&self) -> &'static swf::Encoding {
        self.encoding
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn frame_rate(&self) -> f32 {
        self.header.frame_rate().into()
    }

    pub fn total_frames(&self) -> u16 {
        self.header.num_frames()
    }

    pub fn is_action_script_3(&self) -> bool {
        self.header.is_action_script_3()
    }

    pub fn version(&self) -> u8 {
        self.header.version()
    }
}

#[derive(Debug, Clone)]
pub struct SwfSlice {
    pub movie: Arc<SwfMovie>,
    pub start: usize,
    pub end: usize,
}

impl SwfSlice {
    pub fn empty(movie: Arc<SwfMovie>) -> Self {
        Self {
            movie: movie.clone(),
            start: 0,
            end: movie.data.len(),
        }
    }

    /// Creates an empty SwfSlice of the same movie.
    #[inline]
    pub fn copy_empty(&self) -> Self {
        Self::empty(self.movie.clone())
    }

    /// Construct a reader for this slice.
    ///
    /// The `from` parameter is the offset to start reading the slice from.
    pub fn read_from(&self, from: u64) -> swf::read::Reader<'_> {
        swf::read::Reader::new(&self.data()[from as usize..], self.version())
    }

    pub fn data(&self) -> &[u8] {
        &self.movie.data[self.start..self.end]
    }

    pub fn version(&self) -> u8 {
        self.movie.header.version()
    }

    pub fn movie(&self) -> Arc<SwfMovie> {
        self.movie.clone()
    }

    /// 根据一个读取器和一个大小构建一个新的 SwfSlice。
    /// 这旨在允许构建对给定 SWF 标签内容的引用。您只需要当前的读取器和您想要引用的标签的大小即可。
    /// 返回的切片可能是也可能不是当前切片的子切片。如果生成的切片超出底层影片的边界，或者给定的读取器指向不同的底层影片，此函数将返回一个空切片。
    pub fn resize_to_reader(&self, reader: &mut Reader<'_>, size: usize) -> SwfSlice {
        if self.movie.data().as_ptr() as usize <= reader.get_ref().as_ptr() as usize
            && (reader.get_ref().as_ptr() as usize)
                < self.movie.data().as_ptr() as usize + self.movie.data().len()
        {
            let outer_offset =
                reader.get_ref().as_ptr() as usize - self.movie.data().as_ptr() as usize;
            let new_start = outer_offset;
            let new_end = outer_offset + size;

            let len = self.movie.data().len();

            if new_start < len && new_end < len {
                Self {
                    movie: self.movie.clone(),
                    start: new_start,
                    end: new_end,
                }
            } else {
                self.copy_empty()
            }
        } else {
            self.copy_empty()
        }
    }
}

impl AsRef<[u8]> for SwfSlice {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        self.data()
    }
}

/// Whether or not to end tag decoding.
pub enum ControlFlow {
    /// Stop decoding after this tag.
    Exit,

    /// Continue decoding the next tag.
    Continue,
}

#[derive(Error, Debug)]
pub enum Error {
    #[error("Couldn't read SWF: {0}")]
    InvalidSwf(#[from] swf::error::Error),

    #[error("Couldn't register bitmap: {0}")]
    InvalidBitmap(#[from] decoder::error::Error),

    #[error("IO Error: {0}")]
    IOError(#[from] std::io::Error),
}

pub fn decode_tags<'a, F>(reader: &mut Reader<'a>, mut tag_callback: F) -> Result<bool, Error>
where
    F: for<'b> FnMut(&'b mut Reader<'a>, TagCode, usize) -> Result<ControlFlow, Error>,
{
    loop {
        let (tag_code, tag_len) = reader.read_tag_code_and_length()?;
        if tag_len > reader.get_ref().len() {
            error!("Unexpected EOF when reading tag");
            *reader.get_mut() = &reader.get_ref()[reader.get_ref().len()..];
            return Ok(false);
        }

        let tag_slice = &reader.get_ref()[..tag_len];
        let end_slice = &reader.get_ref()[tag_len..];
        if let Some(tag) = TagCode::from_u16(tag_code) {
            *reader.get_mut() = tag_slice;
            let result = tag_callback(reader, tag, tag_len);

            match result {
                Err(e) => {
                    error!("Error running definition tag: {:?}, got {}", tag, e)
                }
                Ok(ControlFlow::Exit) => {
                    *reader.get_mut() = end_slice;
                    break;
                }
                Ok(ControlFlow::Continue) => {}
            }
        } else {
            warn!("Unknown tag code: {:?}", tag_code);
        }

        *reader.get_mut() = end_slice;
    }

    Ok(true)
}
