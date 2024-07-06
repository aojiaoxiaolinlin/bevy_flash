use super::display_object::{graphic::Graphic, movie_clip::MovieClip};

pub enum Character {
    MovieClip(MovieClip),
    Graphic(Graphic),
}
