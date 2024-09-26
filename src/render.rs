use std::{collections::BTreeMap, sync::Arc};

use bevy::{
    app::{App, Plugin, PostUpdate, Update},
    asset::{load_internal_asset, Assets, Handle},
    math::{Mat4, Vec3},
    prelude::{
        BuildChildren, Commands, Component, Entity, IntoSystemConfigs, Mesh, Query, Res, ResMut,
        Shader, Transform, Visibility, Without,
    },
    render::view::{NoFrustumCulling, VisibilitySystems},
    sprite::{Material2dPlugin, MaterialMesh2dBundle, Mesh2dHandle},
};
use material::{BitmapMaterial, GradientMaterial, SWFColorMaterial, SWFTransform};
use ruffle_render::transform::Transform as RuffleTransform;

use crate::{
    bundle::{ShapeMark, ShapeMarkEntities, Swf, SwfState},
    plugin::ShapeDrawType,
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
            .add_systems(Update, render_swf)
            .add_systems(
                PostUpdate,
                calculate_shape_bounds.in_set(VisibilitySystems::CalculateBounds),
            );
    }
}

#[derive(Component, Default)]
pub struct SWFShapeMesh {
    transform: Mat4,
}

pub fn render_swf(
    mut commands: Commands,
    mut color_materials: ResMut<Assets<SWFColorMaterial>>,
    mut gradient_materials: ResMut<Assets<GradientMaterial>>,
    mut bitmap_materials: ResMut<Assets<BitmapMaterial>>,
    mut query: Query<(&mut Swf, Entity, &mut ShapeMarkEntities)>,
    mut entities_material_query: Query<(
        Entity,
        Option<&Handle<SWFColorMaterial>>,
        Option<&Handle<GradientMaterial>>,
        Option<&Handle<BitmapMaterial>>,
        &mut SWFShapeMesh,
    )>,
) {
    for (mut swf, entity, mut shape_mark_entities) in query.iter_mut() {
        match swf.status {
            SwfState::Loading => {
                continue;
            }
            SwfState::Ready => {
                shape_mark_entities.clear_current_frame_entity();
                let render_list = swf.root_movie_clip.raw_container().render_list();
                let parent_clip_transform = swf.root_movie_clip.base().transform().clone();
                let display_objects = swf
                    .root_movie_clip
                    .raw_container_mut()
                    .display_objects_mut();

                let mut z_index = 0.000;
                let mut depth_layer = (String::from(""), String::from(""));

                handler_render_list(
                    entity,
                    &mut commands,
                    &mut color_materials,
                    &mut gradient_materials,
                    &mut bitmap_materials,
                    &mut entities_material_query,
                    shape_mark_entities.as_mut(),
                    render_list,
                    display_objects,
                    &parent_clip_transform,
                    &mut z_index,
                    &mut depth_layer,
                );

                shape_mark_entities
                    .graphic_entities()
                    .iter()
                    .for_each(|(_, entity)| {
                        commands.entity(*entity).insert(Visibility::Hidden);
                    });
                shape_mark_entities
                    .current_frame_entities()
                    .iter()
                    .for_each(|shape_mark| {
                        let entity = shape_mark_entities.entity(shape_mark).unwrap();
                        commands.entity(*entity).insert(Visibility::Inherited);
                    });
                swf.status = SwfState::Loading;
            }
        }
    }
}

pub fn handler_render_list(
    parent_entity: Entity,
    commands: &mut Commands,
    color_materials: &mut ResMut<Assets<SWFColorMaterial>>,
    gradient_materials: &mut ResMut<Assets<GradientMaterial>>,
    bitmap_materials: &mut ResMut<Assets<BitmapMaterial>>,
    entities_material_query: &mut Query<
        '_,
        '_,
        (
            Entity,
            Option<&Handle<SWFColorMaterial>>,
            Option<&Handle<GradientMaterial>>,
            Option<&Handle<BitmapMaterial>>,
            &mut SWFShapeMesh,
        ),
    >,
    shape_mark_entities: &mut ShapeMarkEntities,
    render_list: Arc<Vec<u128>>,
    display_objects: &mut BTreeMap<u128, DisplayObject>,
    parent_clip_transform: &RuffleTransform,
    z_index: &mut f32,
    depth_layer: &mut (String, String),
) {
    for display_object in render_list.iter() {
        if let Some(display_object) = display_objects.get_mut(display_object) {
            match display_object {
                DisplayObject::Graphic(graphic) => {
                    let current_transform = graphic.base().transform();
                    let swf_transform: SWFTransform = RuffleTransform {
                        matrix: parent_clip_transform.matrix * current_transform.matrix,
                        color_transform: parent_clip_transform.color_transform
                            * current_transform.color_transform,
                    }
                    .into();
                    let mut shape_mark = ShapeMark {
                        parent_layer: depth_layer.clone(),
                        depth: graphic.depth(),
                        id: graphic.character_id(),
                        graphic_index: 0,
                    };
                    graphic
                        .shape_mesh()
                        .iter_mut()
                        .enumerate()
                        .for_each(|(index, shape)| {
                            // 记录当前帧生成的mesh实体
                            shape_mark.graphic_index = index;
                            shape_mark_entities.record_current_frame_entity(shape_mark.clone());

                            if let Some(&existing_entity) = shape_mark_entities.entity(&shape_mark)
                            {
                                let exists_material = entities_material_query
                                    .iter_mut()
                                    .find(|(entity, _, _, _, _)| *entity == existing_entity);
                                match shape.draw_type.clone() {
                                    ShapeDrawType::Color(mut swf_color_material) => {
                                        // 当缓存某实体后该实体在该系统尚未运行完成时会查询不到对应的材质，此时重新生成材质
                                        if let Some(exists_material) = exists_material {
                                            let color_material =
                                                exists_material.1.expect("未找到颜色填充材质");
                                            if let Some(swf_color_material) =
                                                color_materials.get_mut(color_material)
                                            {
                                                swf_color_material.transform =
                                                    swf_transform.clone();
                                                let mut swf_shape_mesh = exists_material.4;
                                                swf_shape_mesh.transform =
                                                    swf_transform.world_transform;
                                            }
                                        } else {
                                            swf_color_material.transform = swf_transform.clone();
                                            let swf_shape_mesh = SWFShapeMesh {
                                                transform: swf_transform.world_transform,
                                            };
                                            commands.entity(existing_entity).insert((
                                                color_materials.add(swf_color_material),
                                                swf_shape_mesh,
                                            ));
                                        }
                                    }
                                    ShapeDrawType::Gradient(handle) => {
                                        // 此处是否应该从handle直接获取，而不应该使用exists_material
                                        if let Some(exists_material) = exists_material {
                                            let gradient_material =
                                                exists_material.2.expect("未找到渐变色填充材质");
                                            if let Some(swf_gradient_material) =
                                                gradient_materials.get_mut(gradient_material)
                                            {
                                                swf_gradient_material.transform =
                                                    swf_transform.clone();
                                                let mut swf_shape_mesh = exists_material.4;
                                                swf_shape_mesh.transform =
                                                    swf_transform.world_transform;
                                            }
                                        } else {
                                            if let Some(swf_gradient_material) =
                                                gradient_materials.get_mut(&handle)
                                            {
                                                swf_gradient_material.transform =
                                                    swf_transform.clone();
                                                let swf_shape_mesh = SWFShapeMesh {
                                                    transform: swf_transform.world_transform,
                                                };
                                                commands
                                                    .entity(existing_entity)
                                                    .insert(swf_shape_mesh);
                                            }
                                        }
                                    }
                                    ShapeDrawType::Bitmap(mut bitmap_material) => {
                                        if let Some(exists_material) = exists_material {
                                            let bitmap_material =
                                                exists_material.3.expect("未找到渐变色填充材质");
                                            if let Some(swf_bitmap_material) =
                                                bitmap_materials.get_mut(bitmap_material)
                                            {
                                                swf_bitmap_material.transform =
                                                    swf_transform.clone();
                                                let mut swf_shape_mesh = exists_material.4;
                                                swf_shape_mesh.transform =
                                                    swf_transform.world_transform;
                                            }
                                        } else {
                                            bitmap_material.transform = swf_transform.clone();
                                            let swf_shape_mesh = SWFShapeMesh {
                                                transform: swf_transform.world_transform,
                                            };
                                            commands.entity(existing_entity).insert((
                                                bitmap_materials.add(bitmap_material),
                                                swf_shape_mesh,
                                            ));
                                        }
                                    }
                                }
                            } else {
                                let transform =
                                    Transform::from_translation(Vec3::new(0.0, 0.0, *z_index));
                                match &mut shape.draw_type {
                                    ShapeDrawType::Color(swf_color_material) => {
                                        swf_color_material.transform = swf_transform.clone().into();
                                        let aabb_transform =
                                            swf_color_material.transform.world_transform;
                                        commands.entity(parent_entity).with_children(|parent| {
                                            let entity = parent
                                                .spawn((
                                                    MaterialMesh2dBundle {
                                                        mesh: shape.mesh.clone().into(),
                                                        material: color_materials
                                                            .add(swf_color_material.clone()),
                                                        transform,
                                                        ..Default::default()
                                                    },
                                                    SWFShapeMesh {
                                                        transform: aabb_transform,
                                                    },
                                                ))
                                                .id();
                                            shape_mark_entities
                                                .add_entities_pool(shape_mark.clone(), entity);
                                        });
                                    }
                                    ShapeDrawType::Gradient(materials_handle) => {
                                        let mut aabb_transform = Mat4::default();
                                        if let Some(gradient_material) =
                                            gradient_materials.get_mut(materials_handle.id())
                                        {
                                            gradient_material.transform =
                                                swf_transform.clone().into();
                                            aabb_transform =
                                                gradient_material.transform.world_transform;
                                        }
                                        commands.entity(parent_entity).with_children(|parent| {
                                            let entity = parent
                                                .spawn((
                                                    MaterialMesh2dBundle {
                                                        mesh: shape.mesh.clone().into(),
                                                        material: materials_handle.clone(),
                                                        transform,
                                                        ..Default::default()
                                                    },
                                                    SWFShapeMesh {
                                                        transform: aabb_transform,
                                                    },
                                                ))
                                                .id();
                                            shape_mark_entities
                                                .add_entities_pool(shape_mark.clone(), entity);
                                        });
                                    }
                                    ShapeDrawType::Bitmap(bitmap_material) => {
                                        let mut bitmap_material = bitmap_material.clone();
                                        bitmap_material.transform = swf_transform.clone().into();
                                        let aabb_transform =
                                            bitmap_material.transform.world_transform;
                                        commands.entity(parent_entity).with_children(|parent| {
                                            let entity = parent
                                                .spawn((
                                                    MaterialMesh2dBundle {
                                                        mesh: shape.mesh.clone().into(),
                                                        material: bitmap_materials
                                                            .add(bitmap_material),
                                                        transform,
                                                        ..Default::default()
                                                    },
                                                    SWFShapeMesh {
                                                        transform: aabb_transform,
                                                    },
                                                ))
                                                .id();
                                            shape_mark_entities
                                                .add_entities_pool(shape_mark.clone(), entity);
                                        });
                                    }
                                }
                            }
                            *z_index += 0.001;
                        });
                }
                DisplayObject::MovieClip(movie_clip) => {
                    let current_transform = RuffleTransform {
                        matrix: parent_clip_transform.matrix * movie_clip.base().transform().matrix,
                        color_transform: parent_clip_transform.color_transform
                            * movie_clip.base().transform().color_transform,
                    };
                    depth_layer
                        .0
                        .push_str(&movie_clip.character_id().to_string());
                    depth_layer.0.push_str(&movie_clip.depth().to_string());
                    *z_index += movie_clip.depth() as f32 / 100.0;
                    // dbg!(movie_clip.character_id(), movie_clip.depth());
                    // dbg!(movie_clip.blend_mode());
                    handler_render_list(
                        parent_entity,
                        commands,
                        color_materials,
                        gradient_materials,
                        bitmap_materials,
                        entities_material_query,
                        shape_mark_entities,
                        movie_clip.raw_container().render_list(),
                        movie_clip.raw_container_mut().display_objects_mut(),
                        &current_transform,
                        z_index,
                        depth_layer,
                    );
                }
            }
        }
    }
}

pub fn calculate_shape_bounds(
    mut commands: Commands,
    meshes: Res<Assets<Mesh>>,
    meshes_without_aabb: Query<(Entity, &Mesh2dHandle, &SWFShapeMesh), Without<NoFrustumCulling>>,
) {
    meshes_without_aabb
        .iter()
        .for_each(|(entity, mesh_handle, swf_shape_mesh)| {
            if let Some(mesh) = meshes.get(&mesh_handle.0) {
                if let Some(mut aabb) = mesh.compute_aabb() {
                    let swf_transform = Mat4::from_cols_array_2d(&[
                        [1.0, 0.0, 0.0, 0.0],
                        [0.0, -1.0, 0.0, 0.0],
                        [0.0, 0.0, 1.0, 0.0],
                        [0.0, 0.0, 0.0, 1.0],
                    ]) * swf_shape_mesh.transform;
                    aabb.center = swf_transform.transform_point3a(aabb.center);
                    commands.entity(entity).try_insert(aabb);
                }
            }
        });
}
