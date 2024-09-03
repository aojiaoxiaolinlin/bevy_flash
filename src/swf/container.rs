use std::{
    collections::BTreeMap,
    ops::Bound,
    sync::{mpsc::RecvError, Arc},
};

use swf::Depth;

use super::display_object::{movie_clip::MovieClip, DisplayObject, TDisplayObject};

type DisplayId = u16;

#[derive(Clone)]
pub struct ChildContainer {
    render_list: Arc<Vec<DisplayId>>,
    depth_list: BTreeMap<Depth, DisplayId>,

    display_objects: BTreeMap<DisplayId, DisplayObject>,
}

impl ChildContainer {
    pub fn new() -> Self {
        Self {
            render_list: Arc::new(Vec::new()),
            depth_list: BTreeMap::new(),
            display_objects: BTreeMap::new(),
        }
    }

    pub fn render_list_len(&self) -> usize {
        self.render_list.len()
    }

    pub fn render_list(&self) -> Arc<Vec<DisplayId>> {
        self.render_list.clone()
    }

    pub fn render_list_mut(&mut self) -> &mut Vec<DisplayId> {
        Arc::make_mut(&mut self.render_list)
    }

    pub fn child_by_depth(&mut self, depth: Depth) -> Option<&mut DisplayObject> {
        let display_object_id = self.depth_list.get_mut(&depth);
        if let Some(display_object_id) = display_object_id {
            self.display_objects.get_mut(display_object_id)
        } else {
            None
        }
    }

    pub fn display_objects(&self) -> &BTreeMap<DisplayId, DisplayObject> {
        &self.display_objects
    }

    pub fn display_objects_mut(&mut self) -> &mut BTreeMap<DisplayId, DisplayObject> {
        &mut self.display_objects
    }

    fn insert_child_into_depth_list(
        &mut self,
        depth: Depth,
        child: DisplayObject,
    ) -> (Option<DisplayId>, DisplayId) {
        let display_object_id = self.display_objects.len() as DisplayId;
        self.display_objects.insert(display_object_id, child);
        let prev_child = self.depth_list.insert(depth, display_object_id);
        (prev_child, display_object_id)
    }

    fn insert_id(&mut self, id: usize, child: DisplayId) {
        self.render_list_mut().insert(id, child);
    }

    fn push_id(&mut self, child: DisplayId) {
        self.render_list_mut().push(child);
    }

    pub fn remove_child_from_depth_list(&mut self, child: Depth) {
        if let Some(_other_child) = self.depth_list.get(&child) {
            let remove = self.depth_list.remove(&child);
            if let Some(remove) = remove {
                self.display_objects.remove(&remove);
            }
        }
    }

    pub fn remove_child(&mut self, child: DisplayId) {
        self.remove_child_from_depth_list(child);
        Self::remove_child_from_render_list(self, child);
    }

    fn remove_child_from_render_list(container: &mut ChildContainer, child: DisplayId) -> bool {
        let render_list_position = container.render_list.iter().position(|x| *x == child);
        if let Some(position) = render_list_position {
            container.render_list_mut().remove(position);
            true
        } else {
            false
        }
    }

    pub fn replace_at_depth(&mut self, depth: Depth, child: DisplayObject) {
        let (prev_child, child_display_object_id) =
            self.insert_child_into_depth_list(depth, child.clone());
        if let Some(prev_child) = prev_child {
            if let Some(position) = self.render_list.iter().position(|x| *x == prev_child) {
                dbg!("目前不会执行到这里");
                // self.insert_id(position + 1, child_display_object_id);
            }
        } else {
            let above = self
                .depth_list
                .range((Bound::Excluded(depth), Bound::Unbounded))
                .map(|(_, v)| *v)
                .next();
            if let Some(above_child) = above {
                if let Some(position) = self.render_list.iter().position(|x| *x == above_child) {
                    self.insert_id(position, child_display_object_id);
                } else {
                    self.push_id(child_display_object_id)
                }
            } else {
                self.push_id(child_display_object_id)
            }
        }
    }
}
