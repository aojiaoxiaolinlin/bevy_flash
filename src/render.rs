use std::{collections::BTreeMap, sync::Arc};

use bevy::{
    app::{App, Plugin, PostUpdate},
    asset::{load_internal_asset, Assets, Handle},
    prelude::{Commands, Entity, EventReader, Gizmos, Query, ResMut, Shader, Transform, With},
    sprite::{Material2dPlugin, MaterialMesh2dBundle, Mesh2dHandle},
};
use glam::Vec3;
use material::{BitmapMaterial, GradientMaterial, SWFColorMaterial};
use ruffle_render::transform::Transform as RuffleTransform;

use crate::{
    bundle::Swf,
    plugin::SWFRenderEvent,
    swf::display_object::{DisplayObject, TDisplayObject},
};

pub const SWF_COLOR_MATERIAL_SHADER_HANDLE: Handle<Shader> =
    Handle::weak_from_u128(283691495474896754103765489274589);
pub const GRADIENT_MATERIAL_SHADER_HANDLE: Handle<Shader> =
    Handle::weak_from_u128(55042096615683885463288330940691701066);
pub const BITMAP_MATERIAL_SHADER_HANDLE: Handle<Shader> =
    Handle::weak_from_u128(1209708179628049255077713250256144531);

pub(crate) mod material;
pub(crate) mod tessellator;
pub struct FlashRenderPlugin;

impl Plugin for FlashRenderPlugin {
    fn build(&self, app: &mut App) {
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
            .add_plugins(Material2dPlugin::<SWFColorMaterial>::default())
            .add_plugins(Material2dPlugin::<BitmapMaterial>::default())
            .add_systems(PostUpdate, render_swf);
    }
}

pub fn render_swf(
    mut commands: Commands,
    mut materials: ResMut<Assets<SWFColorMaterial>>,
    mut gradient_materials: ResMut<Assets<GradientMaterial>>,
    mut bitmap_materials: ResMut<Assets<BitmapMaterial>>,
    mut query: Query<(&mut Swf, &Transform)>,
    mut gizmos: Gizmos,
    entities_query: Query<Entity, With<Mesh2dHandle>>,
    mut swf_render_events: EventReader<SWFRenderEvent>,
) {
    for _swf_render_event in swf_render_events.read() {
        query.iter_mut().for_each(|(mut swf, transform)| {
            let render_list = swf.root_movie_clip.raw_container().render_list();
            let parent_transform = swf.root_movie_clip.base().transform().clone();
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
                &mut bitmap_materials,
                render_list,
                display_objects,
                &mut gizmos,
                &parent_transform,
                transform,
                &mut z_index,
            );
        });
    }
}

pub fn handler_render_list(
    commands: &mut Commands,
    materials: &mut ResMut<Assets<SWFColorMaterial>>,
    gradient_materials: &mut ResMut<Assets<GradientMaterial>>,
    bitmap_materials: &mut ResMut<Assets<BitmapMaterial>>,
    render_list: Arc<Vec<u128>>,
    display_objects: &mut BTreeMap<u128, DisplayObject>,
    gizmos: &mut Gizmos,
    parent_transform: &RuffleTransform,
    transform: &Transform,
    z_index: &mut f32,
) {
    for display_object in render_list.iter() {
        if let Some(display_object) = display_objects.get_mut(display_object) {
            match display_object {
                DisplayObject::Graphic(graphic) => {
                    if let Some(mesh) = graphic.mesh() {
                        let current_transform = graphic.base().transform();
                        let swf_transform = RuffleTransform {
                            matrix: parent_transform.matrix * current_transform.matrix,
                            color_transform: parent_transform.color_transform
                                * current_transform.color_transform,
                        };
                        commands.spawn(MaterialMesh2dBundle {
                            mesh: mesh.into(),
                            material: materials.add(SWFColorMaterial {
                                transform: swf_transform.clone().into(),
                            }),
                            transform: transform.with_translation(Vec3::new(
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
                                gradient_material.transform = swf_transform.clone().into();
                            }
                            commands.spawn(MaterialMesh2dBundle {
                                mesh: mesh_handle.clone().into(),
                                material: material.clone(),
                                transform: transform.with_translation(Vec3::new(
                                    0.0,
                                    0.0,
                                    *z_index + 0.1,
                                )),
                                ..Default::default()
                            });
                            *z_index += 0.1;
                        }
                        for (mesh_handle, material) in graphic.bitmap_mesh() {
                            // 这里引用同一个Graphic会指向同一个material
                            // dbg!(material.id());
                            // if let Some(bitmap_material) = bitmap_materials.get_mut(material.id()) {
                            //     bitmap_material.transform = transform.clone().into();
                            // }
                            // 暂时这样吧，以后看看有没有更好的渲染更新方式
                            let mut bitmap_material = material.clone();
                            bitmap_material.transform = swf_transform.clone().into();
                            commands.spawn(MaterialMesh2dBundle {
                                mesh: mesh_handle.clone().into(),
                                material: bitmap_materials.add(bitmap_material),
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
                    // dbg!(movie_clip.character_id(), movie_clip.depth());
                    // dbg!(movie_clip.blend_mode());
                    handler_render_list(
                        commands,
                        materials,
                        gradient_materials,
                        bitmap_materials,
                        movie_clip.raw_container().render_list(),
                        movie_clip.raw_container_mut().display_objects_mut(),
                        gizmos,
                        &current_transform,
                        transform,
                        z_index,
                    );
                }
            }
        }
    }
}
