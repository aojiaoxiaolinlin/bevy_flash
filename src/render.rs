use std::{collections::BTreeMap, sync::Arc};

use bevy::{
    app::{App, Plugin, PostUpdate},
    asset::{load_internal_asset, Assets, Handle},
    prelude::{
        BuildChildren, Commands, Component, Entity, EventReader, Mut, Query, ResMut, Shader,
        Transform, Visibility, With,
    },
    sprite::{Material2dPlugin, MaterialMesh2dBundle},
};
use material::{BitmapMaterial, GradientMaterial, SWFColorMaterial, SWFTransform};
use ruffle_render::transform::Transform as RuffleTransform;

use crate::{
    bundle::{ShapeMark, ShapeMarkEntities, Swf, SwfState},
    plugin::{SWFRenderEvent, ShapeDrawType},
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

#[derive(Component)]
pub struct SWFShapeMesh;

pub fn render_swf(
    mut commands: Commands,
    mut color_materials: ResMut<Assets<SWFColorMaterial>>,
    mut gradient_materials: ResMut<Assets<GradientMaterial>>,
    mut bitmap_materials: ResMut<Assets<BitmapMaterial>>,
    mut query: Query<(&mut Swf, Entity, &mut ShapeMarkEntities)>,
    mut entities_material_query: Query<
        (
            Entity,
            Option<&Handle<SWFColorMaterial>>,
            Option<&Handle<GradientMaterial>>,
            Option<&Handle<BitmapMaterial>>,
            &mut Transform,
        ),
        With<SWFShapeMesh>,
    >,
    mut swf_render_events: EventReader<SWFRenderEvent>,
) {
    for _swf_render_event in swf_render_events.read() {
        for (mut swf, entity, mut shape_mark_entities) in query.iter_mut() {
            match swf.status {
                SwfState::Loading => {
                    continue;
                }
                SwfState::Ready => {
                    let render_list = swf.root_movie_clip.raw_container().render_list();
                    let parent_clip_transform = swf.root_movie_clip.base().transform().clone();
                    let display_objects = swf
                        .root_movie_clip
                        .raw_container_mut()
                        .display_objects_mut();

                    let mut z_index = 0.000;

                    shape_mark_entities.clear_current_frame_entity();

                    handler_render_list(
                        entity,
                        &mut commands,
                        &mut color_materials,
                        &mut gradient_materials,
                        &mut bitmap_materials,
                        &mut entities_material_query,
                        &mut shape_mark_entities,
                        render_list,
                        display_objects,
                        &parent_clip_transform,
                        &mut z_index,
                    );

                    shape_mark_entities
                        .non_current_frame_entity()
                        .iter_mut()
                        .for_each(|entity| {
                            commands.entity(**entity).insert(Visibility::Hidden);
                        });
                    shape_mark_entities
                        .current_frame_entities()
                        .iter()
                        .for_each(|shape_mark| {
                            let entity = shape_mark_entities.entity(shape_mark).unwrap();
                            commands.entity(*entity).insert(Visibility::Inherited);
                        });
                }
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
            &mut Transform,
        ),
        With<SWFShapeMesh>,
    >,
    shape_mark_entities: &mut Mut<'_, ShapeMarkEntities>,
    render_list: Arc<Vec<u128>>,
    display_objects: &mut BTreeMap<u128, DisplayObject>,
    parent_clip_transform: &RuffleTransform,
    z_index: &mut f32,
) {
    for display_object in render_list.iter() {
        if let Some(display_object) = display_objects.get_mut(display_object) {
            match display_object {
                DisplayObject::Graphic(graphic) => {
                    let current_transform = graphic.base().transform();
                    let swf_transform = RuffleTransform {
                        matrix: parent_clip_transform.matrix * current_transform.matrix,
                        color_transform: parent_clip_transform.color_transform
                            * current_transform.color_transform,
                    };
                    let swf_transform: SWFTransform = swf_transform.clone().into();
                    let color_transform = swf_transform.1;

                    let mut shape_mark = ShapeMark {
                        depth: graphic.depth(),
                        id: graphic.character_id(),
                        graphic_index: 0,
                    };
                    graphic
                        .shape_mesh()
                        .iter_mut()
                        .enumerate()
                        .for_each(|(index, shape)| {
                            let mut transform = swf_transform.0;

                            // 记录当前帧生成的mesh实体
                            shape_mark.graphic_index = index;
                            shape_mark_entities.record_current_frame_entity(shape_mark);

                            if let Some(&existing_entity) = shape_mark_entities.entity(&shape_mark)
                            {
                                let exists_material = entities_material_query
                                    .iter_mut()
                                    .find(|(entity, _, _, _, _)| *entity == existing_entity);
                                // TODO: 以下考虑使用宏解决重复代码
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
                                                    color_transform.clone();

                                                let mut exists_transform = exists_material.4;
                                                transform.translation.z =
                                                    exists_transform.translation.z;
                                                *exists_transform = transform;
                                            }
                                        } else {
                                            transform.translation.z = *z_index;
                                            // 由于本系统执行期间无法查询本系统生成的实体所以此时无法复用，新建
                                            swf_color_material.transform = color_transform.clone();
                                            commands.entity(existing_entity).insert((
                                                color_materials.add(swf_color_material.clone()),
                                                transform,
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
                                                    color_transform.clone();
                                                let mut exists_transform = exists_material.4;

                                                transform.translation.z =
                                                    exists_transform.translation.z;
                                                *exists_transform = transform;
                                            }
                                        } else {
                                            transform.translation.z = *z_index;
                                            if let Some(swf_gradient_material) =
                                                gradient_materials.get_mut(&handle)
                                            {
                                                swf_gradient_material.transform =
                                                    color_transform.clone();
                                                commands.entity(existing_entity).insert(transform);
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
                                                    color_transform.clone();
                                                let mut exists_transform = exists_material.4;
                                                transform.translation.z =
                                                    exists_transform.translation.z;
                                                *exists_transform = transform;
                                            }
                                        } else {
                                            bitmap_material.transform = color_transform.clone();
                                            commands.entity(existing_entity).insert((
                                                bitmap_materials.add(bitmap_material.clone()),
                                                transform,
                                            ));
                                        }
                                    }
                                }
                            } else {
                                transform.translation.z = transform.translation.z + *z_index;
                                match &mut shape.draw_type {
                                    ShapeDrawType::Color(swf_color_material) => {
                                        swf_color_material.transform = color_transform.clone();

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
                                                    SWFShapeMesh,
                                                ))
                                                .id();
                                            shape_mark_entities
                                                .add_entities_pool(shape_mark, entity);
                                        });
                                    }
                                    ShapeDrawType::Gradient(materials_handle) => {
                                        if let Some(gradient_material) =
                                            gradient_materials.get_mut(materials_handle.id())
                                        {
                                            gradient_material.transform = color_transform.clone();
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
                                                    SWFShapeMesh,
                                                ))
                                                .id();
                                            shape_mark_entities
                                                .add_entities_pool(shape_mark, entity);
                                        });
                                    }
                                    ShapeDrawType::Bitmap(bitmap_material) => {
                                        let mut bitmap_material = bitmap_material.clone();
                                        bitmap_material.transform = color_transform.clone();
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
                                                    SWFShapeMesh,
                                                ))
                                                .id();
                                            shape_mark_entities
                                                .add_entities_pool(shape_mark, entity);
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
                    );
                }
            }
        }
    }
}
