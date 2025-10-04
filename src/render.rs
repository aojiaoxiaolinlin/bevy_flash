pub(crate) mod blend_pipeline;
mod graph;
pub(crate) mod material;
pub(crate) mod offscreen_texture;
mod pipeline;
mod texture_attachment;

use bevy::{
    app::{App, Plugin},
    asset::{Assets, Handle, RenderAssetUsages, load_internal_asset},
    ecs::{resource::Resource, world::FromWorld},
    mesh::{Indices, Mesh, PrimitiveTopology},
    render::{RenderApp, RenderStartup},
    shader::Shader,
    sprite_render::Material2dPlugin,
};

use graph::FlashFilterRenderPlugin;
use material::{BitmapMaterial, ColorMaterial, GradientMaterial};

use crate::render::{
    material::{
        BITMAP_MATERIAL_SHADER_HANDLE, FLASH_COMMON_MATERIAL_SHADER_HANDLE,
        GRADIENT_MATERIAL_SHADER_HANDLE, SWF_COLOR_MATERIAL_SHADER_HANDLE,
    },
    offscreen_texture::{ExtractedOffscreenTexture, OffscreenTexturePlugin},
    pipeline::{
        BEVEL_FILTER_SHADER_HANDLE, BLUR_FILTER_SHADER_HANDLE, COLOR_MATRIX_FILTER_SHADER_HANDLE,
        GLOW_FILTER_SHADER_HANDLE, OFFSCREEN_MESH2D_BITMAP_SHADER_HANDLE,
        OFFSCREEN_MESH2D_GRADIENT_SHADER_HANDLE, OFFSCREEN_MESH2D_SHADER_HANDLE,
        init_bevel_filter_pipeline, init_blur_filter_pipeline, init_color_matrix_filter_pipeline,
        init_glow_filter_pipeline,
    },
};

pub struct FlashRenderPlugin;

impl Plugin for FlashRenderPlugin {
    fn build(&self, app: &mut App) {
        load_shaders(app);

        app.add_plugins(Material2dPlugin::<GradientMaterial>::default())
            .add_plugins(Material2dPlugin::<ColorMaterial>::default())
            .add_plugins(Material2dPlugin::<BitmapMaterial>::default())
            .add_plugins((OffscreenTexturePlugin, FlashFilterRenderPlugin))
            .init_resource::<FilterTextureMesh>();

        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };

        render_app.add_systems(
            RenderStartup,
            (
                init_blur_filter_pipeline,
                init_color_matrix_filter_pipeline,
                init_glow_filter_pipeline,
                init_bevel_filter_pipeline,
            ),
        );
    }
}

/// 用于滤镜纹理渲染的Mesh，一个固定的矩形
#[derive(Resource, Debug, Clone)]
/// 用于滤镜纹理渲染的固定矩形网格
pub struct FilterTextureMesh(pub Handle<Mesh>);

impl FromWorld for FilterTextureMesh {
    fn from_world(world: &mut bevy::ecs::world::World) -> Self {
        let mut meshes = world.resource_mut::<Assets<Mesh>>();
        let mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )
        .with_inserted_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
        )
        .with_inserted_indices(Indices::U32(vec![0, 1, 2, 0, 2, 3]));
        Self(meshes.add(mesh))
    }
}

fn load_shaders(app: &mut App) {
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

    load_internal_asset!(
        app,
        OFFSCREEN_MESH2D_SHADER_HANDLE,
        "render/shaders/offscreen_mesh2d/color.wgsl",
        Shader::from_wgsl
    );
    load_internal_asset!(
        app,
        OFFSCREEN_MESH2D_GRADIENT_SHADER_HANDLE,
        "render/shaders/offscreen_mesh2d/gradient.wgsl",
        Shader::from_wgsl
    );
    load_internal_asset!(
        app,
        OFFSCREEN_MESH2D_BITMAP_SHADER_HANDLE,
        "render/shaders/offscreen_mesh2d/bitmap.wgsl",
        Shader::from_wgsl
    );
    load_internal_asset!(
        app,
        BLUR_FILTER_SHADER_HANDLE,
        "render/shaders/filters/blur.wgsl",
        Shader::from_wgsl
    );
    load_internal_asset!(
        app,
        COLOR_MATRIX_FILTER_SHADER_HANDLE,
        "render/shaders/filters/color_matrix.wgsl",
        Shader::from_wgsl
    );
    load_internal_asset!(
        app,
        GLOW_FILTER_SHADER_HANDLE,
        "render/shaders/filters/glow.wgsl",
        Shader::from_wgsl
    );
    load_internal_asset!(
        app,
        BEVEL_FILTER_SHADER_HANDLE,
        "render/shaders/filters/bevel.wgsl",
        Shader::from_wgsl
    );
}
