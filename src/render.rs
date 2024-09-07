use std::{collections::BTreeMap, sync::Arc};

use bevy::{
    app::{App, Plugin, PostUpdate},
    asset::{Assets, Handle},
    ecs::entity::EntityHashMap,
    log::info,
    prelude::{Commands, Component, Entity, Gizmos, Local, Mesh, Query, ResMut, Shader, Transform},
    sprite::{
        ColorMaterial, ColorMesh2dBundle, Material2d, Material2dPlugin, MaterialMesh2dBundle,
        Mesh2dHandle,
    },
};
use glam::{Quat, Vec3};
use ruffle_render::transform::Transform as RuffleTransform;

use crate::{
    bundle::Swf,
    swf::display_object::{graphic::GraphicStatus, render_base, DisplayObject, TDisplayObject},
};

/// 使用UUID指定,SWF着色器Handle
// pub const SWF_GRAPHIC_HANDLE: Handle<Shader> =
//     Handle::weak_from_u128(251354789657743035148351631714426867038);
pub(crate) mod commands;
mod pipeline;
pub(crate) mod tessellator;

pub struct FlashRenderPlugin;

impl Plugin for FlashRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostUpdate, render_swf);
    }
}

pub fn render_swf(
    mut commands: Commands,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut query: Query<&mut Swf>,
    mut mesh_query: Query<&mut Mesh2dHandle>,
    mut gizmos: Gizmos,
    mut entities: Local<Vec<Entity>>,
) {
    query.iter_mut().for_each(|mut swf| {
        let render_list = swf.root_movie_clip.raw_container().render_list();
        let display_objects = swf
            .root_movie_clip
            .raw_container_mut()
            .display_objects_mut();
        handler_render_list(
            &mut commands,
            &mut materials,
            &mut mesh_query,
            render_list,
            display_objects,
            &mut gizmos,
            &mut entities,
        );
    });
}

pub fn handler_render_list(
    commands: &mut Commands,
    materials: &mut ResMut<Assets<ColorMaterial>>,
    mesh_query: &mut Query<&mut Mesh2dHandle>,
    render_list: Arc<Vec<u128>>,
    display_objects: &mut BTreeMap<u128, DisplayObject>,
    gizmos: &mut Gizmos,
    entities: &mut Vec<Entity>,
) {
    entities.iter().for_each(|entity| {
        commands.entity(*entity).despawn();
    });
    entities.clear();
    for display_object in render_list.iter() {
        if let Some(display_object) = display_objects.get_mut(display_object) {
            match display_object {
                DisplayObject::Graphic(graphic) => {
                    if let Some(mesh) = graphic.mesh() {
                        // 如果已经存在，则不再创建
                        // for (swf_component, mut transform) in query_entity.iter_mut() {
                        //     if swf_component.id == mesh.id() {
                        //         let graphic_transform: Transform =
                        //             SWFTransform(graphic.base().transform().clone()).into();
                        //         // 减去基础变换以定位到基础位置
                        //         transform.rotation = graphic_transform.rotation;
                        //         transform.scale =
                        //             graphic_transform.scale * swf_component.base_transform.scale;

                        //         let width = graphic.bounds.width().to_pixels() as f32;
                        //         let height = graphic.bounds.height().to_pixels() as f32;

                        //         let new_width = width * transform.scale.x;
                        //         let new_height = height * transform.scale.y;

                        //         let translation = swf_component.base_transform.translation
                        //             - graphic_transform.translation;

                        //         let delta = Vec3::new(
                        //             (new_width - width) / 2.0,
                        //             (new_height - height) / 2.0,
                        //             0.0,
                        //         );
                        //         transform.translation = translation - delta;
                        //         // 绘制矩形边框
                        //         gizmos.rect_2d(
                        //             Vec2::ZERO,
                        //             0.,
                        //             Vec2::new(
                        //                 width * transform.scale.x,
                        //                 height * transform.scale.y,
                        //             ),
                        //             WHITE,
                        //         );
                        //         return;
                        //     }
                        // }
                        let base_transform: Transform =
                            SWFTransform(graphic.base().transform().clone()).into();
                        let entity = commands.spawn(ColorMesh2dBundle {
                            mesh: mesh.into(),
                            material: materials.add(ColorMaterial::default()),
                            transform: base_transform,
                            ..Default::default()
                        });
                        entities.push(entity.id());
                        // let status = graphic.status();
                        // match status {
                        //     GraphicStatus::Place => {
                        //         info!("PlaceObject: graphic id: {}", graphic.character_id(),);

                        //         graphic.set_status(GraphicStatus::Normal);
                        //     }
                        //     GraphicStatus::Replace => {
                        //         mesh_query.iter_mut().for_each(|mut mesh_handle| {
                        //             info!("ReplaceObject: graphic id: {}", graphic.character_id(),);
                        //             mesh_handle.0 = mesh.clone();
                        //             graphic.set_status(GraphicStatus::Normal);
                        //         });
                        //     }
                        //     _ => {}
                        // }
                    }
                }
                DisplayObject::MovieClip(movie_clip) => {
                    handler_render_list(
                        commands,
                        materials,
                        mesh_query,
                        movie_clip.raw_container().render_list(),
                        movie_clip.raw_container_mut().display_objects_mut(),
                        gizmos,
                        entities,
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
        Self {
            translation: Vec3::from(translation),
            rotation: Quat::from_rotation_z(form.matrix.b.to_radians()),
            scale: Vec3::from(scale),
        }
    }
}
