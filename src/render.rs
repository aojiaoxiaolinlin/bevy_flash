use bevy::{
    app::{App, Plugin, PostUpdate},
    asset::{load_internal_asset, Asset, AssetApp, Assets, Handle},
    ecs::entity::EntityHashMap,
    prelude::{
        Commands, IntoSystemConfigs, Mesh, Query, ReflectDefault, ResMut, Resource, Shader,
        Transform,
    },
    reflect::{self, Reflect},
    render::render_resource::{AsBindGroup, ShaderRef},
    sprite::{Material2d, Material2dPlugin, MaterialMesh2dBundle},
};
use glam::Vec3;
use ruffle_render::transform::Transform as RuffleTransform;

use crate::{
    assets::SwfMovie,
    swf::{characters::Character, display_object::render_base},
};

/// 使用UUID指定,SWF着色器Handle
pub const SWF_GRAPHIC_HANDLE: Handle<Shader> =
    Handle::weak_from_u128(251354789657743035148351631714426867038);

pub(crate) mod commands;
mod pipeline;
pub(crate) mod tessellator;

pub struct ExtractedSWFShape {}
#[derive(Resource, Default)]
pub struct ExtractedSWFShapes {
    pub swf_sprites: EntityHashMap<ExtractedSWFShape>,
}

pub struct FlashRenderPlugin;

impl Plugin for FlashRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(Material2dPlugin::<CustomMaterial>::default())
            .register_asset_reflect::<CustomMaterial>()
            .add_systems(PostUpdate, (pre_render, render).chain());
        load_internal_asset!(
            app,
            SWF_GRAPHIC_HANDLE,
            "./render/shaders/graphic.wgsl",
            Shader::from_wgsl
        );
    }
}

/// 从SWF中提取形状，提前转为`Mesh2d`。swf加载完成后执行 。

fn pre_render(query: Query<&Handle<SwfMovie>>, mut swf_movie: ResMut<Assets<SwfMovie>>) {
    for swf_handle in query.iter() {
        if let Some(swf_movie) = swf_movie.get_mut(swf_handle.id()) {
            let root_movie_clip = swf_movie.root_movie_clip.clone();
            render_base(root_movie_clip.into(), RuffleTransform::default());
        }
    }
}

fn render(
    mut commands: Commands,
    mut materials: ResMut<Assets<CustomMaterial>>,
    mut swf_movie: ResMut<Assets<SwfMovie>>,
    query: Query<&Handle<SwfMovie>>,
) {
    for swf_handle in query.iter() {
        if let Some(swf_movie) = swf_movie.get_mut(swf_handle.id()) {
            let library = &mut swf_movie.library;
            for (_, character) in library.characters_mut() {
                match character {
                    Character::Graphic(graphic) => {
                        if let Some(mesh) = graphic.mesh() {
                            commands.spawn(MaterialMesh2dBundle {
                                mesh: mesh.into(),
                                material: materials.add(CustomMaterial {}),
                                transform: Transform::from_scale(Vec3::splat(8.)),
                                ..Default::default()
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone, Default)]
#[reflect(Default, Debug)]
struct CustomMaterial {}

impl Material2d for CustomMaterial {
    fn fragment_shader() -> ShaderRef {
        SWF_GRAPHIC_HANDLE.into()
    }
}
