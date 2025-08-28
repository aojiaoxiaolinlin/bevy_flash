use super::material::BlendMaterialKey;

#[derive(Debug, Copy, Clone)]
pub enum TrivialBlend {
    Normal,
    Add,
    Subtract,
    Screen,
    Lighten,
    Darken,
    Multiply,
}

#[derive(Debug, Copy, Clone)]
pub enum ComplexBlend {
    // Multiply,   // Can't be trivial, 0 alpha is special case
    Difference, // Can't be trivial, relies on abs operation
    Invert,     // May be trivial using a constant? Hard because it's without premultiplied alpha
    Alpha,      // Can't be trivial, requires layer tracking
    Erase,      // Can't be trivial, requires layer tracking
    Overlay,    // Can't be trivial, big math expression
    HardLight,  // Can't be trivial, big math expression
}

#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
pub enum BlendMode {
    Trivial(TrivialBlend),
    /// TODO: 需要抓取屏幕纹理进行混合
    Complex(ComplexBlend),
}

impl From<swf::BlendMode> for BlendMode {
    fn from(mode: swf::BlendMode) -> BlendMode {
        match mode {
            swf::BlendMode::Normal => BlendMode::Trivial(TrivialBlend::Normal),
            swf::BlendMode::Layer => BlendMode::Trivial(TrivialBlend::Normal),
            swf::BlendMode::Add => BlendMode::Trivial(TrivialBlend::Add),
            swf::BlendMode::Subtract => BlendMode::Trivial(TrivialBlend::Subtract),
            swf::BlendMode::Screen => BlendMode::Trivial(TrivialBlend::Screen),
            swf::BlendMode::Lighten => BlendMode::Trivial(TrivialBlend::Lighten),
            swf::BlendMode::Darken => BlendMode::Trivial(TrivialBlend::Darken),
            swf::BlendMode::Multiply => BlendMode::Trivial(TrivialBlend::Multiply),
            swf::BlendMode::Alpha => BlendMode::Complex(ComplexBlend::Alpha),
            swf::BlendMode::Difference => BlendMode::Complex(ComplexBlend::Difference),
            swf::BlendMode::Invert => BlendMode::Complex(ComplexBlend::Invert),
            swf::BlendMode::Erase => BlendMode::Complex(ComplexBlend::Erase),
            swf::BlendMode::Overlay => BlendMode::Complex(ComplexBlend::Overlay),
            swf::BlendMode::HardLight => BlendMode::Complex(ComplexBlend::HardLight),
        }
    }
}

impl From<BlendMode> for BlendMaterialKey {
    fn from(value: BlendMode) -> Self {
        match value {
            BlendMode::Trivial(TrivialBlend::Normal) => BlendMaterialKey::NORMAL,
            BlendMode::Trivial(TrivialBlend::Add) => BlendMaterialKey::BLEND_ADD,
            BlendMode::Trivial(TrivialBlend::Subtract) => BlendMaterialKey::BLEND_SUBTRACT,
            BlendMode::Trivial(TrivialBlend::Screen) => BlendMaterialKey::BLEND_SCREEN,
            BlendMode::Trivial(TrivialBlend::Lighten) => BlendMaterialKey::BLEND_LIGHTEN,
            BlendMode::Trivial(TrivialBlend::Multiply) => BlendMaterialKey::BLEND_MULTIPLY,
            BlendMode::Trivial(TrivialBlend::Darken) => BlendMaterialKey::BLEND_DARKEN,
            _ => BlendMaterialKey::NORMAL,
        }
    }
}
