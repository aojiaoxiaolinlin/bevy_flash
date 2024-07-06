use std::{collections::HashMap, f32::consts::E, sync::Arc};

use anyhow::anyhow;
use swf::{
    error::Error, extensions::ReadSwfExt, read::Reader, CharacterId, Depth, PlaceObject,
    PlaceObjectAction, SwfStr, TagCode,
};

use crate::flash_utils::{
    characters::Character,
    container::ChildContainer,
    library::MovieLibrary,
    util::{self, ControlFlow, SwfMovie, SwfSlice},
};

use super::{graphic::Graphic, DisplayObjectBase, TDisplayObject};

type FrameNumber = u16;

#[derive(Clone)]
pub struct MovieClip {
    pub id: CharacterId,
    base: DisplayObjectBase,
    swf_slice: SwfSlice,
    current_frame: FrameNumber,
    total_frames: FrameNumber,
    frame_labels: Vec<(FrameNumber, String)>,
    container: ChildContainer,
    // flags: MovieClipFlags,
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
            // flags: MovieClipFlags::empty(),
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
            // flags: MovieClipFlags::empty(),
            tag_stream_pos: 0,
            // drawing: Drawing::new(),
            queued_tags: HashMap::new(),
        }
    }

    fn container_mut(&mut self) -> &mut ChildContainer {
        &mut self.container
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
        let _ = util::decode_tags(&mut reader, tag_callback);
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
        let graphic = Graphic::from_swf_tag(swf_shape);
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
        let swf_slice = self.swf_slice.clone();

        let mut reader = swf_slice.read_from(self.tag_stream_pos);

        let tag_callback = |reader: &mut Reader<'_>, tag_code, tag_len| {
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
                TagCode::PlaceObject if is_action_script_3 => {
                    self.queue_place_object(library, reader, 1)
                }
                TagCode::PlaceObject2 if is_action_script_3 => {
                    self.queue_place_object(library, reader, 2)
                }
                TagCode::PlaceObject3 if is_action_script_3 => {
                    self.queue_place_object(library, reader, 3)
                }
                TagCode::PlaceObject4 if is_action_script_3 => {
                    self.queue_place_object(library, reader, 4)
                }
                // TagCode::RemoveObject => self.remove_object(reader),
                // TagCode::RemoveObject2 => self.remove_object2(reader),
                TagCode::ShowFrame => return Ok(ControlFlow::Exit),
                _ => Ok(()),
            }?;
            Ok(ControlFlow::Continue)
        };

        let _ = util::decode_tags(&mut reader, tag_callback);
        let tag_stream_start = self.swf_slice.as_ref().as_ptr() as u64;
        self.tag_stream_pos = reader.get_ref().as_ptr() as u64 - tag_stream_start;
    }

    fn place_object(
        &mut self,
        library: &mut MovieLibrary,
        reader: &mut Reader<'_>,
        version: u8,
    ) -> Result<(), Error> {
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

    fn queue_place_object(
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
                self.instantiate_child(library, id, &place_object);
            }
            _ => {}
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
        place_object: &PlaceObject,
    ) {
        let child = self.instantiate_by_id(id, library);
        match child {
            Ok(mut child) => {
                child.set_depth(place_object.depth);
                child.set_place_frame(self.current_frame);
                child.apply_place_object(&place_object, self.swf_slice.version());
                if let Some(name) = place_object.name {
                    let name = name
                        .to_str_lossy(SwfStr::encoding_for_version(self.swf_slice.version()))
                        .to_string();
                    child.set_name(Some(name));
                }
                if let Some(clip_depth) = place_object.clip_depth {
                    child.set_clip_depth(clip_depth);
                }
                self.replace_at_depth(place_object.depth, child);
            }
            Err(e) => {
                dbg!(e);
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
    fn replace_at_depth(&mut self, depth: Depth, mut child: Box<dyn TDisplayObject>) {
        child.set_place_frame(0);
        child.set_depth(depth);
        let removed_child = self.container_mut().replace_at_depth(depth, child);
        if let Some(_removed_child) = removed_child {
            todo!("remove child")
        }
    }
}

impl TDisplayObject for MovieClip {
    fn enter_frame(&mut self, library: &mut MovieLibrary) {
        let swf_slice = self.swf_slice.clone();

        for child in self
            .container_mut()
            .render_list_mut()
            .write()
            .unwrap()
            .iter_mut()
        {
            child.write().unwrap().enter_frame(library);
        }

        if self.swf_slice.movie().is_action_script_3() {
            self.run_frame_internal(library, true);

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
