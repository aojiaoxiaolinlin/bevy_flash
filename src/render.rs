pub(crate) mod blend_pipeline;
mod graph;
pub(crate) mod material;
pub(crate) mod offscreen_texture;
mod pipeline;
mod sort_item;
mod texture_attachment;

use bevy::{
    app::{App, Plugin},
    asset::{Handle, load_internal_asset, weak_handle},
    render::render_resource::Shader,
    sprite::Material2dPlugin,
};

use graph::FlashFilterRenderGraphPlugin;
use material::{BitmapMaterial, ColorMaterial, GradientMaterial};

use crate::render::offscreen_texture::{ExtractedOffscreenTexture, OffscreenTexturePlugin};

pub const SWF_COLOR_MATERIAL_SHADER_HANDLE: Handle<Shader> =
    weak_handle!("8c2a5b0f-3e6d-4f8a-b217-84d2f5e1c9b3");
pub const GRADIENT_MATERIAL_SHADER_HANDLE: Handle<Shader> =
    weak_handle!("5e9f1a78-9b34-4c15-8d7e-2a3b0f47d862");
pub const BITMAP_MATERIAL_SHADER_HANDLE: Handle<Shader> =
    weak_handle!("a34c7d82-1f5b-4a9e-93d8-6b7e20c45a1f");
pub const FLASH_COMMON_MATERIAL_SHADER_HANDLE: Handle<Shader> =
    weak_handle!("e53b9f82-6a4c-4d5b-91e7-4f2a63b8c5d9");

pub struct FlashRenderPlugin;

impl Plugin for FlashRenderPlugin {
    fn build(&self, app: &mut App) {
        load_internal_asset!(
            app,
            FLASH_COMMON_MATERIAL_SHADER_HANDLE,
            "render/shaders/common.wgsl",
            Shader::from_wgsl
        );
        load_internal_asset!(
            app,
            SWF_COLOR_MATERIAL_SHADER_HANDLE,
            "render/shaders/color.wgsl",
            Shader::from_wgsl
        );
        load_internal_asset!(
            app,
            GRADIENT_MATERIAL_SHADER_HANDLE,
            "render/shaders/gradient.wgsl",
            Shader::from_wgsl
        );
        load_internal_asset!(
            app,
            BITMAP_MATERIAL_SHADER_HANDLE,
            "render/shaders/bitmap.wgsl",
            Shader::from_wgsl
        );

        app.add_plugins(Material2dPlugin::<GradientMaterial>::default())
            .add_plugins(Material2dPlugin::<ColorMaterial>::default())
            .add_plugins(Material2dPlugin::<BitmapMaterial>::default())
            .add_plugins((OffscreenTexturePlugin, FlashFilterRenderGraphPlugin));
    }
}
