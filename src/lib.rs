use std::collections::btree_map::ValuesMut;

use crate::assets::{ShapeMaterialType, Swf, SwfLoader};
use crate::commands::ShapeCommand;
use crate::player::{Flash, FlashPlayer};
use crate::render::FlashRenderPlugin;
use crate::render::blend_pipeline::BlendMode;
use crate::render::material::{BitmapMaterial, ColorMaterial, GradientMaterial, SwfMaterial};
use crate::swf_runtime::display_object::{DisplayObject, TDisplayObject};
use crate::swf_runtime::filter::Filter;
use crate::swf_runtime::matrix::Matrix;
use crate::swf_runtime::movie_clip::MovieClip;
use crate::swf_runtime::transform::{Transform as SwfTransform, TransformStack};

use bevy::app::{App, PostUpdate, Update};
use bevy::asset::{AssetEvent, Assets, Handle, RenderAssetUsages};
use bevy::color::Color;
use bevy::ecs::component::Component;
use bevy::ecs::entity::{Entity, EntityHashMap};
use bevy::ecs::event::{Event, EventReader};
use bevy::ecs::hierarchy::Children;
use bevy::ecs::query::With;
use bevy::ecs::resource::Resource;
use bevy::ecs::system::{Commands, EntityCommands, Local, Query, Res, ResMut};
use bevy::ecs::world::FromWorld;
use bevy::image::Image;
use bevy::log::{error, info, info_once, warn, warn_once};
use bevy::math::{IVec2, Vec3};
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::{Deref, DerefMut};
use bevy::render::mesh::{Indices, Mesh, Mesh2d, PrimitiveTopology};
use bevy::render::view::{NoFrustumCulling, Visibility};
use bevy::sprite::MeshMaterial2d;
use bevy::time::{Time, Timer, TimerMode};
use bevy::transform::components::{GlobalTransform, Transform};
use bevy::{app::Plugin, asset::AssetApp};
use swf::{CharacterId, Rectangle, Twips};

pub mod assets;
mod commands;
pub mod player;
mod render;
pub mod swf_runtime;

/// Flash 插件模块，为 Bevy 引入 Flash 动画。
pub struct FlashPlugin;

impl Plugin for FlashPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(FlashRenderPlugin)
            .init_asset::<Swf>()
            .init_asset_loader::<SwfLoader>()
            .init_resource::<BitmapMesh>()
            .init_resource::<FlashPlayerTimer>()
            .add_event::<FlashCompleteEvent>()
            .add_systems(Update, prepare_root_clip)
            .add_systems(PostUpdate, advance_animation);
    }
}

/// 所有Flash动画都设置为30FPS
#[derive(Resource, Debug, Clone, Deref, DerefMut)]
pub struct FlashPlayerTimer(Timer);

impl Default for FlashPlayerTimer {
    /// 动画定时器，默认30fps
    fn default() -> Self {
        Self(Timer::from_seconds(1. / 30., TimerMode::Repeating))
    }
}

/// 位图Mesh,是一个固定的矩形
#[derive(Resource, Debug, Clone, Deref, DerefMut)]
pub struct BitmapMesh(Handle<Mesh>);

impl FromWorld for BitmapMesh {
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

/// 准备Root MovieClip
fn prepare_root_clip(
    mut commands: Commands,
    mut player: Query<(Entity, &mut FlashPlayer, &Flash)>,
    swf_res: Res<Assets<Swf>>,
    mut asset_event: EventReader<AssetEvent<Swf>>,
) {
    for event in asset_event.read() {
        match event {
            AssetEvent::LoadedWithDependencies { id } => {
                if let Some((entity, mut player, _)) =
                    player.iter_mut().find(|(_, _, flash)| flash.id() == *id)
                {
                    let Some(swf) = swf_res.get(*id) else {
                        continue;
                    };
                    let mut root = MovieClip::new(swf.swf_movie.clone());
                    player.play_target_animation(&swf, &mut root);
                    commands.entity(entity).insert(root);
                }
            }
            _ => {}
        }
    }
}

pub struct ImageCacheEntity {
    handle: Handle<Image>,
    clear_color: Color,
    filters: Vec<Filter>,
    commands: Vec<ShapeCommand>,
}

pub struct RenderContext<'a> {
    transform_stack: &'a mut TransformStack,
    scale: Vec3,
    cache_draw: &'a mut Vec<ImageCacheEntity>,
    commands: Vec<ShapeCommand>,
    shape_meshes: &'a HashMap<CharacterId, Vec<(ShapeMaterialType, Handle<Mesh>)>>,
}

impl<'a> RenderContext<'a> {
    fn new(
        transform_stack: &'a mut TransformStack,
        scale: Vec3,
        cache_draw: &'a mut Vec<ImageCacheEntity>,
        commands: Vec<ShapeCommand>,
        shape_meshes: &'a HashMap<CharacterId, Vec<(ShapeMaterialType, Handle<Mesh>)>>,
    ) -> Self {
        Self {
            transform_stack,
            scale,
            cache_draw,
            commands,
            shape_meshes,
        }
    }
    pub fn render_shape(
        &mut self,
        id: u16,
        transform: SwfTransform,
        shape_depth_layer: String,
        blend_mode: BlendMode,
    ) {
        self.commands.push(ShapeCommand::RenderShape {
            transform,
            id,
            shape_depth_layer,
            blend_mode,
        });
    }
}

enum ShapeMaterialHandle {
    ColorMaterial(Handle<ColorMaterial>),
    GradientMaterial(Handle<GradientMaterial>),
    BitmapMaterial(Handle<BitmapMaterial>),
}

/// 标记Mesh2d实体为ShapeMesh
#[derive(Component)]
pub struct ShapeMesh;

/// 为 Flash 动画添加完成事件
#[derive(Event, Clone, Deref)]
pub struct FlashCompleteEvent {
    /// 当前播放的动画名
    pub animation_name: Option<Box<str>>,
}

/// 为 Flash 动画添加帧事件
#[derive(Event, Clone, Deref, DerefMut)]
pub struct FrameEvent(String);

/// 推进Flash动画
fn advance_animation(
    time: Res<Time>,
    bitmap_mesh_res: Res<BitmapMesh>,
    swf_res: Res<Assets<Swf>>,
    mut commands: Commands,
    mut timer: ResMut<FlashPlayerTimer>,
    mut player: Query<(
        Entity,
        &mut FlashPlayer,
        &mut MovieClip,
        &Flash,
        &GlobalTransform,
        Option<&Children>,
    )>,
    mut shape_mesh: Query<&mut Transform, With<ShapeMesh>>,
    mut images: ResMut<Assets<Image>>,
    mut color_materials: ResMut<Assets<ColorMaterial>>,
    mut gradient_materials: ResMut<Assets<GradientMaterial>>,
    mut bitmap_materials: ResMut<Assets<BitmapMaterial>>,
    mut bitmap_cache: Local<EntityHashMap<HashMap<CharacterId, Handle<BitmapMaterial>>>>,
    mut shape_material_entity_cache: Local<
        EntityHashMap<HashMap<String, Vec<(Entity, ShapeMaterialHandle)>>>,
    >,
) {
    let mut current_live_player = vec![];
    if timer.tick(time.delta()).just_finished() {
        for (entity, mut player, mut root, swf, global_transform, children) in player.iter_mut() {
            current_live_player.push(entity);

            if !player.is_looping() && player.is_completed() {
                // 触发完成事件
                if !player.completed() {
                    commands.entity(entity).trigger(FlashCompleteEvent {
                        animation_name: player.current_animation().map(|s| s.into()),
                    });
                    player.set_completed(true);
                }

                continue;
            }

            let Some(swf) = swf_res.get(swf.id()) else {
                continue;
            };

            if player.is_looping() && player.is_completed() {
                // 循环播放，跳回当前动画第一帧，
                player.play_target_animation(swf, root.as_mut());
            }

            let characters = &swf.characters;
            // 进入一帧
            root.enter_frame(characters);
            player.incr_frame();

            // 处理DisplayList
            let mut cache_draw = vec![];
            let mut transform_stack = TransformStack::default();
            let mut context = RenderContext::new(
                &mut transform_stack,
                global_transform.scale(),
                &mut cache_draw,
                Vec::new(),
                &swf.shape_meshes,
            );

            process_display_list(
                root.render_list_mut(),
                &mut context,
                swf::BlendMode::Normal,
                images.as_mut(),
                String::from("0"),
            );

            let shape_commands = context.commands;
            let shape_material_cache = shape_material_entity_cache.entry(entity).or_default();

            let mut current_live_shape_entity = vec![];
            spawn_or_update_shape(
                commands.entity(entity),
                color_materials.as_mut(),
                gradient_materials.as_mut(),
                bitmap_materials.as_mut(),
                &mut shape_mesh,
                shape_material_cache,
                cache_draw,
                shape_commands,
                &swf.shape_meshes,
                &mut current_live_shape_entity,
            );
            if let Some(children) = children {
                children.iter().for_each(|child| {
                    commands.entity(*child).insert(Visibility::Hidden);
                });
                current_live_shape_entity.iter().for_each(|entity| {
                    commands.entity(*entity).insert(Visibility::Inherited);
                });
            }
        }
        shape_material_entity_cache.retain(|entity, _| current_live_player.contains(entity));
    }
}

pub struct CacheInfo {
    handle: Handle<Image>,
    dirty: bool,
    base_transform: SwfTransform,
    bounds: Rectangle<Twips>,
    draw_offset: IVec2,
    filters: Vec<Filter>,
}

fn process_display_list(
    display_list: ValuesMut<'_, u16, DisplayObject>,
    context: &mut RenderContext<'_>,
    blend_mode: swf::BlendMode,
    images: &mut Assets<Image>,
    shape_depth_layer: String,
) {
    for display_object in display_list {
        let id = display_object.id();
        let shape_depth_layer = format!("{}_{}_{}", shape_depth_layer, display_object.depth(), id);
        context
            .transform_stack
            .push(display_object.base().transform());
        let blend_mode = if blend_mode == swf::BlendMode::Normal {
            display_object.blend_mode()
        } else {
            blend_mode
        };

        // 处理当前DisplayObject滤镜
        let mut cache_info = None;
        let base_transform = context.transform_stack.transform();
        let bounds = display_object.render_bounds_with_transform(
            &base_transform.matrix,
            false,
            context.scale,
        );

        let mut filters = display_object.filters();
        let swf_version = display_object.swf_version();
        filters.retain(|f| !f.impotent());

        if let Some(cache) = display_object.base_mut().cache_mut() {
            let width = bounds.width().to_pixels().ceil().max(0.);
            let height = bounds.height().to_pixels().ceil().max(0.);
            if width <= u16::MAX as f64 && height <= u16::MAX as f64 {
                let width = width as u16;
                let height = height as u16;
                let mut filter_rect = Rectangle {
                    x_min: Twips::ZERO,
                    y_min: Twips::ZERO,
                    x_max: Twips::from_pixels_i32(width as i32),
                    y_max: Twips::from_pixels_i32(height as i32),
                };
                let scale = context.scale;
                for filter in &mut filters {
                    filter.scale(scale.x, scale.y);
                    filter_rect = filter.calculate_dest_rect(filter_rect);
                }
                let filter_rect = Rectangle {
                    x_min: filter_rect.x_min.to_pixels().floor() as i32,
                    y_min: filter_rect.y_min.to_pixels().floor() as i32,
                    x_max: filter_rect.x_max.to_pixels().ceil() as i32,
                    y_max: filter_rect.y_max.to_pixels().ceil() as i32,
                };
                let draw_offset = IVec2::new(filter_rect.x_min, filter_rect.y_min);
                if cache.is_dirty(&base_transform.matrix, width, height) {
                    cache.update(
                        images,
                        &base_transform.matrix,
                        width,
                        height,
                        filter_rect.width() as u64,
                        filter_rect.height() as u64,
                        swf_version,
                        draw_offset,
                    );
                    cache_info = cache.handle().map(|handle| CacheInfo {
                        handle,
                        dirty: true,
                        base_transform,
                        bounds,
                        draw_offset,
                        filters,
                    });
                } else {
                    cache_info = cache.handle().map(|handle| CacheInfo {
                        handle,
                        dirty: false,
                        base_transform,
                        bounds,
                        draw_offset,
                        filters,
                    });
                }
            } else {
                warn_once!("缓存大小超出限制，已清除缓存, {}, ({width} x {height}", id);
                cache.clear();
                cache_info = None;
            }
        }

        if let Some(cache_info) = cache_info {
            let offset_x = cache_info.bounds.x_min - cache_info.base_transform.matrix.tx
                + Twips::from_pixels_i32(cache_info.draw_offset.x);
            let offset_y = cache_info.bounds.y_min - cache_info.base_transform.matrix.ty
                + Twips::from_pixels_i32(cache_info.draw_offset.y);
            if cache_info.dirty {
                let mut transform_stack = TransformStack::new();
                transform_stack.push(&SwfTransform {
                    color_transform: Default::default(),
                    matrix: Matrix {
                        tx: -offset_x,
                        ty: -offset_y,
                        ..cache_info.base_transform.matrix
                    },
                });
                // 中间纹理绘制
                let mut offscreen_context = RenderContext {
                    transform_stack: &mut transform_stack,
                    scale: context.scale,
                    cache_draw: context.cache_draw,
                    commands: Vec::new(),
                    shape_meshes: context.shape_meshes,
                };
                render_display_object(
                    display_object,
                    &mut offscreen_context,
                    blend_mode,
                    images,
                    shape_depth_layer,
                );
                // 将offscreen_context需要缓存绘制的render_shapes合并到context.cache_draw.render_shapes
                offscreen_context.cache_draw.push(ImageCacheEntity {
                    handle: cache_info.handle.clone(),
                    clear_color: Color::NONE,
                    commands: offscreen_context.commands,
                    filters: cache_info.filters,
                });
            }
            // 引用绘制出来的中间纹理，按照位图材质的方式绘制到view
            let matrix = context.transform_stack.transform().matrix;
            context.commands.push(ShapeCommand::RenderBitmap {
                transform: SwfTransform {
                    matrix: Matrix {
                        tx: matrix.tx,
                        ty: matrix.ty,
                        ..Default::default()
                    },
                    color_transform: cache_info.base_transform.color_transform,
                },
                id,
                handle: cache_info.handle,
            });
        } else {
            // 绘制 background，大概率不重要
            // TODO:
            // 绘制自己
            render_display_object(
                display_object,
                context,
                blend_mode,
                images,
                shape_depth_layer,
            );
        }

        // TODO:处理复杂混合模式

        context.transform_stack.pop();
    }
}

fn render_display_object(
    display_object: &mut DisplayObject,
    context: &mut RenderContext<'_>,
    blend_mode: swf::BlendMode,
    images: &mut Assets<Image>,
    shape_depth_layer: String,
) {
    match display_object {
        DisplayObject::MovieClip(movie_clip) => {
            process_display_list(
                movie_clip.render_list_mut(),
                context,
                blend_mode,
                images,
                shape_depth_layer,
            );
        }
        DisplayObject::Graphic(graphic) => {
            graphic.render_self(context, blend_mode, shape_depth_layer);
        }
        DisplayObject::MorphShape(morph_shape) => {
            morph_shape.render_self(context, blend_mode, shape_depth_layer);
        }
    }
}

fn spawn_or_update_shape(
    mut commands: EntityCommands<'_>,
    color_materials: &mut Assets<ColorMaterial>,
    gradient_materials: &mut Assets<GradientMaterial>,
    bitmap_materials: &mut Assets<BitmapMaterial>,
    shape_mesh: &mut Query<&mut Transform, With<ShapeMesh>>,
    shape_material_cache: &mut HashMap<String, Vec<(Entity, ShapeMaterialHandle)>>,
    cache_draw: Vec<ImageCacheEntity>,
    shape_commands: Vec<ShapeCommand>,
    shape_meshes: &HashMap<CharacterId, Vec<(ShapeMaterialType, Handle<Mesh>)>>,
    current_live_shape_entity: &mut Vec<Entity>,
) {
    // 当前根据shape_commands 生成的shape layer 用于记录是否多次引用了同一个shape, 避免重复生成
    let mut current_shape_depth_layers = HashSet::new();

    // 1. 处理需要绘制中间纹理的Shape
    for cache_entity in cache_draw {
        // TODO:
    }

    // 2. 处理不需要缓存的Shape
    let mut z_index = 0.;
    for (index, shape_command) in shape_commands.iter().enumerate() {
        z_index += index as f32 * 0.001;
        match shape_command {
            ShapeCommand::RenderShape {
                transform: swf_transform,
                id,
                shape_depth_layer,
                blend_mode,
            } => {
                let Some(shape_meshes) = shape_meshes.get(id) else {
                    continue;
                };
                if let Some(shape_material_handles_cache) = get_shape_material_handle_cache(
                    id,
                    shape_depth_layer,
                    shape_material_cache,
                    &mut current_shape_depth_layers,
                )
                // if let Some(shape_material_handles_cache) =
                //     shape_material_cache.get(shape_depth_layer)
                {
                    for (index, (entity, handle)) in shape_material_handles_cache.iter().enumerate()
                    {
                        z_index += index as f32 * 0.001;
                        let Ok(mut transform) = shape_mesh.get_mut(*entity) else {
                            continue;
                        };
                        transform.translation.z = z_index;
                        match handle {
                            ShapeMaterialHandle::ColorMaterial(color) => {
                                update_material(color, color_materials, swf_transform, blend_mode);
                            }
                            ShapeMaterialHandle::GradientMaterial(gradient) => {
                                update_material(
                                    gradient,
                                    gradient_materials,
                                    swf_transform,
                                    blend_mode,
                                );
                            }
                            ShapeMaterialHandle::BitmapMaterial(bitmap) => {
                                update_material(
                                    bitmap,
                                    bitmap_materials,
                                    swf_transform,
                                    blend_mode,
                                );
                            }
                        }
                        current_live_shape_entity.push(*entity);
                    }
                    continue;
                }
                // 该Shape没有生成过，需要生成
                let shape_material_handles_cache = shape_material_cache
                    .entry(shape_depth_layer.to_owned())
                    .or_default();
                for (material_type, mesh) in shape_meshes {
                    let mesh = mesh.clone();
                    match material_type {
                        ShapeMaterialType::Color(color) => {
                            // let Some(material) = color_materials.get_mut(color.id()) else {
                            //     continue;
                            // };
                            // material.transform = (*transform).into();

                            let mut color_material = *color;
                            color_material.update_swf_material((*swf_transform).into());
                            color_material.set_blend_key((*blend_mode).into());
                            let color = color_materials.add(color_material);

                            commands.with_children(|parent| {
                                let entity = parent
                                    .spawn((
                                        Mesh2d(mesh),
                                        MeshMaterial2d(color.clone()),
                                        Transform::from_translation(Vec3::Z * z_index),
                                        ShapeMesh,
                                        // 由于Flash顶点特殊性不应用剔除
                                        NoFrustumCulling,
                                    ))
                                    .id();

                                shape_material_handles_cache.push((
                                    entity,
                                    ShapeMaterialHandle::ColorMaterial(color.clone()),
                                ));
                                current_live_shape_entity.push(entity);
                            });
                        }
                        ShapeMaterialType::Gradient(gradient) => {
                            // let Some(material) = gradient_materials.get_mut(gradient.id()) else {
                            //     continue;
                            // };
                            // material.transform = (*transform).into();
                            let mut gradient_material = gradient.clone();
                            gradient_material.update_swf_material((*swf_transform).into());
                            gradient_material.set_blend_key((*blend_mode).into());
                            let gradient = gradient_materials.add(gradient_material);
                            commands.with_children(|parent| {
                                let entity = parent
                                    .spawn((
                                        Mesh2d(mesh),
                                        MeshMaterial2d(gradient.clone()),
                                        Transform::from_translation(Vec3::Z * z_index),
                                        ShapeMesh,
                                        // 由于Flash顶点特殊性不应用剔除
                                        NoFrustumCulling,
                                    ))
                                    .id();
                                shape_material_handles_cache.push((
                                    entity,
                                    ShapeMaterialHandle::GradientMaterial(gradient.clone()),
                                ));
                                current_live_shape_entity.push(entity);
                            });
                        }
                        ShapeMaterialType::Bitmap(bitmap) => {
                            // let Some(material) = bitmap_materials.get_mut(bitmap.id()) else {
                            //     continue;
                            // };
                            // material.transform = (*transform).into();
                            let mut bitmap_material = bitmap.clone();
                            bitmap_material.update_swf_material((*swf_transform).into());
                            bitmap_material.set_blend_key((*blend_mode).into());
                            let bitmap = bitmap_materials.add(bitmap_material);
                            commands.with_children(|parent| {
                                let entity = parent
                                    .spawn((
                                        Mesh2d(mesh),
                                        MeshMaterial2d(bitmap.clone()),
                                        Transform::from_translation(Vec3::Z * z_index),
                                        ShapeMesh,
                                        // 由于Flash顶点特殊性不应用剔除
                                        NoFrustumCulling,
                                    ))
                                    .id();
                                shape_material_handles_cache.push((
                                    entity,
                                    ShapeMaterialHandle::BitmapMaterial(bitmap.clone()),
                                ));
                                current_live_shape_entity.push(entity);
                            });
                        }
                    }
                }
            }
            ShapeCommand::RenderBitmap {
                transform,
                id,
                handle,
            } => {}
        }
    }
}

fn update_material<T: SwfMaterial>(
    handle: &Handle<T>,
    swf_materials: &mut Assets<T>,
    swf_transform: &SwfTransform,
    blend_mode: &BlendMode,
) {
    // 当缓存某实体后该实体在该系统尚未运行完成时会查询不到对应的材质，此时重新生成材质。
    if let Some(swf_material) = swf_materials.get_mut(handle) {
        swf_material.update_swf_material((*swf_transform).into());
        swf_material.set_blend_key((*blend_mode).into());
    }
}

/// 尽量复用已经生成的实体。只有在同一帧同一个 shape被多次使用时才需要重新生成
fn get_shape_material_handle_cache<'a>(
    id: &CharacterId,
    shape_depth_layer: &String,
    shape_material_cache: &'a HashMap<String, Vec<(Entity, ShapeMaterialHandle)>>,
    current_shape_depth_layers: &mut HashSet<String>,
) -> Option<&'a Vec<(Entity, ShapeMaterialHandle)>> {
    // 如果当前id已经生成过，则根据深度层获取缓存
    if let Some(shape_material_handles_cache) = shape_material_cache.get(shape_depth_layer) {
        current_shape_depth_layers.insert(shape_depth_layer.to_owned());
        Some(shape_material_handles_cache)
    } else {
        // 从shape_material_cache中获取key值最后一个“_"后的字符匹配id
        if let Some((k, shape_material_handles_cache)) = shape_material_cache
            .iter()
            .filter(|(k, _)| !current_shape_depth_layers.contains(k.as_str()))
            .find(|(key, _)| {
                key.split("_")
                    .last()
                    .map(|v| v == id.to_string())
                    .unwrap_or(false)
            })
            .map(|(k, v)| (k, v))
        {
            current_shape_depth_layers.insert(k.to_owned());
            Some(shape_material_handles_cache)
        } else {
            None
        }
    }
}
