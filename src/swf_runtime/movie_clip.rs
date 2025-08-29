use std::borrow::Cow;
use std::cmp::max;
use std::collections::BTreeMap;
use std::collections::btree_map::{Values, ValuesMut};
use std::sync::Arc;

use bevy::ecs::component::Component;
use bevy::log::error;
use bevy::platform::collections::HashMap;
use smallvec::SmallVec;
use swf::extensions::ReadSwfExt;
use swf::read::Reader;
use swf::{
    CharacterId, Color, DefineBitsLossless, Depth, PlaceObjectAction, Rectangle, TagCode, Twips,
};

use super::character::{BitmapLibrary, Character, CompressedBitmap, instantiate_by_id};
use super::decoder::decode_define_bits_jpeg_dimensions;
use super::display_object::{DisplayObject, DisplayObjectBase, FrameNumber, TDisplayObject};
use super::graphic::Graphic;
use super::morph_shape::MorphShape;
use super::tag_utils;
use super::tag_utils::{ControlFlow, Error, SwfMovie, SwfSlice};

#[derive(Debug, Clone, Component)]
pub struct MovieClip {
    id: CharacterId,
    base: DisplayObjectBase,
    swf_slice: SwfSlice,
    frame_labels: HashMap<Box<str>, FrameNumber>,
    total_frames: FrameNumber,
    current_frame: FrameNumber,
    tag_stream_pos: u64,
    depth_list: BTreeMap<Depth, DisplayObject>,
    playing: bool,
}

impl MovieClip {
    pub fn new(movie: Arc<SwfMovie>) -> Self {
        Self {
            id: 0,
            base: DisplayObjectBase::default(),
            total_frames: movie.total_frames(),
            swf_slice: SwfSlice::empty(movie),
            frame_labels: HashMap::new(),
            current_frame: 0,
            tag_stream_pos: 0,
            depth_list: BTreeMap::new(),
            playing: true,
        }
    }

    pub fn new_with_data(id: CharacterId, swf_slice: SwfSlice, total_frame: FrameNumber) -> Self {
        Self {
            id,
            base: DisplayObjectBase::default(),
            swf_slice,
            frame_labels: HashMap::new(),
            total_frames: total_frame,
            current_frame: 0,
            tag_stream_pos: 0,
            depth_list: BTreeMap::new(),
            playing: true,
        }
    }

    fn play(&mut self) {
        self.playing = true;
    }

    fn stop(&mut self) {
        self.playing = false;
    }

    pub(crate) fn preload(
        &mut self,
        characters: &mut HashMap<CharacterId, Character>,
        bitmaps: &mut BitmapLibrary,
    ) {
        let swf = self.swf_slice.clone();
        let mut reader = Reader::new(swf.data(), swf.version());
        let tag_callback = |reader: &mut Reader<'_>, tag_code, tag_len| {
            match tag_code {
                TagCode::DefineShape => define_shape(characters, self.movie(), reader, 1),
                TagCode::DefineShape2 => define_shape(characters, self.movie(), reader, 2),
                TagCode::DefineShape3 => define_shape(characters, self.movie(), reader, 3),
                TagCode::DefineShape4 => define_shape(characters, self.movie(), reader, 4),
                TagCode::DefineMorphShape => {
                    define_morph_shape(characters, self.movie(), reader, 1)
                }
                TagCode::DefineMorphShape2 => {
                    define_morph_shape(characters, self.movie(), reader, 2)
                }
                TagCode::DefineBitsJpeg2 => define_bits_jpeg_2(bitmaps, reader),
                TagCode::DefineBitsJpeg3 => define_bits_jpeg_3_or_4(bitmaps, reader, 3),
                TagCode::DefineBitsJpeg4 => define_bits_jpeg_3_or_4(bitmaps, reader, 4),
                TagCode::DefineBitsLossless => define_bits_lossless(bitmaps, reader, 1),
                TagCode::DefineBitsLossless2 => define_bits_lossless(bitmaps, reader, 2),
                TagCode::FrameLabel => self.frame_label(reader, self.current_frame()),
                TagCode::DefineSprite => {
                    return define_sprite(characters, bitmaps, &self.swf_slice, reader, tag_len);
                }
                TagCode::ShowFrame => {
                    self.current_frame += 1;
                    Ok(())
                }
                TagCode::End => {
                    return Ok(ControlFlow::Exit);
                }
                _ => Ok(()),
            }?;
            Ok(ControlFlow::Continue)
        };
        let _ = tag_utils::decode_tags(&mut reader, tag_callback);
    }

    fn frame_label(
        &mut self,
        reader: &mut Reader,
        current_frame: FrameNumber,
    ) -> Result<(), Error> {
        let frame_label = reader.read_frame_label()?;
        let label = frame_label.label.to_str_lossy(self.movie().encoding());
        self.frame_labels
            .insert(label.as_ref().into(), current_frame + 1);
        Ok(())
    }

    pub fn frame_labels(&self) -> &HashMap<Box<str>, FrameNumber> {
        &self.frame_labels
    }

    pub fn skin_frame(&self) -> HashMap<Box<str>, FrameNumber> {
        self.frame_labels
            .iter()
            .filter(|(k, _)| k.starts_with("skin_"))
            .map(|(k, v)| (k[5..].into(), *v))
            .collect()
    }

    pub fn render_list(&self) -> Values<'_, u16, DisplayObject> {
        self.depth_list.values()
    }

    pub fn render_list_mut(&mut self) -> ValuesMut<'_, u16, DisplayObject> {
        self.depth_list.values_mut()
    }

    pub fn current_frame(&self) -> FrameNumber {
        self.current_frame
    }

    pub fn total_frames(&self) -> FrameNumber {
        self.total_frames
    }

    fn determine_next_frame(&self) -> NextFrame {
        if self.current_frame() < self.total_frames() {
            NextFrame::Next
        } else if self.total_frames() > 1 {
            NextFrame::First
        } else {
            NextFrame::Same
        }
    }

    fn run_frame_internal(
        &mut self,
        characters: &HashMap<u16, Character>,
        run_display_actions: bool,
    ) {
        let next_frame = self.determine_next_frame();
        match next_frame {
            NextFrame::Next => {
                // self.current_frame += 1;
            }
            NextFrame::First => {
                self.run_goto(characters, 1, true);
            }
            NextFrame::Same => {}
        }

        let data = self.swf_slice.clone();
        let mut reader = data.read_from(self.tag_stream_pos);
        let tag_callback = |reader: &mut Reader<'_>, tag_code, _tag_len| {
            match tag_code {
                TagCode::PlaceObject if run_display_actions => {
                    self.place_object(characters, reader, 1)
                }
                TagCode::PlaceObject2 if run_display_actions => {
                    self.place_object(characters, reader, 2)
                }
                TagCode::PlaceObject3 if run_display_actions => {
                    self.place_object(characters, reader, 3)
                }
                TagCode::PlaceObject4 if run_display_actions => {
                    self.place_object(characters, reader, 4)
                }
                TagCode::RemoveObject if run_display_actions => self.remove_object(reader, 1),
                TagCode::RemoveObject2 if run_display_actions => self.remove_object(reader, 2),
                TagCode::ShowFrame => return Ok(ControlFlow::Exit),
                _ => Ok(()),
            }?;
            Ok(ControlFlow::Continue)
        };
        let _ = tag_utils::decode_tags(&mut reader, tag_callback);
        let tag_stream_start = self.swf_slice.as_ref().as_ptr() as u64;

        self.tag_stream_pos = reader.get_ref().as_ptr() as u64 - tag_stream_start;
        if matches!(next_frame, NextFrame::Next) {
            self.current_frame += 1;
        }
    }

    #[inline]
    fn place_object(
        &mut self,
        characters: &HashMap<CharacterId, Character>,
        reader: &mut Reader<'_>,
        version: u8,
    ) -> Result<(), Error> {
        let place_object = if version == 1 {
            reader.read_place_object()
        } else {
            reader.read_place_object_2_or_3(version)
        }?;

        match place_object.action {
            swf::PlaceObjectAction::Place(id) => {
                if let Some(child) = self.instantiate_child(characters, id, &place_object) {
                    self.replace_at_depth(child, place_object.depth);
                }
            }
            swf::PlaceObjectAction::Replace(id) => {
                let swf_slice = self.swf_slice.clone();
                let current_frame = self.current_frame();
                if let Some(child) = self.child_by_depth(place_object.depth) {
                    child.replace_with(id, characters);
                    child.apply_place_object(&place_object, swf_slice.version());
                    child.set_place_frame(current_frame);
                }
            }
            swf::PlaceObjectAction::Modify => {
                let swf_slice = self.swf_slice.clone();
                if let Some(child) = self.child_by_depth(place_object.depth) {
                    child.apply_place_object(&place_object, swf_slice.version());
                }
            }
        }
        Ok(())
    }

    fn instantiate_child(
        &mut self,
        characters: &HashMap<CharacterId, Character>,
        id: CharacterId,
        place_object: &swf::PlaceObject,
    ) -> Option<DisplayObject> {
        instantiate_by_id(
            id,
            characters,
            place_object,
            self.movie(),
            &self.swf_slice,
            self.current_frame,
        )
    }

    fn replace_at_depth(&mut self, child: DisplayObject, depth: u16) -> Option<DisplayObject> {
        self.depth_list.insert(depth, child)
    }

    fn child_by_depth(&mut self, depth: u16) -> Option<&mut DisplayObject> {
        self.depth_list.get_mut(&depth)
    }

    #[inline]
    fn remove_object(&mut self, reader: &mut Reader<'_>, version: u8) -> Result<(), Error> {
        let remove_object = if version == 1 {
            reader.read_remove_object_1()
        } else {
            reader.read_remove_object_2()
        }?;
        self.depth_list.remove(&remove_object.depth);
        Ok(())
    }

    pub fn goto_frame(
        &mut self,
        characters: &HashMap<CharacterId, Character>,
        frame: FrameNumber,
        stop: bool,
    ) {
        if stop {
            self.stop();
        } else {
            self.play();
        }

        let frame = frame.max(1);

        if frame != self.current_frame {
            self.run_goto(characters, frame, false)
        }
    }

    fn run_goto(
        &mut self,
        characters: &HashMap<CharacterId, Character>,
        frame: FrameNumber,
        _is_implicit: bool,
    ) {
        let is_rewind = if frame <= self.current_frame() {
            self.tag_stream_pos = 0;
            self.current_frame = 0;
            true
        } else {
            false
        };

        let from_frame = self.current_frame();
        let tag_stream_start = self.swf_slice.as_ref().as_ptr() as u64;
        let mut frame_pos = self.tag_stream_pos;
        let data = self.swf_slice.clone();
        let mut index = 0;

        let mut goto_commands: Vec<GotoPlaceObject<'_>> = vec![];
        let clamped_frame = frame.min(max(self.total_frames(), 0));
        let mut reader = data.read_from(frame_pos);
        while self.current_frame() < clamped_frame && !reader.get_ref().is_empty() {
            self.current_frame += 1;
            frame_pos = reader.get_ref().as_ptr() as u64 - tag_stream_start;

            let tag_callback = |reader: &mut _, tag_code, _tag_len| {
                match tag_code {
                    TagCode::PlaceObject => {
                        index += 1;
                        self.goto_place_object(reader, 1, &mut goto_commands, is_rewind, index)
                    }
                    TagCode::PlaceObject2 => {
                        index += 1;
                        self.goto_place_object(reader, 2, &mut goto_commands, is_rewind, index)
                    }
                    TagCode::PlaceObject3 => {
                        index += 1;
                        self.goto_place_object(reader, 3, &mut goto_commands, is_rewind, index)
                    }
                    TagCode::PlaceObject4 => {
                        index += 1;
                        self.goto_place_object(reader, 4, &mut goto_commands, is_rewind, index)
                    }
                    TagCode::RemoveObject => self.goto_remove_object(
                        reader,
                        1,
                        &mut goto_commands,
                        is_rewind,
                        from_frame,
                    ),
                    TagCode::RemoveObject2 => self.goto_remove_object(
                        reader,
                        2,
                        &mut goto_commands,
                        is_rewind,
                        from_frame,
                    ),
                    TagCode::ShowFrame => return Ok(ControlFlow::Exit),
                    _ => Ok(()),
                }?;
                Ok(ControlFlow::Continue)
            };
            let _ = tag_utils::decode_tags(&mut reader, tag_callback);
        }

        let hit_target_frame = self.current_frame == frame;
        let render_list = self.render_list();
        if is_rewind {
            let children: SmallVec<[_; 16]> = render_list
                .filter(|display_object| display_object.place_frame() > frame)
                .map(|display_object| display_object.depth())
                .collect();
            for child in children {
                self.depth_list.remove(&child);
            }
        }

        let movie = self.movie();
        let run_goto_command =
            |clip: &mut MovieClip,
             params: &GotoPlaceObject<'_>,
             character: &HashMap<CharacterId, Character>| {
                let child_entry = clip.child_by_depth(params.depth());

                match (params.place_object.action, child_entry, is_rewind) {
                    (_, Some(prev_child), true)
                    | (PlaceObjectAction::Modify, Some(prev_child), _) => {
                        prev_child.apply_place_object(&params.place_object, movie.version());
                    }
                    (PlaceObjectAction::Replace(id), Some(prev_child), _) => {
                        prev_child.replace_with(id, character);
                        prev_child.apply_place_object(&params.place_object, movie.version());
                        prev_child.set_place_frame(params.frame);
                    }
                    (PlaceObjectAction::Place(id), _, _)
                    | (swf::PlaceObjectAction::Replace(id), _, _) => {
                        if let Some(mut child) =
                            clip.instantiate_child(character, id, &params.place_object)
                        {
                            // Set the place frame to the frame where the object *would* have been placed.
                            child.set_place_frame(params.frame);
                            clip.replace_at_depth(child, params.depth());
                        }
                    }
                    _ => {
                        error!("Unhandled goto command: {:?}", &params.place_object);
                    }
                }
            };

        goto_commands.sort_by_key(|params| params.index);

        goto_commands
            .iter()
            .filter(|params| params.frame < frame)
            .for_each(|goto| run_goto_command(self, goto, characters));
        if hit_target_frame {
            self.current_frame -= 1;
            self.tag_stream_pos = frame_pos;
            self.run_frame_internal(characters, false);
        } else {
            self.current_frame = clamped_frame;
        }

        goto_commands
            .iter()
            .filter(|params| params.frame >= frame)
            .for_each(|goto| run_goto_command(self, goto, characters));
    }

    #[inline]
    fn goto_place_object<'a>(
        &mut self,
        reader: &mut Reader<'a>,
        version: u8,
        goto_commands: &mut Vec<GotoPlaceObject<'a>>,
        is_rewind: bool,
        index: usize,
    ) -> Result<(), Error> {
        let place_object = if version == 1 {
            reader.read_place_object()
        } else {
            reader.read_place_object_2_or_3(version)
        }?;
        let depth = place_object.depth;
        let mut goto_place =
            GotoPlaceObject::new(self.current_frame, place_object, is_rewind, index);
        if let Some(i) = goto_commands.iter().position(|o| o.depth() == depth) {
            goto_commands[i].merge(&mut goto_place);
        } else {
            goto_commands.push(goto_place);
        }
        Ok(())
    }
    /// Handles a RemoveObject tag when running a goto action.
    #[inline]
    fn goto_remove_object(
        &mut self,
        reader: &mut Reader<'_>,
        version: u8,
        goto_commands: &mut Vec<GotoPlaceObject<'_>>,
        is_rewind: bool,
        from_frame: FrameNumber,
    ) -> Result<(), Error> {
        let remove_object = if version == 1 {
            reader.read_remove_object_1()
        } else {
            reader.read_remove_object_2()
        }?;
        let depth = remove_object.depth;
        if let Some(i) = goto_commands.iter().position(|o| o.depth() == depth) {
            goto_commands.swap_remove(i);
        }
        if !is_rewind {
            let to_frame = self.current_frame;
            self.current_frame = from_frame;

            let child = self.child_by_depth(depth);

            if let Some(child) = child {
                let depth = child.depth();
                self.depth_list.remove(&depth);
            }
            self.current_frame = to_frame;
        }
        Ok(())
    }
}

impl TDisplayObject for MovieClip {
    fn base(&self) -> &DisplayObjectBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut DisplayObjectBase {
        &mut self.base
    }

    fn movie(&self) -> Arc<SwfMovie> {
        self.swf_slice.movie()
    }

    fn enter_frame(&mut self, characters: &HashMap<u16, Character>) {
        for child in self.render_list_mut().rev() {
            child.enter_frame(characters);
        }
        if self.playing {
            self.run_frame_internal(characters, true);
        }
    }

    fn self_bounds(&mut self) -> Rectangle<Twips> {
        Default::default()
    }

    fn children_mut(&mut self) -> Option<ValuesMut<'_, u16, DisplayObject>> {
        Some(self.render_list_mut())
    }

    fn id(&self) -> CharacterId {
        self.id
    }
}

impl From<MovieClip> for DisplayObject {
    fn from(movie_clip: MovieClip) -> Self {
        Self::MovieClip(movie_clip)
    }
}

/// Indication of what frame `run_frame` should jump to next.
#[derive(PartialEq, Eq)]
enum NextFrame {
    /// Construct and run the next frame in the clip.
    Next,

    /// Jump to the first frame in the clip.
    First,

    /// Do not construct or run any frames.
    Same,
}

fn define_sprite(
    characters: &mut HashMap<CharacterId, Character>,
    bitmaps: &mut BitmapLibrary,
    swf_slice: &SwfSlice,
    reader: &mut Reader<'_>,
    tag_len: usize,
) -> Result<ControlFlow, Error> {
    let start = reader.as_slice();
    let id = reader.read_character_id()?;
    let num_frames = reader.read_u16()?;
    let num_read = reader.pos(start);
    let mut movie_clip = MovieClip::new_with_data(
        id,
        swf_slice.resize_to_reader(reader, tag_len - num_read),
        num_frames,
    );
    movie_clip.preload(characters, bitmaps);
    characters.insert(id, Character::MovieClip(movie_clip));
    Ok(ControlFlow::Continue)
}

#[inline]
fn define_shape(
    characters: &mut HashMap<CharacterId, Character>,
    movie: Arc<SwfMovie>,
    reader: &mut Reader,
    version: u8,
) -> Result<(), Error> {
    let shape = reader.read_define_shape(version)?;
    let id = shape.id;
    characters.insert(id, Character::Graphic(Graphic::from_swf_tag(shape, movie)));

    Ok(())
}

#[inline]
fn define_morph_shape(
    characters: &mut HashMap<CharacterId, Character>,
    movie: Arc<SwfMovie>,
    reader: &mut Reader,
    version: u8,
) -> Result<(), Error> {
    let tag = reader.read_define_morph_shape(version)?;
    let id = tag.id;
    characters.insert(
        id,
        Character::MorphShape(MorphShape::from_swf_tag(&tag, movie)),
    );
    Ok(())
}

#[inline]
fn define_bits_jpeg_2(bitmaps: &mut BitmapLibrary, reader: &mut Reader) -> Result<(), Error> {
    let id = reader.read_u16()?;
    let jpeg_data = reader.read_slice_to_end();
    let (width, height) = decode_define_bits_jpeg_dimensions(jpeg_data)?;
    bitmaps.insert(
        id,
        CompressedBitmap::Jpeg {
            data: jpeg_data.to_owned(),
            alpha: None,
            width,
            height,
        },
    );

    Ok(())
}

#[inline]
fn define_bits_jpeg_3_or_4(
    bitmaps: &mut BitmapLibrary,
    reader: &mut Reader,
    version: u8,
) -> Result<(), Error> {
    let id = reader.read_u16()?;
    let jpeg_len = reader.read_u32()? as usize;
    if version == 4 {
        let _ = reader.read_u16()?;
    }
    let jpeg_data = reader.read_slice(jpeg_len)?;
    let alpha_data = reader.read_slice_to_end();
    let (width, height) = decode_define_bits_jpeg_dimensions(jpeg_data)?;
    bitmaps.insert(
        id,
        CompressedBitmap::Jpeg {
            data: jpeg_data.to_owned(),
            alpha: Some(alpha_data.to_owned()),
            width,
            height,
        },
    );
    Ok(())
}

#[inline]
fn define_bits_lossless(
    bitmaps: &mut BitmapLibrary,
    reader: &mut Reader,
    version: u8,
) -> Result<(), Error> {
    let bits_lossless = reader.read_define_bits_lossless(version)?;
    bitmaps.insert(
        bits_lossless.id,
        CompressedBitmap::Lossless(DefineBitsLossless {
            version,
            id: bits_lossless.id,
            format: bits_lossless.format,
            width: bits_lossless.width,
            height: bits_lossless.height,
            data: Cow::Owned(bits_lossless.data.into_owned()),
        }),
    );
    Ok(())
}

#[derive(Debug)]
pub(crate) struct GotoPlaceObject<'a> {
    frame: FrameNumber,

    place_object: swf::PlaceObject<'a>,

    index: usize,
}

impl<'a> GotoPlaceObject<'a> {
    fn new(
        frame: FrameNumber,
        mut place_object: swf::PlaceObject<'a>,
        is_rewind: bool,
        index: usize,
    ) -> Self {
        if is_rewind {
            if let swf::PlaceObjectAction::Place(_) = place_object.action {
                if place_object.matrix.is_none() {
                    place_object.matrix = Some(Default::default());
                }
                if place_object.color_transform.is_none() {
                    place_object.color_transform = Some(Default::default());
                }
                if place_object.ratio.is_none() {
                    place_object.ratio = Some(Default::default());
                }
                if place_object.blend_mode.is_none() {
                    place_object.blend_mode = Some(Default::default());
                }
                if place_object.is_bitmap_cached.is_none() {
                    place_object.is_bitmap_cached = Some(Default::default());
                }
                if place_object.background_color.is_none() {
                    place_object.background_color = Some(Color::from_rgba(0));
                }
                if place_object.filters.is_none() {
                    place_object.filters = Some(Default::default());
                }
            }
        }
        Self {
            frame,
            place_object,
            index,
        }
    }

    #[inline]
    fn depth(&self) -> Depth {
        self.place_object.depth
    }

    fn merge(&mut self, next: &mut GotoPlaceObject<'a>) {
        let cur_place = &mut self.place_object;
        let next_place = &mut next.place_object;
        match (cur_place.action, next_place.action) {
            (cur, PlaceObjectAction::Modify) => {
                cur_place.action = cur;
            }
            (_, new) => {
                cur_place.action = new;
                self.frame = next.frame;
            }
        };
        if next_place.matrix.is_some() {
            cur_place.matrix = next_place.matrix.take();
        }
        if next_place.color_transform.is_some() {
            cur_place.color_transform = next_place.color_transform.take();
        }
        if next_place.ratio.is_some() {
            cur_place.ratio = next_place.ratio.take();
        }
        if next_place.blend_mode.is_some() {
            cur_place.blend_mode = next_place.blend_mode.take();
        }
        if next_place.is_bitmap_cached.is_some() {
            cur_place.is_bitmap_cached = next_place.is_bitmap_cached.take();
        }
        if next_place.is_visible.is_some() {
            cur_place.is_visible = next_place.is_visible.take();
        }
        if next_place.background_color.is_some() {
            cur_place.background_color = next_place.background_color.take();
        }
        if next_place.filters.is_some() {
            cur_place.filters = next_place.filters.take();
        }
    }
}
