use std::sync::Arc;

use swf::{error::Error, read::Reader, Fixed8, HeaderExt, TagCode};

pub enum ControlFlow {
    /// Stop decoding after this tag.
    Exit,

    /// Continue decoding the next tag.
    Continue,
}

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

impl SwfMovie {
    pub fn header(&self) -> &HeaderExt {
        &self.header
    }

    pub fn version(&self) -> u8 {
        self.header.version()
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn encoding(&self) -> &'static swf::Encoding {
        self.encoding
    }

    pub fn is_action_script_3(&self) -> bool {
        self.header.is_action_script_3()
    }

    pub fn num_frames(&self) -> u16 {
        self.header.num_frames()
    }

    pub fn frame_rate(&self) -> Fixed8 {
        self.header.frame_rate()
    }
}

#[derive(Clone)]
pub struct SwfSlice {
    movie: Arc<SwfMovie>,
    start: usize,
    end: usize,
}

impl SwfSlice {
    #[inline]
    pub fn empty(movie: Arc<SwfMovie>) -> Self {
        Self {
            movie: movie.clone(),
            start: 0,
            end: movie.data().len(),
        }
    }

    pub fn data(&self) -> &[u8] {
        &self.movie.data()[self.start..self.end]
    }

    /// Get the version of the SWF this data comes from.
    pub fn version(&self) -> u8 {
        self.movie.version()
    }

    pub fn movie(&self) -> &Arc<SwfMovie> {
        &self.movie
    }
    #[inline]
    pub fn copy_empty(&self) -> Self {
        Self::empty(self.movie.clone())
    }
    pub fn read_from(&self, from: u64) -> Reader<'_> {
        Reader::new(&self.movie.data()[from as usize..], self.version())
    }
    pub fn resize_to_reader(&self, reader: &mut Reader<'_>, size: usize) -> Self {
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

pub fn decode_tags<'a, F>(reader: &mut Reader<'a>, mut tag_callback: F) -> Result<bool, Error>
where
    F: for<'b> FnMut(&'b mut Reader<'a>, TagCode, usize) -> Result<ControlFlow, Error>,
{
    loop {
        let (tag_code, tag_len) = reader.read_tag_code_and_length()?;
        if tag_len > reader.get_ref().len() {
            // tracing::error!("Unexpected EOF when reading tag");
            dbg!("Unexpected EOF when reading tag");
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
                    eprintln!("Error running definition tag: {:?}", e);
                }
                Ok(ControlFlow::Exit) => {
                    *reader.get_mut() = end_slice;
                    break;
                }
                Ok(ControlFlow::Continue) => {}
            }
        } else {
            // tracing::warn!("Unknown tag code: {:?}", tag_code);
            dbg!("Unknown tag code", tag_code);
        }

        *reader.get_mut() = end_slice;
    }

    Ok(true)
}
