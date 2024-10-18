use std::{cmp::max, collections::HashMap, sync::Arc, usize};

use anyhow::anyhow;
use bevy::log::{error, info};
use bitflags::bitflags;

use smallvec::SmallVec;
use swf::{
    extensions::ReadSwfExt, read::Reader, CharacterId, Color, Depth, PlaceObjectAction, SwfStr,
    TagCode,
};

use crate::swf::{
    characters::{Character, CompressedBitmap},
    container::ChildContainer,
    library::MovieLibrary,
    tag_utils::{self, ControlFlow, Error, SwfMovie, SwfSlice, SwfStream},
};

use super::{graphic::Graphic, DisplayObject, DisplayObjectBase, TDisplayObject};

type FrameNumber = u16;
type SwfVersion = u8;
/// Indication of what frame `run_frame` should jump to next.
#[derive(PartialEq, Eq)]
pub enum NextFrame {
    /// Construct and run the next frame in the clip.
    Next,

    /// Jump to the first frame in the clip.
    First,

    /// Do not construct or run any frames.
    Same,
}
bitflags! {
    /// Boolean state flags used by `MovieClip`.
    #[derive(Clone, Copy)]
    struct MovieClipFlags: u8 {
        /// Whether this `MovieClip` has run its initial frame.
        const INITIALIZED             = 1 << 0;

        /// Whether this `MovieClip` is playing or stopped.
        const PLAYING                 = 1 << 1;

        /// Whether this `MovieClip` has been played as a result of an AS3 command.
        ///
        /// The AS3 `isPlaying` property is broken and yields false until you first
        /// call `play` to unbreak it. This flag tracks that bug.
        const PROGRAMMATICALLY_PLAYED = 1 << 2;

        /// Executing an AVM2 frame script.
        ///
        /// This causes any goto action to be queued and executed at the end of the script.
        const EXECUTING_AVM2_FRAME_SCRIPT = 1 << 3;

        /// Flag set when AVM2 loops to the next frame.
        ///
        /// Because AVM2 queues PlaceObject tags to run later, explicit gotos
        /// that happen while those tags run should cancel the loop.
        const LOOP_QUEUED = 1 << 4;

        const RUNNING_CONSTRUCT_FRAME = 1 << 5;

        /// Whether this `MovieClip` has been post-instantiated yet.
        const POST_INSTANTIATED = 1 << 5;
    }
}
#[derive(Clone)]
pub struct MovieClip {
    base: DisplayObjectBase,
    swf: SwfSlice,
    pub id: CharacterId,
    pub current_frame: FrameNumber,
    pub total_frames: FrameNumber,
    frame_labels: Vec<(FrameNumber, String)>,
    container: ChildContainer,
    flags: MovieClipFlags,
    tag_stream_pos: u64,
    queued_tags: HashMap<Depth, QueuedTagList>,
}
impl Default for MovieClip {
    fn default() -> Self {
        Self {
            base: Default::default(),
            swf: Default::default(),
            id: Default::default(),
            current_frame: Default::default(),
            total_frames: Default::default(),
            frame_labels: Default::default(),
            container: Default::default(),
            flags: MovieClipFlags::PLAYING,
            tag_stream_pos: Default::default(),
            queued_tags: Default::default(),
        }
    }
}
impl MovieClip {
    pub fn new(movie: Arc<SwfMovie>) -> Self {
        Self {
            base: DisplayObjectBase::default(),
            id: Default::default(),
            current_frame: 0,
            total_frames: movie.num_frames(),
            frame_labels: Default::default(),
            swf: SwfSlice::empty(movie),
            container: ChildContainer::new(),
            flags: MovieClipFlags::PLAYING,
            tag_stream_pos: 0,
            queued_tags: HashMap::new(),
        }
    }
    pub fn new_with_data(id: CharacterId, total_frames: FrameNumber, swf: SwfSlice) -> Self {
        Self {
            base: DisplayObjectBase::default(),
            id,
            total_frames,
            current_frame: 0,
            frame_labels: Default::default(),
            swf,
            container: ChildContainer::new(),
            flags: MovieClipFlags::PLAYING,
            tag_stream_pos: 0,
            queued_tags: HashMap::new(),
        }
    }

    pub fn raw_container(&self) -> &ChildContainer {
        &self.container
    }

    pub fn raw_container_mut(&mut self) -> &mut ChildContainer {
        &mut self.container
    }

    pub fn replace_at_depth(&mut self, depth: Depth, child: DisplayObject) {
        self.raw_container_mut().replace_at_depth(depth, child);
    }

    pub fn num_children(&self) -> usize {
        self.container.render_list_len()
    }

    fn child_by_depth(&mut self, depth: Depth) -> Option<&mut DisplayObject> {
        self.container.child_by_depth(depth)
    }

    pub fn first_child_movie_clip(&mut self) -> Option<&MovieClip> {
        if let Some(child) = self.container.first_child() {
            match child {
                DisplayObject::MovieClip(mc) => Some(mc),
                _ => None,
            }
        } else {
            None
        }
    }

    pub fn parse_swf(&mut self, library: &mut MovieLibrary) {
        let swf = self.swf.clone();
        let mut reader = Reader::new(&swf.data()[..], swf.version());
        let tag_callback = |reader: &mut SwfStream<'_>, tag_code, tag_len| {
            match tag_code {
                // TagCode::SetBackgroundColor => self.set_background_color(library, reader),
                TagCode::DefineBitsJpeg3 => self.define_bits_jpeg_3_or_4(library, reader, 3),
                TagCode::DefineBitsJpeg4 => self.define_bits_jpeg_3_or_4(library, reader, 4),
                TagCode::DefineShape => self.define_shape(library, reader, 1),
                TagCode::DefineShape2 => self.define_shape(library, reader, 2),
                TagCode::DefineShape3 => self.define_shape(library, reader, 3),
                TagCode::DefineShape4 => self.define_shape(library, reader, 4),
                TagCode::DefineSprite => return self.define_sprite(library, reader, tag_len),
                TagCode::FrameLabel => self.frame_label(reader),
                TagCode::ShowFrame => self.show_frame(),
                _ => Ok(()),
            }?;
            Ok(ControlFlow::Continue)
        };

        let _ = tag_utils::decode_tags(&mut reader, tag_callback);
    }

    #[inline]
    fn frame_label(&mut self, reader: &mut SwfStream) -> Result<(), Error> {
        let frame_label = reader.read_frame_label()?;
        let label = frame_label
            .label
            .to_str_lossy(SwfStr::encoding_for_version(self.swf.version()));
        self.frame_labels
            .push((self.current_frame, label.into_owned()));
        Ok(())
    }

    #[inline]
    fn show_frame(&mut self) -> Result<(), Error> {
        self.current_frame += 1;
        Ok(())
    }

    #[inline]
    fn define_sprite(
        &mut self,
        library: &mut MovieLibrary,
        reader: &mut SwfStream,
        tag_len: usize,
    ) -> Result<ControlFlow, Error> {
        let start = reader.as_slice();
        let id = reader.read_character_id()?;
        let num_frames = reader.read_u16()?;
        let num_read = reader.pos(start);
        let mut movie_clip = MovieClip::new_with_data(
            id,
            num_frames,
            self.swf.resize_to_reader(reader, tag_len - num_read),
        );
        movie_clip.parse_swf(library);
        library.register_character(id, Character::MovieClip(movie_clip));
        Ok(ControlFlow::Continue)
    }

    #[inline]
    fn define_shape(
        &mut self,
        library: &mut MovieLibrary,
        reader: &mut SwfStream,
        version: u8,
    ) -> Result<(), Error> {
        let swf_shape = reader.read_define_shape(version)?;
        let id = swf_shape.id;
        let graphic = Graphic::from_swf_tag(swf_shape, self.movie().clone());
        library.register_character(id, Character::Graphic(graphic));
        Ok(())
    }

    #[inline]
    fn define_bits_jpeg_3_or_4(
        &mut self,
        library: &mut MovieLibrary,
        reader: &mut SwfStream,
        version: u8,
    ) -> Result<(), Error> {
        let id = reader.read_u16()?;
        let jpeg_len = reader.read_u32()? as usize;
        if version == 4 {
            let _de_blocking = reader.read_u16()?;
        }
        let jpeg_data = reader.read_slice(jpeg_len)?;
        let alpha_data = reader.read_slice_to_end();
        let (width, height) = ruffle_render::utils::decode_define_bits_jpeg_dimensions(jpeg_data)?;
        library.register_character(
            id,
            Character::Bitmap(CompressedBitmap::Jpeg {
                data: jpeg_data.to_owned(),
                alpha: Some(alpha_data.to_owned()),
                width,
                height,
            }),
        );
        Ok(())
    }

    pub fn run_frame_internal(
        &mut self,
        library: &mut MovieLibrary,
        run_display_actions: bool,
        is_action_script_3: bool,
    ) {
        let next_frame: NextFrame = self.determine_next_frame();
        match next_frame {
            NextFrame::Next => {}
            NextFrame::First => {
                // dbg!(self.name(), "end");
                return self.run_goto(library, 1, true);
            }
            NextFrame::Same => {}
        }
        let data = self.swf.clone();
        let mut reader = data.read_from(self.tag_stream_pos);
        let tag_callback = |reader: &mut SwfStream<'_>, tag_code, _tag_len| {
            match tag_code {
                TagCode::PlaceObject if run_display_actions && !is_action_script_3 => {
                    self.place_object(library, reader, 1)
                }
                TagCode::PlaceObject2 if run_display_actions && !is_action_script_3 => {
                    self.place_object(library, reader, 2)
                }
                TagCode::PlaceObject3 if run_display_actions && !is_action_script_3 => {
                    self.place_object(library, reader, 3)
                }
                TagCode::PlaceObject4 if run_display_actions && !is_action_script_3 => {
                    self.place_object(library, reader, 4)
                }
                TagCode::RemoveObject if run_display_actions && !is_action_script_3 => {
                    self.remove_object(reader, 1)
                }
                TagCode::RemoveObject2 if run_display_actions && !is_action_script_3 => {
                    self.remove_object(reader, 2)
                }
                TagCode::PlaceObject if run_display_actions && is_action_script_3 => {
                    self.queue_place_object(reader, 1)
                }
                TagCode::PlaceObject2 if run_display_actions && is_action_script_3 => {
                    self.queue_place_object(reader, 2)
                }
                TagCode::PlaceObject3 if run_display_actions && is_action_script_3 => {
                    self.queue_place_object(reader, 3)
                }
                TagCode::PlaceObject4 if run_display_actions && is_action_script_3 => {
                    self.queue_place_object(reader, 4)
                }
                TagCode::RemoveObject if run_display_actions && is_action_script_3 => {
                    self.queue_remove_object(reader, 1)
                }
                TagCode::RemoveObject2 if run_display_actions && is_action_script_3 => {
                    self.queue_remove_object(reader, 2)
                }
                // TagCode::SetBackgroundColor => self.set_background_color(library, reader),
                TagCode::ShowFrame => return Ok(ControlFlow::Exit),
                _ => Ok(()),
            }?;
            Ok(ControlFlow::Continue)
        };
        let _ = tag_utils::decode_tags(&mut reader, tag_callback);
        let tag_stream_start = self.swf.as_ref().as_ptr() as u64;

        let remove_actions = self.unqueue_removes();

        for (_, tag) in remove_actions {
            let mut reader = data.read_from(tag.tag_start);
            let version = match tag.tag_type {
                QueuedTagAction::Remove(v) => v,
                _ => unreachable!(),
            };

            if let Err(e) = self.remove_object(&mut reader, version) {
                error!("Error running queued tag: {:?}, got {}", tag.tag_type, e);
            }
        }

        self.tag_stream_pos = reader.get_ref().as_ptr() as u64 - tag_stream_start;
        if matches!(next_frame, NextFrame::Next) && is_action_script_3 {
            self.current_frame += 1;
        }
    }

    #[inline]
    fn place_object(
        &mut self,
        library: &mut MovieLibrary,
        reader: &mut SwfStream,
        version: SwfVersion,
    ) -> Result<(), Error> {
        let place_object = if version == 1 {
            reader.read_place_object()
        } else {
            reader.read_place_object_2_or_3(version)
        }?;
        match place_object.action {
            PlaceObjectAction::Place(id) => {
                if let Some(child) =
                    self.instantiate_child(id, place_object.depth, &place_object, library)
                {
                    self.replace_at_depth(place_object.depth, child);
                }
            }
            PlaceObjectAction::Replace(id) => {
                let swf = self.swf.clone();
                let current_frame = self.current_frame;
                if let Some(child) = self.child_by_depth(place_object.depth.into()) {
                    child.replace_with(id, library);
                    child.apply_place_object(&place_object, swf.version());
                    child.set_place_frame(current_frame);
                }
            }
            PlaceObjectAction::Modify => {
                let swf = self.swf.clone();
                if let Some(child) = self.child_by_depth(place_object.depth.into()) {
                    child.apply_place_object(&place_object, swf.version());
                }
            }
        }
        Ok(())
    }

    #[inline]
    fn queue_place_object(
        &mut self,
        reader: &mut SwfStream,
        version: SwfVersion,
    ) -> Result<(), Error> {
        let tag_start = reader.get_ref().as_ptr() as u64 - self.swf.as_ref().as_ptr() as u64;
        let place_object = if version == 1 {
            reader.read_place_object()
        } else {
            reader.read_place_object_2_or_3(version)
        }?;

        let new_tag = QueuedTag {
            tag_type: QueuedTagAction::Place(version),
            tag_start,
        };
        let bucket = self
            .queued_tags
            .entry(place_object.depth as Depth)
            .or_insert_with(|| QueuedTagList::None);
        bucket.queue_add(new_tag);
        Ok(())
    }

    #[inline]
    fn remove_object(&mut self, reader: &mut SwfStream, version: SwfVersion) -> Result<(), Error> {
        let remove_object = if version == 1 {
            reader.read_remove_object_1()
        } else {
            reader.read_remove_object_2()
        }?;
        if let Some(_child) = self.child_by_depth(remove_object.depth.into()) {
            self.raw_container_mut()
                .remove_child(remove_object.depth.into());
        }
        Ok(())
    }

    #[inline]
    fn queue_remove_object(
        &mut self,
        reader: &mut SwfStream,
        version: SwfVersion,
    ) -> Result<(), Error> {
        let tag_start = reader.get_ref().as_ptr() as u64 - self.swf.as_ref().as_ptr() as u64;
        let remove_object = if version == 1 {
            reader.read_remove_object_1()
        } else {
            reader.read_remove_object_2()
        }?;
        let new_tag = QueuedTag {
            tag_type: QueuedTagAction::Remove(version),
            tag_start,
        };
        let bucket = self
            .queued_tags
            .entry(remove_object.depth as Depth)
            .or_insert_with(|| QueuedTagList::None);
        bucket.queue_remove(new_tag);
        Ok(())
    }

    fn instantiate_child(
        &mut self,
        id: CharacterId,
        depth: Depth,
        place_object: &swf::PlaceObject,
        library: &mut MovieLibrary,
    ) -> Option<DisplayObject> {
        let child = self.instantiate_by_id(id, library);
        match child {
            Ok(mut child) => {
                child.set_depth(depth);
                child.set_place_frame(self.current_frame);
                child.apply_place_object(&place_object, self.swf.version());
                if let Some(name) = &place_object.name {
                    let name = name
                        .to_str_lossy(SwfStr::encoding_for_version(self.swf.version()))
                        .into_owned();
                    child.set_name(Some(name));
                }
                if let Some(clip_depth) = place_object.clip_depth {
                    child.set_clip_depth(clip_depth);
                }
                child.post_instantiation(library);
                child.enter_frame(library);
                Some(child)
            }
            Err(_e) => None,
        }
    }

    fn instantiate_by_id(
        &mut self,
        id: CharacterId,
        library: &mut MovieLibrary,
    ) -> anyhow::Result<DisplayObject> {
        if let Some(character) = library.character(id) {
            match character.clone() {
                Character::MovieClip(movie_clip) => Ok(movie_clip.into()),
                Character::Graphic(graphic) => Ok(graphic.into()),
                _ => unreachable!("Character id 不在库中"),
            }
        } else {
            Err(anyhow!("Character id 不在库中"))
        }
    }

    pub fn run_goto(&mut self, library: &mut MovieLibrary, frame: FrameNumber, is_implicit: bool) {
        // let frame_before_rewind = self.current_frame;
        let mut goto_commands: Vec<GotoPlaceObject> = Vec::new();

        let is_rewind = if frame <= self.current_frame {
            self.tag_stream_pos = 0;
            self.current_frame = 0;
            true
        } else {
            false
        };
        let from_frame = self.current_frame;
        let tag_stream_start = self.swf.as_ref().as_ptr() as u64;
        let mut frame_pos = self.tag_stream_pos;
        let data = self.swf.clone();
        let mut index = 0;

        let clamped_frame = frame.min(max(self.total_frames, 0));
        let mut reader = data.read_from(frame_pos);
        while self.current_frame < clamped_frame && !reader.get_ref().is_empty() {
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

        let render_list = self.raw_container().render_list();
        if is_rewind {
            let children: SmallVec<[_; 16]> = render_list
                .iter()
                .filter(|display_id| {
                    if let Some(display_object) = self.container.display_objects().get(display_id) {
                        display_object.place_frame() > frame
                    } else {
                        false
                    }
                })
                .map(|display_id| {
                    self.container
                        .display_objects()
                        .get(display_id)
                        .unwrap()
                        .depth()
                })
                .collect();

            for child in children {
                self.raw_container_mut().remove_child(child);
            }
        }
        let movie = self.movie();
        let run_goto_command = |clip: &mut MovieClip,
                                params: &GotoPlaceObject<'_>,
                                library: &mut MovieLibrary| {
            let child_entry = clip.child_by_depth(params.depth());

            if movie.is_action_script_3() && is_implicit && child_entry.is_none() {
                let new_tag = QueuedTag {
                    tag_type: QueuedTagAction::Place(params.version),
                    tag_start: params.tag_start,
                };
                let bucket = clip
                    .queued_tags
                    .entry(params.place_object.depth as Depth)
                    .or_insert_with(|| QueuedTagList::None);
                bucket.queue_add(new_tag);
                return;
            }

            match (params.place_object.action, child_entry, is_rewind) {
                (_, Some(prev_child), true) | (PlaceObjectAction::Modify, Some(prev_child), _) => {
                    prev_child.apply_place_object(&params.place_object, movie.version());
                }
                (PlaceObjectAction::Replace(id), Some(prev_child), _) => {
                    prev_child.replace_with(id, library);
                    prev_child.apply_place_object(&params.place_object, movie.version());
                    prev_child.set_place_frame(params.frame);
                }
                (PlaceObjectAction::Place(id), _, _)
                | (swf::PlaceObjectAction::Replace(id), _, _) => {
                    if let Some(mut child) =
                        clip.instantiate_child(id, params.depth(), &params.place_object, library)
                    {
                        // Set the place frame to the frame where the object *would* have been placed.
                        child.set_place_frame(params.frame);
                        clip.replace_at_depth(params.depth(), child);
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
            .for_each(|goto| run_goto_command(self, goto, library));
        if hit_target_frame {
            self.current_frame -= 1;
            self.tag_stream_pos = frame_pos;
            self.run_frame_internal(library, false, self.movie().is_action_script_3());
        } else {
            self.current_frame = clamped_frame;
        }

        goto_commands
            .iter()
            .filter(|params| params.frame >= frame)
            .for_each(|goto| run_goto_command(self, goto, library));
    }

    #[inline]
    fn goto_place_object<'a>(
        &mut self,
        reader: &mut SwfStream<'a>,
        version: SwfVersion,
        goto_commands: &mut Vec<GotoPlaceObject<'a>>,
        is_rewind: bool,
        index: usize,
    ) -> Result<(), Error> {
        let tag_start = reader.get_ref().as_ptr() as u64 - self.swf.as_ref().as_ptr() as u64;
        let place_object = if version == 1 {
            reader.read_place_object()
        } else {
            reader.read_place_object_2_or_3(version)
        }?;
        let depth: Depth = place_object.depth.into();
        let mut goto_place = GotoPlaceObject::new(
            self.current_frame,
            place_object,
            is_rewind,
            index,
            tag_start,
            version,
        );
        if let Some(i) = goto_commands.iter().position(|o| o.depth() == depth) {
            goto_commands[i].merge(&mut goto_place);
        } else {
            goto_commands.push(goto_place);
        }
        Ok(())
    }

    #[inline]
    fn goto_remove_object(
        &mut self,
        reader: &mut SwfStream,
        version: SwfVersion,
        goto_commands: &mut Vec<GotoPlaceObject>,
        is_rewind: bool,
        from_frame: FrameNumber,
    ) -> Result<(), Error> {
        let remove_object = if version == 1 {
            reader.read_remove_object_1()
        } else {
            reader.read_remove_object_2()
        }?;
        let depth: Depth = remove_object.depth.into();
        if let Some(i) = goto_commands.iter().position(|o| o.depth() == depth) {
            goto_commands.swap_remove(i);
        }
        if !is_rewind {
            let to_frame = self.current_frame;
            self.current_frame = from_frame;

            let child = self.child_by_depth(depth);

            if let Some(child) = child {
                let depth = child.depth();
                self.raw_container_mut().remove_child(depth);
            }
            self.current_frame = to_frame;
        }
        Ok(())
    }

    fn playing(&self) -> bool {
        self.flags.contains(MovieClipFlags::PLAYING)
    }
    pub fn set_playing(&mut self, playing: bool) {
        self.flags.set(MovieClipFlags::PLAYING, playing);
    }
    pub fn play(&mut self) {
        if self.total_frames > 1 {
            self.set_playing(true);
        }
    }

    pub fn stop(&mut self) {
        self.set_playing(false);
    }

    fn unqueue_adds(&mut self) -> Vec<(Depth, QueuedTag)> {
        let mut unqueued: Vec<_> = self
            .queued_tags
            .iter_mut()
            .filter_map(|(d, b)| b.unqueue_add().map(|b| (*d, b)))
            .collect();
        unqueued.sort_by(|(_, t1), (_, t2)| t1.tag_start.cmp(&t2.tag_start));
        for (depth, _tag) in unqueued.iter() {
            if matches!(self.queued_tags.get(depth), Some(QueuedTagList::None)) {
                self.queued_tags.remove(depth);
            }
        }
        unqueued
    }
    fn unqueue_removes(&mut self) -> Vec<(Depth, QueuedTag)> {
        let mut unqueued: Vec<_> = self
            .queued_tags
            .iter_mut()
            .filter_map(|(d, b)| b.unqueue_remove().map(|b| (*d, b)))
            .collect();
        unqueued.sort_by(|(_, t1), (_, t2)| t1.tag_start.cmp(&t2.tag_start));

        for (depth, _tag) in unqueued.iter() {
            if matches!(self.queued_tags.get(depth), Some(QueuedTagList::None)) {
                self.queued_tags.remove(depth);
            }
        }
        unqueued
    }
    pub fn tag_stream_len(&self) -> usize {
        self.swf.end - self.swf.start
    }
    pub fn total_bytes(self) -> i32 {
        // For a loaded SWF, returns the uncompressed size of the SWF.
        // Otherwise, returns the size of the tag list in the clip's DefineSprite tag.
        if self.is_root() {
            self.movie().uncompressed_len()
        } else {
            self.tag_stream_len() as i32
        }
    }

    pub fn goto_frame(&mut self, library: &mut MovieLibrary, frame: FrameNumber, stop: bool) {
        if stop {
            self.stop();
        } else {
            self.play();
        }

        let frame = frame.max(1);

        if frame != self.current_frame {
            self.run_goto(library, frame, false)
        }
    }

    pub fn determine_next_frame(&self) -> NextFrame {
        if self.current_frame < self.total_frames {
            NextFrame::Next
        } else if self.total_frames > 1 {
            NextFrame::First
        } else {
            NextFrame::Same
        }
    }

    pub fn query_movie_clip(&mut self, arg: &str) -> Option<&mut Self> {
        if self.name() == Some(arg) {
            return Some(self);
        } else {
            for (_, child) in self.raw_container_mut().display_objects_mut() {
                match child {
                    DisplayObject::MovieClip(movie_clip) => {
                        if movie_clip.name() == Some(arg) {
                            return Some(movie_clip);
                        } else {
                            return movie_clip.query_movie_clip(arg);
                        }
                    }
                    _ => {}
                }
            }
        }
        None
    }
}

impl TDisplayObject for MovieClip {
    fn base_mut(&mut self) -> &mut DisplayObjectBase {
        &mut self.base
    }

    fn base(&self) -> &DisplayObjectBase {
        &self.base
    }

    fn character_id(&self) -> CharacterId {
        self.id
    }

    fn as_child(self) -> Option<ChildContainer> {
        Some(self.container)
    }

    fn as_movie(&mut self) -> Option<MovieClip> {
        Some(self.clone())
    }

    fn enter_frame(&mut self, library: &mut MovieLibrary) {
        let is_playing = self.playing();
        let data = self.swf.clone();

        // let skip_frame = self.base().should_skip_next_enter_frame();

        for child in self.raw_container().render_list().iter().rev() {
            if let Some(display_object) = self
                .raw_container_mut()
                .display_objects_mut()
                .get_mut(child)
            {
                display_object.enter_frame(library);
            }

            // if skip_frame {
            //     child.base_mut().set_skip_next_enter_frame(true);
            // }
        }
        // if skip_frame {
        //     self.base_mut().set_skip_next_enter_frame(false);
        //     return;
        // }
        if self.movie().is_action_script_3() {
            if is_playing {
                self.run_frame_internal(library, true, true);
            }
            let place_actions = self.unqueue_adds();

            for (_, tag) in place_actions {
                let mut reader = data.read_from(tag.tag_start);
                let version = match tag.tag_type {
                    QueuedTagAction::Place(v) => v,
                    _ => unreachable!(),
                };
                if let Err(e) = self.place_object(library, &mut reader, version) {
                    info!("Error placing object: {:?}", e);
                }
            }
        }
    }

    fn movie(&self) -> Arc<SwfMovie> {
        self.swf.movie.clone()
    }
}

#[derive(Default, Debug, Eq, PartialEq, Clone, Copy)]
pub enum QueuedTagList {
    #[default]
    None,
    Add(QueuedTag),
    Remove(QueuedTag),
    RemoveThenAdd(QueuedTag, QueuedTag),
}

impl QueuedTagList {
    fn queue_add(&mut self, add_tag: QueuedTag) {
        let new = match self {
            QueuedTagList::None => QueuedTagList::Add(add_tag),
            QueuedTagList::Add(existing) => {
                // Flash player traces "Warning: Failed to place object at depth 1.",
                // so let's log a warning too.
                QueuedTagList::Add(*existing)
            }
            QueuedTagList::Remove(r) => QueuedTagList::RemoveThenAdd(*r, add_tag),
            QueuedTagList::RemoveThenAdd(r, _) => QueuedTagList::RemoveThenAdd(*r, add_tag),
        };

        *self = new;
    }

    fn queue_remove(&mut self, remove_tag: QueuedTag) {
        let new = match self {
            QueuedTagList::None => QueuedTagList::Remove(remove_tag),
            QueuedTagList::Add(_) => QueuedTagList::None,
            QueuedTagList::Remove(_) => QueuedTagList::Remove(remove_tag),
            QueuedTagList::RemoveThenAdd(r, _) => QueuedTagList::Remove(*r),
        };

        *self = new;
    }

    fn unqueue_add(&mut self) -> Option<QueuedTag> {
        let (new_queue, return_val) = match self {
            QueuedTagList::None => (QueuedTagList::None, None),
            QueuedTagList::Add(a) => (QueuedTagList::None, Some(*a)),
            QueuedTagList::Remove(r) => (QueuedTagList::Remove(*r), None),
            QueuedTagList::RemoveThenAdd(r, a) => (QueuedTagList::Remove(*r), Some(*a)),
        };

        *self = new_queue;

        return_val
    }

    fn unqueue_remove(&mut self) -> Option<QueuedTag> {
        let (new_queue, return_val) = match self {
            QueuedTagList::None => (QueuedTagList::None, None),
            QueuedTagList::Add(a) => (QueuedTagList::Add(*a), None),
            QueuedTagList::Remove(r) => (QueuedTagList::None, Some(*r)),
            QueuedTagList::RemoveThenAdd(r, a) => (QueuedTagList::Add(*a), Some(*r)),
        };

        *self = new_queue;

        return_val
    }
}

#[derive(Debug, Eq, PartialEq, Clone, Copy)]
pub struct QueuedTag {
    pub tag_type: QueuedTagAction,
    pub tag_start: u64,
}

/// The type of queued tag.
///
/// The u8 parameter is the tag version.
#[derive(Debug, Eq, PartialEq, Clone, Copy)]
pub enum QueuedTagAction {
    Place(u8),
    Remove(u8),
}

#[derive(Debug)]
pub struct GotoPlaceObject<'a> {
    frame: FrameNumber,

    place_object: swf::PlaceObject<'a>,

    index: usize,

    tag_start: u64,

    version: SwfVersion,
}

impl<'a> GotoPlaceObject<'a> {
    fn new(
        frame: FrameNumber,
        mut place_object: swf::PlaceObject<'a>,
        is_rewind: bool,
        index: usize,
        tag_start: u64,
        version: SwfVersion,
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
            tag_start,
            version,
        }
    }

    #[inline]
    fn depth(&self) -> Depth {
        self.place_object.depth.into()
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
