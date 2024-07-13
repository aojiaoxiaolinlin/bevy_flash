use super::DisplayObjectBase;

pub struct MorphShape {
    base: DisplayObjectBase,
    ratio: u16,
}

impl MorphShape {
    pub fn new(ratio: u16) -> Self {
        Self {
            base: DisplayObjectBase::default(),
            ratio,
        }
    }

    pub fn set_ratio(&mut self, ratio: u16) {
        self.ratio = ratio;
    }
}
