use std::{
    collections::BTreeMap,
    ops::Bound,
    sync::{Arc, RwLock, RwLockWriteGuard},
};

use swf::Depth;

use crate::flash_utils::display_object::TDisplayObject;

#[derive(Clone)]
pub struct ChildContainer {
    render_list: Arc<RwLock<Vec<Arc<RwLock<Box<dyn TDisplayObject>>>>>>,
    depth_list: BTreeMap<Depth, Arc<RwLock<Box<dyn TDisplayObject>>>>,
}

impl ChildContainer {
    pub fn new() -> Self {
        Self {
            render_list: Arc::new(RwLock::new(Vec::new())),
            depth_list: BTreeMap::new(),
        }
    }
    fn insert_id(&mut self, id: usize, child: Arc<RwLock<Box<dyn TDisplayObject>>>) {
        self.render_list.write().unwrap().insert(id, child);
    }

    pub fn render_list_mut(&mut self) -> Arc<RwLock<Vec<Arc<RwLock<Box<dyn TDisplayObject>>>>>> {
        self.render_list.clone()
    }
    pub fn render_list_write(&self) -> RwLockWriteGuard<Vec<Arc<RwLock<Box<dyn TDisplayObject>>>>> {
        self.render_list.write().unwrap()
    }

    fn insert_child_into_depth_list(
        &mut self,
        depth: Depth,
        child: Arc<RwLock<Box<dyn TDisplayObject>>>,
    ) -> Option<Arc<RwLock<Box<dyn TDisplayObject>>>> {
        self.depth_list.insert(depth, child)
    }
    /// 在深度列表的特定位置向容器中插入一个子显示对象，并移除已在该位置上已存在的子对象。
    /// 将子对象插入深度列表后，我们将尝试为其分配一个呈现列表位置，该位置在深度列表中前一个项目之后。子代放入呈现列表的位置与 Flash Player 的行为一致。
    /// 从深度列表中移除的任何子代也将从呈现列表中移除，前提是该子代未被标记为由脚本放置。如果该子元素已从上述列表中移除，则将在此返回。否则，此方法将返回 "无"。
    pub fn replace_at_depth(
        &mut self,
        depth: Depth,
        child: Box<dyn TDisplayObject>,
    ) -> Option<Box<dyn TDisplayObject>> {
        let child = Arc::new(RwLock::new(child));
        let old_child = self.insert_child_into_depth_list(depth, child.clone());
        if let Some(old_child) = old_child {
            if let Some(position) = self
                .clone()
                .render_list
                .read()
                .unwrap()
                .iter()
                .position(|x| {
                    x.read().unwrap().character_id() == old_child.read().unwrap().character_id()
                })
            {
                // 目前好像不会执行这个分支，先留着
                dbg!("目前好像不会执行这个分支，先留着");
                self.insert_id(position + 1, child);
                None
            } else {
                dbg!("ChildContainer::replace_at_depth: Previous child is not in render list");
                self.render_list_write().push(child);
                None
            }
        } else {
            let above = self
                .depth_list
                .range((Bound::Excluded(depth), Bound::Unbounded))
                .map(|(_, v)| v.clone())
                .next();
            if let Some(above_child) = above {
                if let Some(position) =
                    self.render_list
                        .clone()
                        .read()
                        .unwrap()
                        .iter()
                        .position(|x| {
                            x.read().unwrap().character_id()
                                == above_child.read().unwrap().character_id()
                        })
                {
                    self.insert_id(position, child);
                    None
                } else {
                    self.render_list_write().push(child);
                    None
                }
            } else {
                self.render_list_write().push(child);
                None
            }
        }
    }
}
