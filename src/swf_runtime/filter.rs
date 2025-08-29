use swf::{Rectangle, Twips};

/// 用于渲染的滤镜结构
#[derive(Debug, Clone, PartialEq)]
pub enum Filter {
    BevelFilter(swf::BevelFilter),
    BlurFilter(swf::BlurFilter),
    ColorMatrixFilter(swf::ColorMatrixFilter),
    ConvolutionFilter(swf::ConvolutionFilter),
    DropShadowFilter(swf::DropShadowFilter),
    GlowFilter(swf::GlowFilter),
    GradientBevelFilter(swf::GradientFilter),
    GradientGlowFilter(swf::GradientFilter),
}

impl Filter {
    pub fn scale(&mut self, x: f32, y: f32) {
        match self {
            Filter::BevelFilter(filter) => filter.scale(x, y),
            Filter::BlurFilter(filter) => filter.scale(x, y),
            Filter::DropShadowFilter(filter) => filter.scale(x, y),
            Filter::GlowFilter(filter) => filter.scale(x, y),
            Filter::GradientBevelFilter(filter) => filter.scale(x, y),
            Filter::GradientGlowFilter(filter) => filter.scale(x, y),
            _ => {}
        }
    }

    pub fn calculate_dest_rect(&self, source_rect: Rectangle<Twips>) -> Rectangle<Twips> {
        match self {
            Filter::BlurFilter(filter) => filter.calculate_dest_rect(source_rect),
            Filter::GlowFilter(filter) => filter.calculate_dest_rect(source_rect),
            Filter::DropShadowFilter(filter) => filter.calculate_dest_rect(source_rect),
            Filter::BevelFilter(filter) => filter.calculate_dest_rect(source_rect),
            _ => source_rect,
        }
    }

    /// Checks if this filter is impotent.
    /// Impotent filters will have no effect if applied, and can safely be skipped.
    pub fn impotent(&self) -> bool {
        // TODO: There's more cases here, find them!
        match self {
            Filter::BlurFilter(filter) => filter.impotent(),
            Filter::ColorMatrixFilter(filter) => filter.impotent(),
            _ => false,
        }
    }
}

impl From<&swf::Filter> for Filter {
    fn from(value: &swf::Filter) -> Self {
        match value {
            swf::Filter::DropShadowFilter(filter) => {
                Filter::DropShadowFilter(filter.as_ref().to_owned())
            }
            swf::Filter::BlurFilter(filter) => Filter::BlurFilter(filter.as_ref().to_owned()),
            swf::Filter::GlowFilter(filter) => Filter::GlowFilter(filter.as_ref().to_owned()),
            swf::Filter::BevelFilter(filter) => Filter::BevelFilter(filter.as_ref().to_owned()),
            swf::Filter::GradientGlowFilter(filter) => {
                Filter::GradientGlowFilter(filter.as_ref().to_owned())
            }
            swf::Filter::ConvolutionFilter(filter) => {
                Filter::ConvolutionFilter(filter.as_ref().to_owned())
            }
            swf::Filter::ColorMatrixFilter(filter) => {
                Filter::ColorMatrixFilter(filter.as_ref().to_owned())
            }
            swf::Filter::GradientBevelFilter(filter) => {
                Filter::GradientBevelFilter(filter.as_ref().to_owned())
            }
        }
    }
}
