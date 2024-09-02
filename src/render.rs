use std::{collections::BTreeMap, sync::Arc};

use bevy::{
    app::{App, Plugin, PostUpdate},
    asset::{load_internal_asset, Asset, AssetApp, AssetId, Assets, Handle},
    color::palettes::css::{GREEN, WHITE},
    ecs::entity::EntityHashMap,
    math::VectorSpace,
    prelude::{
        Commands, Component, Entity, Gizmos, IntoSystemConfigs, Mesh, Query, ReflectDefault,
        ResMut, Resource, Shader, Transform, With,
    },
    reflect::{self, Reflect},
    render::render_resource::{AsBindGroup, ShaderRef},
    sprite::{
        ColorMaterial, ColorMesh2dBundle, Material2d, Material2dPlugin, MaterialMesh2dBundle,
        Mesh2dHandle,
    },
};
use glam::{Quat, Vec2, Vec3};
use ruffle_render::transform::{self, Transform as RuffleTransform};

use crate::{
    assets::SwfMovie,
    swf::{
        characters::Character,
        display_object::{
            graphic::{self, Graphic},
            render_base, DisplayObject, TDisplayObject,
        },
    },
};

/// 使用UUID指定,SWF着色器Handle
pub const SWF_GRAPHIC_HANDLE: Handle<Shader> =
    Handle::weak_from_u128(251354789657743035148351631714426867038);

pub(crate) mod commands;
mod pipeline;
pub(crate) mod tessellator;

#[derive(Component)]
pub struct SWFComponent {
    pub id: AssetId<Mesh>,
    pub base_transform: Transform,
}

pub struct FlashRenderPlugin;

impl Plugin for FlashRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostUpdate, (pre_render, render).chain());
    }
}

/// 从SWF中提取形状，提前转为`Mesh2d`。swf加载完成后执行 。

fn pre_render(query: Query<&Handle<SwfMovie>>, mut swf_movie: ResMut<Assets<SwfMovie>>) {
    for swf_handle in query.iter() {
        if let Some(swf_movie) = swf_movie.get_mut(swf_handle.id()) {
            // render_base(
            //     swf_movie.root_movie_clip.clone().into(),
            //     RuffleTransform::default(),
            // );
        }
    }
}

fn render(
    mut commands: Commands,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut swf_movie: ResMut<Assets<SwfMovie>>,
    query: Query<&Handle<SwfMovie>>,
    mut query_entity: Query<(&SWFComponent, &mut Transform)>,
    mut gizmos: Gizmos,
) {
    query.iter().for_each(|swf_handle| {
        if let Some(swf_movie) = swf_movie.get_mut(swf_handle.id()) {
            let render_list = swf_movie.root_movie_clip.raw_container_mut().render_list();
            let display_objects = swf_movie
                .root_movie_clip
                .raw_container_mut()
                .display_objects();
            handler_render_list(
                &mut commands,
                &mut materials,
                render_list,
                display_objects,
                &mut query_entity,
                &mut gizmos,
            );
        }
    });
}

pub fn handler_render_list(
    commands: &mut Commands,
    materials: &mut ResMut<Assets<ColorMaterial>>,
    render_list: Arc<Vec<u16>>,
    display_objects: &BTreeMap<u16, DisplayObject>,
    query_entity: &mut Query<(&SWFComponent, &mut Transform)>,
    gizmos: &mut Gizmos,
) {
    for display_object in render_list.iter() {
        if let Some(display_object) = display_objects.get(display_object) {
            match display_object {
                DisplayObject::Graphic(graphic) => {
                    if let Some(mesh) = graphic.mesh() {
                        // 如果已经存在，则不再创建
                        for (swf_component, mut transform) in query_entity.iter_mut() {
                            if swf_component.id == mesh.id() {
                                let graphic_transform: Transform =
                                    SWFTransform(graphic.base().transform().clone()).into();
                                // 减去基础变换以定位到基础位置
                                transform.rotation = graphic_transform.rotation;
                                dbg!(transform.rotation);
                                transform.scale =
                                    graphic_transform.scale * swf_component.base_transform.scale;

                                let width = graphic.bounds.width().to_pixels() as f32;
                                let height = graphic.bounds.height().to_pixels() as f32;

                                let new_width = width * transform.scale.x;
                                let new_height = height * transform.scale.y;

                                let translation = swf_component.base_transform.translation
                                    - graphic_transform.translation;

                                let delta = Vec3::new(
                                    (new_width - width) / 2.0,
                                    (new_height - height) / 2.0,
                                    0.0,
                                );
                                transform.translation = translation - delta;
                                // 绘制矩形边框
                                gizmos.rect_2d(
                                    Vec2::ZERO,
                                    0.,
                                    Vec2::new(
                                        width * transform.scale.x,
                                        height * transform.scale.y,
                                    ),
                                    WHITE,
                                );
                                return;
                            }
                        }
                        let base_transform: Transform =
                            SWFTransform(graphic.base().transform().clone()).into();
                        commands.spawn((
                            SWFComponent {
                                id: mesh.clone().id(),
                                base_transform,
                            },
                            ColorMesh2dBundle {
                                mesh: mesh.into(),
                                material: materials.add(ColorMaterial::default()),
                                transform: base_transform,
                                ..Default::default()
                            },
                        ));
                    }
                }
                DisplayObject::MovieClip(movie_clip) => {
                    handler_render_list(
                        commands,
                        materials,
                        movie_clip.clone().raw_container_mut().render_list(),
                        display_objects,
                        query_entity,
                        gizmos,
                    );
                }
            }
        }
    }
}

struct SWFTransform(RuffleTransform);

impl From<SWFTransform> for Transform {
    fn from(form: SWFTransform) -> Self {
        let form = form.0;
        let translation: [f32; 3] = [
            form.matrix.tx.to_pixels() as f32,
            form.matrix.ty.to_pixels() as f32,
            0.0,
        ];
        let scale = [form.matrix.a, form.matrix.d, 1.0];
        dbg!(form.matrix.b);
        Self {
            translation: Vec3::from(translation),
            rotation: Quat::from_rotation_z(form.matrix.b.to_radians()),
            scale: Vec3::from(scale),
        }
    }
}
