use std::{collections::HashMap, sync::Arc};

use anyhow::anyhow;
use bitflags::bitflags;

use swf::{
    extensions::ReadSwfExt, read::Reader, CharacterId, Depth, PlaceObjectAction, SwfStr, TagCode,
};

use crate::flash_utils::{
    characters::Character,
    container::{ChildContainer, RenderIter},
    library::MovieLibrary,
    tag_utils::{self, ControlFlow, Error, SwfMovie, SwfSlice, SwfStream},
};

use super::{graphic::Graphic, DisplayObject, DisplayObjectBase, TDisplayObject};

type FrameNumber = u16;
type SwfVersion = u8;
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
    current_frame: FrameNumber,
    pub total_frames: FrameNumber,
    frame_labels: Vec<(FrameNumber, String)>,
    container: ChildContainer,
    flags: MovieClipFlags,
    tag_stream_pos: u64,
    queued_tags: HashMap<Depth, QueuedTagList>,
}

impl MovieClip {
    pub fn new(movie: Arc<SwfMovie>) -> Self {
        Self {
            base: DisplayObjectBase::default(),
            id: Default::default(),
            current_frame: Default::default(),
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
            current_frame: Default::default(),
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

    pub fn iter_render_list(self) -> RenderIter {
        RenderIter::from_container(self.into())
    }

    pub fn num_children(&self) -> usize {
        self.container.render_list_len()
    }

    fn child_by_depth(&self, depth: Depth) -> Option<DisplayObject> {
        self.container.child_by_depth(depth)
    }

    pub fn parse_swf(&mut self, library: &mut MovieLibrary) {
        let swf = self.swf.clone();
        let mut reader = Reader::new(&swf.data()[..], swf.version());
        let tag_callback = |reader: &mut SwfStream<'_>, tag_code, tag_len| {
            match tag_code {
                // TagCode::SetBackgroundColor => self.set_background_color(library, reader),
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

    fn frame_label(&mut self, reader: &mut SwfStream) -> Result<(), Error> {
        let frame_label = reader.read_frame_label()?;
        let label = frame_label
            .label
            .to_str_lossy(SwfStr::encoding_for_version(self.swf.version()));
        self.frame_labels
            .push((self.current_frame, label.into_owned()));
        Ok(())
    }

    fn show_frame(&mut self) -> Result<(), Error> {
        self.current_frame += 1;
        Ok(())
    }

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

    pub fn run_frame_internal(&mut self, library: &mut MovieLibrary, is_action_script_3: bool) {
        let next_frame = self.determine_next_frame();
        match next_frame {
            NextFrame::Next => {}
            NextFrame::First => {
                self.current_frame = 1;
                dbg!("first frame");
            }
            NextFrame::Same => {
                dbg!("same frame");
            }
        }
        let data = self.swf.clone();
        let mut reader = data.read_from(self.tag_stream_pos);
        let tag_callback = |reader: &mut SwfStream<'_>, tag_code, _tag_len| {
            match tag_code {
                TagCode::PlaceObject if !is_action_script_3 => {
                    self.place_object(library, reader, 1)
                }
                TagCode::PlaceObject2 if !is_action_script_3 => {
                    self.place_object(library, reader, 2)
                }
                TagCode::PlaceObject3 if !is_action_script_3 => {
                    self.place_object(library, reader, 3)
                }
                TagCode::PlaceObject4 if !is_action_script_3 => {
                    self.place_object(library, reader, 4)
                }
                TagCode::PlaceObject if is_action_script_3 => self.queue_place_object(reader, 1),
                TagCode::PlaceObject2 if is_action_script_3 => self.queue_place_object(reader, 2),
                TagCode::PlaceObject3 if is_action_script_3 => self.queue_place_object(reader, 3),
                TagCode::PlaceObject4 if is_action_script_3 => self.queue_place_object(reader, 4),

                // TagCode::SetBackgroundColor => self.set_background_color(library, reader),
                TagCode::ShowFrame => return Ok(ControlFlow::Exit),
                _ => Ok(()),
            }?;
            Ok(ControlFlow::Continue)
        };
        let _ = tag_utils::decode_tags(&mut reader, tag_callback);
        let tag_stream_start = self.swf.as_ref().as_ptr() as u64;
        self.tag_stream_pos = reader.get_ref().as_ptr() as u64 - tag_stream_start;
        if matches!(next_frame, NextFrame::Next) {
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
                self.instantiate_child(id, place_object.depth, &place_object, library);
            }
            PlaceObjectAction::Replace(id) => {
                if let Some(mut child) = self.child_by_depth(place_object.depth.into()) {
                    child.replace_with(id, library);
                    child.apply_place_object(&place_object, self.swf.version());
                    child.set_place_frame(self.current_frame);
                }
            }
            PlaceObjectAction::Modify => {
                if let Some(mut child) = self.child_by_depth(place_object.depth.into()) {
                    child.apply_place_object(&place_object, self.swf.version());
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
                child.set_depth(place_object.depth);
                child.set_place_frame(self.current_frame);
                child.apply_place_object(&place_object, self.swf.version());
                if let Some(name) = &place_object.name {
                    child.set_name(Some(
                        name.to_str_lossy(SwfStr::encoding_for_version(self.swf.version()))
                            .into_owned(),
                    ));
                }
                if let Some(clip_depth) = place_object.clip_depth {
                    child.set_clip_depth(clip_depth);
                }
                // child.post_instantiation(library);
                self.replace_at_depth(place_object.depth, child.clone());
                Some(child)
            }
            Err(e) => {
                dbg!(e);
                None
            }
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
            }
        } else {
            Err(anyhow!("Character id 不在库中"))
        }
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
    fn determine_next_frame(&self) -> NextFrame {
        if self.current_frame < self.total_frames {
            NextFrame::Next
        } else if self.total_frames > 1 {
            NextFrame::First
        } else {
            NextFrame::Same
        }
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

        for child in self.raw_container_mut().render_list_mut().iter_mut().rev() {
            // if skip_frame {
            //     child.base_mut().set_skip_next_enter_frame(true);
            // }
            child.enter_frame(library);
        }
        // if skip_frame {
        //     self.base_mut().set_skip_next_enter_frame(false);
        //     return;
        // }
        if self.movie().is_action_script_3() {
            if is_playing {
                self.run_frame_internal(library, true);
            }
            let place_actions = self.unqueue_adds();

            for (_, tag) in place_actions {
                let mut reader = data.read_from(tag.tag_start);
                let version = match tag.tag_type {
                    QueuedTagAction::Place(v) => v,
                    _ => unreachable!(),
                };
                if let Err(e) = self.place_object(library, &mut reader, version) {
                    dbg!("Error placing object: {:?}", e);
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
