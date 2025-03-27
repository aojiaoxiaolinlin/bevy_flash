use std::collections::HashMap;

use swf::CharacterId;

use super::{
    characters::{Character, CompressedBitmap},
    display_object::graphic::Graphic,
};

#[derive(Clone, Default)]
pub struct MovieLibrary {
    characters: HashMap<CharacterId, Character>,
    bitmap_characters: HashMap<CharacterId, CompressedBitmap>,
    pub instance_count: u16,
}

impl MovieLibrary {
    pub fn new() -> Self {
        Self {
            characters: HashMap::new(),
            bitmap_characters: HashMap::new(),
            instance_count: 0,
        }
    }
    pub fn register_bitmap_character(&mut self, id: CharacterId, bitmap: CompressedBitmap) {
        self.bitmap_characters.insert(id, bitmap);
    }

    pub fn register_character(&mut self, id: CharacterId, character: Character) {
        self.characters.insert(id, character);
    }

    pub fn character(&self, id: CharacterId) -> Option<&Character> {
        self.characters.get(&id)
    }
    pub fn character_mut(&mut self, id: CharacterId) -> Option<&mut Character> {
        self.characters.get_mut(&id)
    }
    pub fn characters(&self) -> &HashMap<CharacterId, Character> {
        &self.characters
    }
    pub fn characters_mut(&mut self) -> &mut HashMap<CharacterId, Character> {
        &mut self.characters
    }

    pub fn get_graphic(&self, id: CharacterId) -> Option<Graphic> {
        if let Some(Character::Graphic(graphic)) = self.characters.get(&id) {
            Some(graphic.clone())
        } else {
            None
        }
    }

    pub fn get_bitmap_characters(&self) -> HashMap<CharacterId, CompressedBitmap> {
        self.bitmap_characters.clone()
    }
}
