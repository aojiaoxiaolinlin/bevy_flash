use bevy::sprite::AlphaMode2d;
use enum_map::Enum;
use ruffle_render::blend::ExtendedBlendMode;

#[derive(Enum, Debug, Copy, Clone)]
pub enum TrivialBlend {
    Normal,
    Add,
    Subtract,
    Screen,
}

#[derive(Enum, Debug, Copy, Clone)]
pub enum ComplexBlend {
    Multiply,   // Can't be trivial, 0 alpha is special case
    Lighten,    // Might be trivial but I can't reproduce the right colors
    Darken,     // Might be trivial but I can't reproduce the right colors
    Difference, // Can't be trivial, relies on abs operation
    Invert,     // May be trivial using a constant? Hard because it's without premultiplied alpha
    Alpha,      // Can't be trivial, requires layer tracking
    Erase,      // Can't be trivial, requires layer tracking
    Overlay,    // Can't be trivial, big math expression
    HardLight,  // Can't be trivial, big math expression
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum BlendType {
    Trivial(TrivialBlend),
    Complex(ComplexBlend),
}

impl BlendType {
    pub fn from(mode: ExtendedBlendMode) -> BlendType {
        match mode {
            ExtendedBlendMode::Normal => BlendType::Trivial(TrivialBlend::Normal),
            ExtendedBlendMode::Layer => BlendType::Trivial(TrivialBlend::Normal),
            ExtendedBlendMode::Add => BlendType::Trivial(TrivialBlend::Add),
            ExtendedBlendMode::Subtract => BlendType::Trivial(TrivialBlend::Subtract),
            ExtendedBlendMode::Screen => BlendType::Trivial(TrivialBlend::Screen),
            ExtendedBlendMode::Alpha => BlendType::Complex(ComplexBlend::Alpha),
            ExtendedBlendMode::Multiply => BlendType::Complex(ComplexBlend::Multiply),
            ExtendedBlendMode::Lighten => BlendType::Complex(ComplexBlend::Lighten),
            ExtendedBlendMode::Darken => BlendType::Complex(ComplexBlend::Darken),
            ExtendedBlendMode::Difference => BlendType::Complex(ComplexBlend::Difference),
            ExtendedBlendMode::Invert => BlendType::Complex(ComplexBlend::Invert),
            ExtendedBlendMode::Erase => BlendType::Complex(ComplexBlend::Erase),
            ExtendedBlendMode::Overlay => BlendType::Complex(ComplexBlend::Overlay),
            ExtendedBlendMode::HardLight => BlendType::Complex(ComplexBlend::HardLight),
            ExtendedBlendMode::Shader => unreachable!(),
        }
    }
}

impl From<BlendType> for AlphaMode2d {
    fn from(value: BlendType) -> Self {
        match value {
            BlendType::Trivial(TrivialBlend::Normal) => AlphaMode2d::Blend,
            BlendType::Trivial(TrivialBlend::Add) => AlphaMode2d::Add,
            BlendType::Trivial(TrivialBlend::Subtract) => AlphaMode2d::Subtract,
            BlendType::Trivial(TrivialBlend::Screen) => AlphaMode2d::Screen,
            BlendType::Complex(ComplexBlend::Lighten) => AlphaMode2d::Lighten,
            BlendType::Complex(ComplexBlend::Multiply) => AlphaMode2d::Multiply,
            BlendType::Complex(ComplexBlend::Darken) => AlphaMode2d::Darken,
            // TODO: Implement complex blend modes
            _ => AlphaMode2d::Blend,
        }
    }
}
