use anyhow::anyhow;
use bitflags::bitflags;
use smallvec::SmallVec;
use std::{
    cmp::max,
    collections::HashMap,
    sync::{Arc, RwLock},
};
use swf::{
    extensions::ReadSwfExt, read::Reader, CharacterId, Color, Depth, PlaceObject,
    PlaceObjectAction, SwfStr, TagCode,
};

use crate::flash_utils::{
    characters::Character,
    container::ChildContainer,
    library::MovieLibrary,
    tag_utils::{self, ControlFlow, Error, SwfMovie, SwfSlice},
};

use super::{graphic::Graphic, DisplayObjectBase, TDisplayObject};

type FrameNumber = u16;
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
    pub id: CharacterId,
    base: DisplayObjectBase,
    swf_slice: SwfSlice,
    current_frame: FrameNumber,
    total_frames: FrameNumber,
    frame_labels: Vec<(FrameNumber, String)>,
    container: ChildContainer,
    flags: MovieClipFlags,
    tag_stream_pos: u64,
    // drawing: Drawing,
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
            swf_slice: SwfSlice::empty(movie),
            container: ChildContainer::new(),
            flags: MovieClipFlags::PLAYING,
            // drawing: Drawing::new(),
            tag_stream_pos: 0,
            queued_tags: HashMap::new(),
        }
    }

    pub fn new_with_data(id: CharacterId, total_frames: FrameNumber, swf_slice: SwfSlice) -> Self {
        Self {
            base: DisplayObjectBase::default(),
            id,
            total_frames,
            current_frame: Default::default(),
            frame_labels: Default::default(),
            swf_slice,
            container: ChildContainer::new(),
            flags: MovieClipFlags::PLAYING,
            tag_stream_pos: 0,
            // drawing: Drawing::new(),
            queued_tags: HashMap::new(),
        }
    }

    fn set_playing(&mut self, value: bool) {
        self.flags.set(MovieClipFlags::PLAYING, value);
    }

    fn playing(&self) -> bool {
        self.flags.contains(MovieClipFlags::PLAYING)
    }

    pub fn container_mut(&mut self) -> &mut ChildContainer {
        &mut self.container
    }
    pub fn container(&self) -> &ChildContainer {
        &self.container
    }

    pub fn parse_swf(&mut self, library: &mut MovieLibrary) {
        let swf = self.swf_slice.clone();
        let mut reader = Reader::new(&swf.data()[..], swf.version());
        let tag_callback = |reader: &mut Reader<'_>, tag_code, tag_len| {
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
        // let _ = util::decode_tags(&mut reader, tag_callback);
        let _ = tag_utils::decode_tags(&mut reader, tag_callback);
    }
    #[inline]
    fn define_shape(
        &mut self,
        library: &mut MovieLibrary,
        reader: &mut Reader<'_>,
        version: u8,
    ) -> Result<(), Error> {
        let swf_shape = reader.read_define_shape(version)?;
        let id = swf_shape.id;
        let graphic = Graphic::from_swf_tag(swf_shape, self.movie().clone());
        library.register_character(id, Character::Graphic(graphic));
        Ok(())
    }

    #[inline]
    fn define_sprite(
        &mut self,
        library: &mut MovieLibrary,
        reader: &mut Reader<'_>,
        tag_len: usize,
    ) -> Result<ControlFlow, Error> {
        let start = reader.as_slice();
        let id = reader.read_character_id()?;
        let num_frames = reader.read_u16()?;
        let num_read = reader.pos(start);
        let mut movie_clip = MovieClip::new_with_data(
            id,
            num_frames,
            self.swf_slice.resize_to_reader(reader, tag_len - num_read),
        );
        movie_clip.parse_swf(library);
        library.register_character(id, Character::MovieClip(movie_clip));
        Ok(ControlFlow::Continue)
    }

    #[inline]
    fn frame_label(&mut self, reader: &mut Reader<'_>) -> Result<(), Error> {
        let frame_label = reader.read_frame_label()?;
        let label = frame_label
            .label
            .to_str_lossy(SwfStr::encoding_for_version(self.swf_slice.version()));
        self.frame_labels
            .push((self.current_frame, label.into_owned()));
        Ok(())
    }
    #[inline]
    fn show_frame(&mut self) -> Result<(), Error> {
        self.current_frame += 1;
        Ok(())
    }

    pub fn run_frame_internal(&mut self, library: &mut MovieLibrary, is_action_script_3: bool) {
        let next_frame = self.determine_next_frame();
        match next_frame {
            NextFrame::Next => {
                dbg!("NextFrame::Next");
            }
            NextFrame::First => {
                dbg!("NextFrame::First");
                // return self.run_goto(library, 1, true);
            }
            NextFrame::Same => {
                dbg!("NextFrame::Same");
                self.stop();
            }
        }
        let swf_slice = self.swf_slice.clone();
        let mut reader = swf_slice.read_from(self.tag_stream_pos);
        dbg!(self.current_frame);
        let tag_callback = |reader: &mut Reader<'_>, tag_code, _tag_len| {
            match tag_code {
                // TagCode::DoAction => self.do_action(reader, tag_len, is_action_script_3),
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
                TagCode::RemoveObject => self.remove_object(reader, 1),
                TagCode::RemoveObject2 => self.remove_object(reader, 2),
                TagCode::ShowFrame => return Ok(ControlFlow::Exit),
                _ => Ok(()),
            }?;
            Ok(ControlFlow::Continue)
        };

        let _ = tag_utils::decode_tags(&mut reader, tag_callback);
        let tag_stream_start = self.swf_slice.as_ref().as_ptr() as u64;
        self.tag_stream_pos = reader.get_ref().as_ptr() as u64 - tag_stream_start;
    }

    #[inline]
    fn queue_place_object(&mut self, reader: &mut Reader<'_>, version: u8) -> Result<(), Error> {
        let tag_start = reader.get_ref().as_ptr() as u64 - self.swf_slice.as_ref().as_ptr() as u64;
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
    fn place_object(
        &mut self,
        library: &mut MovieLibrary,
        reader: &mut Reader<'_>,
        version: u8,
    ) -> Result<(), Error> {
        let place_object = if version == 1 {
            reader.read_place_object()
        } else {
            reader.read_place_object_2_or_3(version)
        }?;
        match place_object.action {
            PlaceObjectAction::Place(id) => {
                self.instantiate_child(library, id, place_object.depth, &place_object);
            }
            _ => {}
        }
        Ok(())
    }

    #[inline]
    fn remove_object(&mut self, reader: &mut Reader<'_>, version: u8) -> Result<(), Error> {
        let remove_object = if version == 1 {
            reader.read_remove_object_1()
        } else {
            reader.read_remove_object_2()
        }?;
        if let Some(child) = self.child_by_depth(remove_object.depth.into()) {
            self.remove_child(child);
        }
        Ok(())
    }

    fn unqueue_adds(&mut self) -> Vec<(Depth, QueuedTag)> {
        let mut unqueued: Vec<_> = self
            .queued_tags
            .iter_mut()
            .filter_map(|(depth, queue_tag_list)| {
                queue_tag_list
                    .unqueue_add()
                    .map(|queue_tag| (*depth, queue_tag))
            })
            .collect();
        unqueued.sort_by(|(_, t1), (_, t2)| t1.tag_start.cmp(&t2.tag_start));

        for (depth, _tag) in unqueued.iter() {
            if matches!(self.queued_tags.get(depth), Some(QueuedTagList::None)) {
                self.queued_tags.remove(depth);
            }
        }
        unqueued
    }

    fn instantiate_child(
        &mut self,
        library: &mut MovieLibrary,
        id: CharacterId,
        depth: Depth,
        place_object: &PlaceObject,
    ) -> Option<Arc<RwLock<Box<dyn TDisplayObject>>>> {
        let child = self.instantiate_by_id(id, library);
        match child {
            Ok(mut child) => {
                child.set_depth(depth);
                child.set_place_frame(self.current_frame);
                child.apply_place_object(&place_object);
                if let Some(name) = place_object.name {
                    let name = name
                        .to_str_lossy(SwfStr::encoding_for_version(self.swf_slice.version()))
                        .to_string();
                    child.set_name(Some(name));
                }
                if let Some(clip_depth) = place_object.clip_depth {
                    child.set_clip_depth(clip_depth);
                }
                let child = Arc::new(RwLock::new(child));
                self.replace_at_depth(depth, child.clone());
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
        library: &MovieLibrary,
    ) -> anyhow::Result<Box<dyn TDisplayObject>> {
        if let Some(character) = library.character(id) {
            match character {
                Character::MovieClip(movie_clip) => Ok(Box::new(movie_clip.clone())),
                Character::Graphic(graphic) => Ok(Box::new(graphic.clone())),
            }
        } else {
            Err(anyhow!("Character Id 不存在"))
        }
    }
    /// 在深度列表的特定位置向容器中插入一个子显示对象，并移除已在该位置上的任何子对象。
    /// 将子对象插入深度列表后，我们将尝试为其分配一个呈现列表位置，该位置在深度列表中前一个项目之后。子代放入呈现列表的位置与 Flash Player 的行为一致。
    /// 从深度列表中移除的任何子代也将从呈现列表中移除，前提是该子代未被标记为由脚本放置。如果该子元素已从上述列表中移除，则将在此返回。否则，此方法将返回 "无"。
    /// 注意：此方法不会对其修改的任何子对象分派事件。这必须由您自己来完成。
    fn replace_at_depth(&mut self, depth: Depth, mut child: Arc<RwLock<Box<dyn TDisplayObject>>>) {
        child.write().unwrap().set_place_frame(0);
        child.write().unwrap().set_depth(depth);
        let removed_child = self.container_mut().replace_at_depth(depth, child);
        if let Some(_removed_child) = removed_child {
            todo!("remove child")
        }
    }

    pub fn run_goto(&mut self, library: &mut MovieLibrary, frame: FrameNumber, is_implicit: bool) {
        self.base_mut().set_skip_next_enter_frame(false);

        let mut goto_commands: Vec<GotoPlaceObject<'_>> = vec![];

        let is_rewind = if frame <= self.current_frame {
            self.tag_stream_pos = 0;
            self.current_frame = 0;
            true
        } else {
            false
        };
        let from_frame = self.current_frame;
        if self.loop_queued() {
            self.queued_tags = HashMap::new();
        }

        if is_implicit {
            self.set_loop_queued();
        }

        // 逐步浏览中间帧，并汇总每个帧的三角积分。
        let tag_stream_start = self.swf_slice.as_ref().as_ptr() as u64;
        let mut frame_pos = self.tag_stream_pos;
        let swf_slice = self.swf_slice.clone();
        let mut index = 0;
        // 理智；确保我们不会走得太远。
        let clamped_frame = frame.min(max(self.total_frames - 1, 0) as FrameNumber);

        let mut reader = swf_slice.read_from(frame_pos);
        while self.current_frame < clamped_frame && !reader.get_ref().is_empty() {
            self.current_frame += 1;

            frame_pos = reader.get_ref().as_ptr() as u64 - tag_stream_start;

            let tag_callback = |reader: &mut _, tag_code: TagCode, _tag_len| {
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
                    TagCode::ShowFrame => return Ok(ControlFlow::Exit),
                    _ => Ok(()),
                }?;
                Ok(ControlFlow::Continue)
            };
            let _ = tag_utils::decode_tags(&mut reader, tag_callback);
            if let Err(e) = self.run_abc_and_symbol_tags(self.current_frame - 1) {
                dbg!(e);
            }
        }
        let hit_target_frame = self.current_frame == frame;

        if is_rewind {
            let children = self.container().render_list_read().clone();
            let children: SmallVec<[_; 16]> = children
                .iter()
                .filter(|clip| clip.read().unwrap().place_frame() > frame)
                .collect();
            for child in children {
                self.remove_child(child.clone());
            }
        }
        let swf_movie = self.swf_slice.movie.clone();
        let queue_tags = &mut self.queued_tags.clone();
        // 运行 goto 命令列表，实际创建和更新显示对象。
        let mut run_goto_command = |clip: &mut MovieClip, params: &GotoPlaceObject| {
            let child_entry = clip.child_by_depth(params.depth());
            if swf_movie.is_action_script_3() && is_implicit && child_entry.is_none() {
                let new_tag = QueuedTag {
                    tag_type: QueuedTagAction::Place(params.version),
                    tag_start: params.tag_start,
                };
                let bucket = queue_tags
                    .entry(params.place_object.depth as Depth)
                    .or_insert_with(|| QueuedTagList::None);
                bucket.queue_add(new_tag);
                return;
            }
            match (params.place_object.action, child_entry, is_rewind) {
                (_, Some(prev_child), true) | (PlaceObjectAction::Modify, Some(prev_child), _) => {
                    prev_child
                        .write()
                        .unwrap()
                        .apply_place_object(&params.place_object);
                }
                (swf::PlaceObjectAction::Replace(id), Some(prev_child), _) => {
                    prev_child.write().unwrap().replace_with(id);
                    prev_child
                        .write()
                        .unwrap()
                        .apply_place_object(&params.place_object);
                    prev_child.write().unwrap().set_place_frame(params.frame);
                }
                (PlaceObjectAction::Place(id), _, _)
                | (swf::PlaceObjectAction::Replace(id), _, _) => {
                    if let Some(child) = clip.instantiate_child(
                        &mut library.clone(),
                        id,
                        params.depth(),
                        &params.place_object,
                    ) {
                        // Set the place frame to the frame where the object *would* have been placed.
                        child.write().unwrap().set_place_frame(params.frame);
                    }
                }
                _ => {
                    dbg!("未处理的情况");
                }
            }
        };
        goto_commands.sort_by_key(|params| params.index);
        goto_commands
            .iter()
            .filter(|params| params.frame < frame)
            .for_each(|goto| run_goto_command(self, goto));

        if hit_target_frame {
            self.current_frame -= 1;
            self.tag_stream_pos = frame_pos;

            self.run_frame_internal(&mut library.clone(), self.movie().is_action_script_3());
        } else {
            self.current_frame = clamped_frame;
        }
        goto_commands
            .iter()
            .filter(|params| params.frame >= frame)
            .for_each(|goto| run_goto_command(self, goto));

        if !is_implicit {
            self.base_mut().set_skip_next_enter_frame(false);
        }
    }

    fn play(&mut self) {
        // Can only play clips with multiple frames.
        if self.total_frames > 1 {
            self.set_playing(true);
        }
    }

    fn stop(&mut self) {
        self.set_playing(false);
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
        let tag_start = reader.get_ref().as_ptr() as u64 - self.swf_slice.as_ref().as_ptr() as u64;
        let place_object = if version == 1 {
            reader.read_place_object()
        } else {
            reader.read_place_object_2_or_3(version)
        }?;
        // 我们将该 PlaceObject 的三角积分与上一条命令合并。
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

    fn run_abc_and_symbol_tags(&mut self, current_frame: FrameNumber) -> Result<(), Error> {
        Ok(())
    }

    fn loop_queued(&self) -> bool {
        self.flags.contains(MovieClipFlags::LOOP_QUEUED)
    }
    fn set_loop_queued(&mut self) {
        self.flags |= MovieClipFlags::LOOP_QUEUED;
    }

    fn remove_child(&mut self, child: Arc<RwLock<Box<dyn TDisplayObject>>>) {
        self.container_mut().remove_child_from_depth_list(child);
    }

    fn child_by_depth(&self, depth: Depth) -> Option<Arc<RwLock<Box<dyn TDisplayObject>>>> {
        self.container().get_depth(depth)
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
    fn enter_frame(&mut self, library: &mut MovieLibrary) {
        let skip_frame = self.base().should_skip_next_enter_frame();
        let swf_slice = self.swf_slice.clone();
        for child in self.container_mut().render_list_write().iter_mut().rev() {
            if skip_frame {
                child
                    .write()
                    .unwrap()
                    .base_mut()
                    .set_skip_next_enter_frame(true);
            }
            child.write().unwrap().enter_frame(library);
        }

        if skip_frame {
            self.base_mut().set_skip_next_enter_frame(false);
            return;
        }

        if self.swf_slice.movie.is_action_script_3() {
            let is_playing = self.playing();
            if is_playing {
                self.run_frame_internal(library, true);
            }

            let place_actions = self.unqueue_adds();

            for (_, tag) in place_actions {
                let mut reader = swf_slice.read_from(tag.tag_start);
                let version = match tag.tag_type {
                    QueuedTagAction::Place(v) => v,
                    _ => unreachable!(), // 不可能出现
                };
                if let Err(e) = self.place_object(library, &mut reader, version) {
                    dbg!(e);
                }
            }
        }
    }

    fn base_mut(&mut self) -> &mut DisplayObjectBase {
        &mut self.base
    }

    fn base(&self) -> &DisplayObjectBase {
        &self.base
    }

    fn character_id(&self) -> CharacterId {
        self.id
    }

    fn movie(&self) -> Arc<SwfMovie> {
        self.swf_slice.movie.clone()
    }
}

#[derive(Debug, Eq, PartialEq, Clone, Copy)]
pub struct QueuedTag {
    pub tag_type: QueuedTagAction,
    pub tag_start: u64,
}

#[derive(Debug, Eq, PartialEq, Clone, Copy)]
pub enum QueuedTagAction {
    Place(u8),
    Remove(u8),
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
                // so let's log a warning too.
                // tracing::warn!("Ignoring queued tag {add_tag:?} at same depth as {existing:?}");
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

#[derive(PartialEq, Eq)]
enum NextFrame {
    /// Construct and run the next frame in the clip.
    Next,

    /// Jump to the first frame in the clip.
    First,

    /// Do not construct or run any frames.
    Same,
}

/// Stores the placement settings for display objects during a
/// goto command.
#[derive(Debug)]
struct GotoPlaceObject<'a> {
    /// The frame number that this character was first placed on.
    frame: FrameNumber,
    /// The display properties of the object.
    place_object: swf::PlaceObject<'a>,
    /// Increasing index of this place command, for sorting.
    index: usize,

    /// The location of the *first* SWF tag that created this command.
    ///
    /// NOTE: Only intended to be used in looping gotos, where tag merging is
    /// not possible and we want to add children after the goto completes.
    tag_start: u64,

    /// The version of the PlaceObject tag at `tag_start`.
    version: u8,
}

impl<'a> GotoPlaceObject<'a> {
    fn new(
        frame: FrameNumber,
        mut place_object: swf::PlaceObject<'a>,
        is_rewind: bool,
        index: usize,
        tag_start: u64,
        version: u8,
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
                // Purposely omitted properties:
                // name, clip_depth, clip_actions, amf_data
                // These properties are only set on initial placement in `MovieClip::instantiate_child`
                // and can not be modified by subsequent PlaceObject tags.
                // Also, is_visible flag persists during rewind unlike all other properties.
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
        use swf::PlaceObjectAction;
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
        // Purposely omitted properties:
        // name, clip_depth, clip_actions, amf_data
        // These properties are only set on initial placement in `MovieClip::instantiate_child`
        // and can not be modified by subsequent PlaceObject tags.
    }
}
