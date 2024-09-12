use std::{collections::BTreeMap, sync::Arc};

use bevy::{
    app::{App, Plugin, PostUpdate},
    asset::{load_internal_asset, Assets, Handle},
    ecs::entity::EntityHashMap,
    log::info,
    prelude::{
        BuildChildren, Commands, Component, Entity, Gizmos, Local, Mesh, Query, ResMut, Shader,
        Transform, With,
    },
    sprite::{Material2dPlugin, MaterialMesh2dBundle, Mesh2dHandle},
};
use glam::{Mat4, Vec3};
use material::{GradientMaterial, SWFColorMaterial, SWFTransform};
use ruffle_render::transform::Transform as RuffleTransform;

use crate::{
    bundle::Swf,
    swf::display_object::{DisplayObject, TDisplayObject},
};

pub(crate) mod commands;
pub(crate) mod material;
mod pipeline;
pub(crate) mod tessellator;
pub struct FlashRenderPlugin;

impl Plugin for FlashRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(Material2dPlugin::<GradientMaterial>::default())
            .add_plugins(Material2dPlugin::<SWFColorMaterial>::default())
            .add_systems(PostUpdate, render_swf);
    }
}

pub fn render_swf(
    mut commands: Commands,
    mut materials: ResMut<Assets<SWFColorMaterial>>,
    mut gradient_materials: ResMut<Assets<GradientMaterial>>,
    mut query: Query<&mut Swf>,
    mut gizmos: Gizmos,
    entities_query: Query<Entity, With<Mesh2dHandle>>,
) {
    query.iter_mut().for_each(|mut swf| {
        let render_list = swf.root_movie_clip.raw_container().render_list();
        let transform = swf.root_movie_clip.base().transform().clone();
        let display_objects = swf
            .root_movie_clip
            .raw_container_mut()
            .display_objects_mut();
        entities_query.iter().for_each(|entity| {
            commands.entity(entity).despawn();
        });

        let mut z_index = 0.0;

        handler_render_list(
            &mut commands,
            &mut materials,
            &mut gradient_materials,
            render_list,
            display_objects,
            &mut gizmos,
            &transform,
            &mut z_index,
        );
    });
}

pub fn handler_render_list(
    commands: &mut Commands,
    materials: &mut ResMut<Assets<SWFColorMaterial>>,
    gradient_materials: &mut ResMut<Assets<GradientMaterial>>,
    render_list: Arc<Vec<u128>>,
    display_objects: &mut BTreeMap<u128, DisplayObject>,
    gizmos: &mut Gizmos,
    parent_transform: &RuffleTransform,
    z_index: &mut f32,
) {
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
                        let current_transform = graphic.base().transform();
                        let transform = RuffleTransform {
                            matrix: parent_transform.matrix * current_transform.matrix,
                            color_transform: parent_transform.color_transform
                                * current_transform.color_transform,
                        };

                        commands.spawn(MaterialMesh2dBundle {
                            mesh: mesh.into(),
                            material: materials.add(SWFColorMaterial {
                                transform: transform.clone().into(),
                            }),
                            transform: Transform::from_translation(Vec3::new(
                                0.0,
                                0.0,
                                *z_index + 0.1,
                            )),
                            ..Default::default()
                        });
                        *z_index += 0.1;
                        for (mesh_handle, material) in graphic.gradient_mesh() {
                            if let Some(gradient_material) =
                                gradient_materials.get_mut(material.id())
                            {
                                gradient_material.transform = transform.clone().into();
                            }
                            commands.spawn(MaterialMesh2dBundle {
                                mesh: mesh_handle.clone().into(),
                                material: material.clone(),
                                transform: Transform::from_translation(Vec3::new(
                                    0.0,
                                    0.0,
                                    *z_index + 0.1,
                                )),
                                ..Default::default()
                            });
                            *z_index += 0.1;
                        }
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
                    let current_transform = RuffleTransform {
                        matrix: parent_transform.matrix * movie_clip.base().transform().matrix,
                        color_transform: parent_transform.color_transform
                            * movie_clip.base().transform().color_transform,
                    };
                    handler_render_list(
                        commands,
                        materials,
                        gradient_materials,
                        movie_clip.raw_container().render_list(),
                        movie_clip.raw_container_mut().display_objects_mut(),
                        gizmos,
                        &current_transform,
                        z_index,
                    );
                }
            }
        }
    }
}
