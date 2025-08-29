use bevy::{asset::Handle, image::Image};
use swf::CharacterId;

use crate::{render::blend_pipeline::BlendMode, swf_runtime::transform::Transform};

pub(crate) enum ShapeCommand {
    RenderShape {
        transform: Transform,
        // Graphic 对应的 CharacterId
        id: CharacterId,
        shape_depth_layer: String,
        blend_mode: BlendMode,
    },
    RenderBitmap {
        transform: Transform,
        // Bitmap 对应的 CharacterId
        id: CharacterId,
        // 材质
        handle: Handle<Image>,
    },
}
