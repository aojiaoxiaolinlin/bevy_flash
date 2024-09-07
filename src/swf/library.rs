use std::{
    collections::HashMap,
    sync::{Arc, Weak},
};

use bevy::{asset::Asset, reflect::TypePath};
use swf::CharacterId;
use weak_table::PtrWeakKeyHashMap;

use crate::assets::SwfMovie;

use super::{characters::Character, display_object::graphic::Graphic};

#[derive(Default, Asset, TypePath)]
pub struct Library {
    movie_libraries: PtrWeakKeyHashMap<Weak<SwfMovie>, MovieLibrary>,
}

impl Library {
    pub fn library_for_movie(&self, movie: Arc<SwfMovie>) -> Option<&MovieLibrary> {
        self.movie_libraries.get(&movie)
    }
    pub fn library_for_movie_mut(&mut self, movie: Arc<SwfMovie>) -> Option<&mut MovieLibrary> {
        self.movie_libraries.get_mut(&movie)
    }
}

#[derive(Clone, Default)]
pub struct MovieLibrary {
    characters: HashMap<CharacterId, Character>,
    pub instance_count: u16,
}

impl MovieLibrary {
    pub fn new() -> Self {
        Self {
            characters: HashMap::new(),
            instance_count: 0,
        }
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
        if let Some(Character::Graphic(graphic)) = self.characters.get(&id).clone() {
            Some(graphic.clone())
        } else {
            None
        }
    }
}
