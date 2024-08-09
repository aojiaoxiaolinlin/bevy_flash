use std::{collections::BTreeMap, ops::Bound, sync::Arc};

use swf::Depth;

use super::display_object::{movie_clip::MovieClip, DisplayObject, TDisplayObject};

#[derive(Clone)]
pub struct ChildContainer {
    render_list: Arc<Vec<DisplayObject>>,
    depth_list: BTreeMap<Depth, DisplayObject>,
}

impl ChildContainer {
    pub fn new() -> Self {
        Self {
            render_list: Arc::new(Vec::new()),
            depth_list: BTreeMap::new(),
        }
    }

    pub fn render_list_len(&self) -> usize {
        self.render_list.len()
    }

    pub fn render_list_mut(&mut self) -> &mut Vec<DisplayObject> {
        Arc::make_mut(&mut self.render_list)
    }

    pub fn child_by_depth(&self, depth: Depth) -> Option<DisplayObject> {
        self.depth_list.get(&depth).cloned()
    }

    fn insert_child_into_depth_list(
        &mut self,
        depth: Depth,
        child: DisplayObject,
    ) -> Option<DisplayObject> {
        self.depth_list.insert(depth, child)
    }

    fn insert_id(&mut self, id: usize, child: DisplayObject) {
        self.render_list_mut().insert(id, child);
    }

    pub fn replace_at_depth(&mut self, depth: Depth, child: DisplayObject) {
        let prev_child = self.insert_child_into_depth_list(depth, child.clone());
        if let Some(prev_child) = prev_child {
            if let Some(position) = self
                .render_list
                .iter()
                .position(|x| x.character_id() == prev_child.character_id())
            {
                dbg!("目前不会执行到这里");
                self.render_list_mut()[position] = child;
            }
        } else {
            let above = self
                .depth_list
                .range((Bound::Excluded(depth), Bound::Unbounded))
                .map(|(_, v)| v.clone())
                .next();
            if let Some(above_child) = above {
                if let Some(position) = self
                    .render_list
                    .iter()
                    .position(|x| x.character_id() == above_child.character_id())
                {
                    self.insert_id(position, child);
                } else {
                    self.render_list_mut().push(child)
                }
            } else {
                self.render_list_mut().push(child)
            }
        }
    }
}

pub struct RenderIter {
    src: Arc<Vec<DisplayObject>>,
    i: usize,
    neg_i: usize,
}
impl RenderIter {
    pub fn from_container(container: MovieClip) -> Self {
        Self {
            src: container.raw_container().render_list.clone(),
            i: 0,
            neg_i: container.num_children(),
        }
    }
}

impl Iterator for RenderIter {
    type Item = DisplayObject;
    fn next(&mut self) -> Option<Self::Item> {
        if self.i == self.neg_i {
            return None;
        }

        let this = self.src.get(self.i).cloned();
        self.i += 1;
        this
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.neg_i - self.i;
        (len, Some(len))
    }
}

impl DoubleEndedIterator for RenderIter {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.i == self.neg_i {
            return None;
        }

        self.neg_i -= 1;
        self.src.get(self.neg_i).cloned()
    }
}
