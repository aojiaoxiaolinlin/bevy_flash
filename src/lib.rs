pub mod assets;
mod commands;
pub mod player;
mod render;
pub mod swf_runtime;

use std::collections::btree_map::ValuesMut;

use crate::assets::{ShapeMaterialType, Swf, SwfLoader};
use crate::commands::{MaterialType, OffscreenDrawCommands, ShapeCommand, ShapeMeshDraw};
use crate::player::{Flash, FlashPlayer};
use crate::render::FlashRenderPlugin;
use crate::render::blend_pipeline::BlendMode;
use crate::render::material::{
    BitmapMaterial, BlendMaterialKey, ColorMaterial, GradientMaterial, SwfMaterial,
};
use crate::render::offscreen_texture::{OffscreenMesh, OffscreenTexture};
use crate::swf_runtime::display_object::{
    DisplayObject, ImageCache, ImageCacheInfo, TDisplayObject,
};
use crate::swf_runtime::filter::Filter;
use crate::swf_runtime::matrix::Matrix;
use crate::swf_runtime::morph_shape::Frame;
use crate::swf_runtime::movie_clip::MovieClip;
use crate::swf_runtime::transform::{Transform as SwfTransform, TransformStack};

use bevy::app::{App, PostUpdate, Update};
use bevy::asset::{AssetEvent, Assets, Handle, RenderAssetUsages};
use bevy::color::Color;
use bevy::ecs::component::Component;
use bevy::ecs::entity::{Entity, EntityHashMap};
use bevy::ecs::event::{Event, EventReader};
use bevy::ecs::hierarchy::Children;
use bevy::ecs::query::{Or, With};
use bevy::ecs::resource::Resource;
use bevy::ecs::system::{Commands, EntityCommands, Local, Query, Res, ResMut};
use bevy::ecs::world::FromWorld;
use bevy::image::Image;
use bevy::log::warn_once;
use bevy::math::{IVec2, Mat4, UVec2, Vec2, Vec3, Vec3Swizzles};
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::{Deref, DerefMut};
use bevy::render::mesh::{Indices, Mesh, Mesh2d, PrimitiveTopology};
use bevy::render::view::{NoFrustumCulling, Visibility};
use bevy::sprite::MeshMaterial2d;
use bevy::time::{Time, Timer, TimerMode};
use bevy::transform::components::{GlobalTransform, Transform};
use bevy::{app::Plugin, asset::AssetApp};
use swf::{CharacterId, Rectangle, Twips};

/// 用于缓存每个实体对应的显示对象
#[derive(Default)]
struct DisplayObjectCache {
    morph_shape_frame_cache: HashMap<CharacterId, fnv::FnvHashMap<u16, Frame>>,
    layer_shape_material_cache: HashMap<String, Vec<(Entity, MaterialType)>>,
    layer_offscreen_shape_draw_cache: HashMap<String, Vec<ShapeMeshDraw>>,
    layer_offscreen_cache: HashMap<String, Entity>,
    image_cache: HashMap<CharacterId, ImageCache>,
}
/// Flash 插件模块，为 Bevy 引入 Flash 动画。
pub struct FlashPlugin;

impl Plugin for FlashPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(FlashRenderPlugin)
            .init_asset::<Swf>()
            .init_asset_loader::<SwfLoader>()
            .init_resource::<FilterTextureMesh>()
            .init_resource::<FlashPlayerTimer>()
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

/// 用于滤镜纹理渲染的Mesh，一个固定的矩形
#[derive(Resource, Debug, Clone, Deref, DerefMut)]
/// 用于滤镜纹理渲染的固定矩形网格
pub struct FilterTextureMesh(Handle<Mesh>);

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

#[derive(Debug)]
pub struct ImageCacheDraw {
    layer: String,
    id: CharacterId,
    handle: Handle<Image>,
    clear_color: Color,
    filters: Vec<Filter>,
    commands: Vec<ShapeCommand>,
    dirty: bool,
    size: UVec2,
}

pub struct RenderContext<'a> {
    // 系统资源
    meshes: &'a mut Assets<Mesh>,
    images: &'a mut Assets<Image>,
    gradients: &'a mut Assets<GradientMaterial>,
    bitmaps: &'a mut Assets<BitmapMaterial>,

    // 渲染需要的数据
    transform_stack: &'a mut TransformStack,
    scale: Vec3,
    cache_draws: &'a mut Vec<ImageCacheDraw>,
    commands: Vec<ShapeCommand>,
    shape_mesh_materials: &'a mut HashMap<CharacterId, Vec<(ShapeMaterialType, Handle<Mesh>)>>,

    // 缓存相关
    /// 变形形状纹理缓存，
    /// TODO: 需要储存在SWF Assets 中
    morph_shape_cache: &'a mut HashMap<CharacterId, fnv::FnvHashMap<u16, Frame>>,
    /// Image 缓存
    image_caches: &'a mut HashMap<CharacterId, ImageCache>,
}

type ShapeMaterialAssets<'a> = (
    &'a mut Assets<ColorMaterial>,
    &'a mut Assets<GradientMaterial>,
    &'a mut Assets<BitmapMaterial>,
);

impl<'a> RenderContext<'a> {
    #[allow(clippy::too_many_arguments)]
    fn new(
        transform_stack: &'a mut TransformStack,
        scale: Vec3,
        cache_draws: &'a mut Vec<ImageCacheDraw>,
        commands: Vec<ShapeCommand>,
        shape_mesh_materials: &'a mut HashMap<CharacterId, Vec<(ShapeMaterialType, Handle<Mesh>)>>,
        meshes: &'a mut Assets<Mesh>,
        images: &'a mut Assets<Image>,
        gradients: &'a mut Assets<GradientMaterial>,
        bitmaps: &'a mut Assets<BitmapMaterial>,
        morph_shape_cache: &'a mut HashMap<CharacterId, fnv::FnvHashMap<u16, Frame>>,
        image_caches: &'a mut HashMap<CharacterId, ImageCache>,
    ) -> Self {
        Self {
            transform_stack,
            scale,
            cache_draws,
            commands,
            shape_mesh_materials,
            meshes,
            images,
            gradients,
            bitmaps,
            morph_shape_cache,
            image_caches,
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

/// 标记Mesh2d实体为ShapeMesh
#[derive(Component)]
pub struct ShapeMesh;

/// 为 Flash 动画添加完成事件
#[derive(Event, Clone, Deref)]
pub struct FlashCompleteEvent {
    /// 当前播放的动画名
    pub animation_name: Option<String>,
}

/// 为 Flash 动画添加帧事件
#[derive(Event, Clone, Deref, DerefMut)]
pub struct FlashFrameEvent(String);
impl FlashFrameEvent {
    pub fn name(&self) -> &str {
        self.0.as_str()
    }
}

/// 为Player实体添加Root MovieClip 组件
fn prepare_root_clip(
    mut commands: Commands,
    mut player: Query<(Entity, &mut FlashPlayer, &Flash)>,
    swf_res: Res<Assets<Swf>>,
    mut asset_event: EventReader<AssetEvent<Swf>>,
) {
    for event in asset_event.read() {
        if let AssetEvent::LoadedWithDependencies { id } = event
            && let Some((entity, mut player, _)) =
                player.iter_mut().find(|(_, _, flash)| flash.id() == *id)
        {
            let Some(swf) = swf_res.get(*id) else {
                continue;
            };
            let mut root = MovieClip::new(swf.swf_movie.clone());
            player.play_target_animation(swf, &mut root);
            commands.entity(entity).insert(root);
        }
    }
}

/// 推进Flash动画
#[allow(clippy::too_many_arguments)]
fn advance_animation(
    time: Res<Time>,
    filter_texture_mesh: Res<FilterTextureMesh>,
    mut swf_res: ResMut<Assets<Swf>>,
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
    mut shape_meshes: Query<&mut Transform, Or<(With<ShapeMesh>, With<OffscreenMesh>)>>,
    mut offscreen_textures: Query<&mut OffscreenTexture>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut color_materials: ResMut<Assets<ColorMaterial>>,
    mut gradient_materials: ResMut<Assets<GradientMaterial>>,
    mut bitmap_materials: ResMut<Assets<BitmapMaterial>>,
    mut display_object_entity_caches: Local<EntityHashMap<DisplayObjectCache>>,
) {
    let mut current_live_player = vec![];
    if timer.tick(time.delta()).just_finished() {
        // 1. 将动画的每一帧将离屏渲染实体列为不活跃
        offscreen_textures
            .iter_mut()
            .for_each(|mut offscreen_texture| {
                offscreen_texture.is_active = false;
            });
        // 2. 更新动画
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

            let Some(swf) = swf_res.get_mut(swf.id()) else {
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

            // 触发帧事件
            if let Some(event) = swf.frame_events().get(&root.current_frame()) {
                commands
                    .entity(entity)
                    .trigger(FlashFrameEvent(event.as_ref().into()));
            }

            let global_scale = global_transform.scale();

            let display_object_cache = display_object_entity_caches.entry(entity).or_default();
            let morph_shape_cache: &mut _ = &mut display_object_cache.morph_shape_frame_cache;
            let image_cache = &mut display_object_cache.image_cache;

            // 处理DisplayList
            let mut cache_draws = vec![];
            let mut transform_stack = TransformStack::default();
            let mut context = RenderContext::new(
                &mut transform_stack,
                global_scale,
                &mut cache_draws,
                Vec::new(),
                &mut swf.shape_mesh_materials,
                meshes.as_mut(),
                images.as_mut(),
                gradient_materials.as_mut(),
                bitmap_materials.as_mut(),
                morph_shape_cache,
                image_cache,
            );

            process_display_list(
                root.render_list_mut(),
                &mut context,
                swf::BlendMode::Normal,
                String::from("0"),
            );

            let shape_commands = context.commands;
            let mut current_live_shape_entity = vec![];
            spawn_or_update_shape(
                &mut commands,
                entity,
                filter_texture_mesh.as_ref(),
                (
                    color_materials.as_mut(),
                    gradient_materials.as_mut(),
                    bitmap_materials.as_mut(),
                ),
                &mut shape_meshes,
                cache_draws,
                shape_commands,
                &swf.shape_mesh_materials,
                &mut current_live_shape_entity,
                &mut offscreen_textures,
                display_object_cache,
                global_scale,
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
        display_object_entity_caches.retain(|entity, _| current_live_player.contains(entity));
    }
}

/// 缓存信息
pub struct CacheInfo {
    image_info: ImageCacheInfo,
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
        let bounds =
            display_object.render_bounds_with_transform(&base_transform.matrix, false, context);

        let mut filters = display_object.filters();
        let swf_version = display_object.swf_version();
        filters.retain(|f| !f.impotent());

        display_object.recheck_cache(id, context.image_caches);
        if let Some(cache) = context.image_caches.get_mut(&id) {
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
                let actual_width = (filter_rect.width() as f32 * scale.x) as u16;
                let actual_height = (filter_rect.height() as f32 * scale.y) as u16;
                if cache.is_dirty(&base_transform.matrix, width, height) {
                    cache.update(
                        context.images,
                        &base_transform.matrix,
                        width,
                        height,
                        actual_width,
                        actual_height,
                        swf_version,
                        draw_offset,
                    );
                    cache_info = cache.image_info().map(|image_info| CacheInfo {
                        image_info,
                        dirty: true,
                        base_transform,
                        bounds,
                        draw_offset,
                        filters,
                    });
                } else {
                    cache_info = cache.image_info().map(|image_info| CacheInfo {
                        image_info,
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
                let mut offscreen_context = RenderContext::new(
                    &mut transform_stack,
                    context.scale,
                    context.cache_draws,
                    Vec::new(),
                    context.shape_mesh_materials,
                    context.meshes,
                    context.images,
                    context.gradients,
                    context.bitmaps,
                    context.morph_shape_cache,
                    context.image_caches,
                );
                render_display_object(
                    display_object,
                    &mut offscreen_context,
                    blend_mode,
                    shape_depth_layer.clone(),
                );
                // 将offscreen_context需要缓存绘制的render_shapes合并到context.cache_draw.render_shapes
                offscreen_context.cache_draws.push(ImageCacheDraw {
                    layer: shape_depth_layer.clone(),
                    id,
                    handle: cache_info.image_info.handle(),
                    clear_color: Color::NONE,
                    commands: offscreen_context.commands,
                    filters: cache_info.filters,
                    dirty: cache_info.dirty,
                    size: cache_info.image_info.size(),
                });
            }
            // 引用绘制出来的中间纹理，按照位图材质的方式绘制到view
            let matrix = context.transform_stack.transform().matrix;
            let scale = context.scale;
            let bitmap_material = BitmapMaterial {
                texture: cache_info.image_info.handle(),
                texture_transform: Mat4::IDENTITY,
                transform: SwfTransform {
                    matrix: Matrix {
                        a: cache_info.image_info.size().x as f32 / scale.x,
                        d: cache_info.image_info.size().y as f32 / scale.y,
                        tx: matrix.tx + offset_x,
                        ty: matrix.ty + offset_y,
                        ..Default::default()
                    },
                    color_transform: cache_info.base_transform.color_transform,
                }
                .into(),
                blend_key: BlendMaterialKey::from(BlendMode::from(blend_mode)),
            };
            context.commands.push(ShapeCommand::RenderBitmap {
                bitmap_material,
                id,
                shape_depth_layer,
                size: cache_info.image_info.size().as_vec2(),
            });
        } else {
            // 绘制 background，大概率不重要
            // TODO:
            // 绘制自己
            render_display_object(display_object, context, blend_mode, shape_depth_layer);
        }

        // TODO:处理复杂混合模式

        context.transform_stack.pop();
    }
}

fn render_display_object(
    display_object: &mut DisplayObject,
    context: &mut RenderContext<'_>,
    blend_mode: swf::BlendMode,
    shape_depth_layer: String,
) {
    match display_object {
        DisplayObject::MovieClip(movie_clip) => {
            process_display_list(
                movie_clip.render_list_mut(),
                context,
                blend_mode,
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

#[allow(clippy::too_many_arguments)]
fn spawn_or_update_shape(
    commands: &mut Commands,
    entity: Entity,
    filter_texture_mesh: &FilterTextureMesh,
    (color_materials, gradient_materials, bitmap_materials): ShapeMaterialAssets<'_>,
    shape_meshes: &mut Query<&mut Transform, Or<(With<ShapeMesh>, With<OffscreenMesh>)>>,
    cache_draw: Vec<ImageCacheDraw>,
    shape_commands: Vec<ShapeCommand>,
    shape_mesh_materials: &HashMap<CharacterId, Vec<(ShapeMaterialType, Handle<Mesh>)>>,
    current_live_shape_entity: &mut Vec<Entity>,
    offscreen_textures: &mut Query<&mut OffscreenTexture>,
    display_object_cache: &mut DisplayObjectCache,
    scale: Vec3,
) {
    let (layer_shape_material_cache, layer_offscreen_shape_draw_cache, layer_offscreen_cache) = (
        &mut display_object_cache.layer_shape_material_cache,
        &mut display_object_cache.layer_offscreen_shape_draw_cache,
        &mut display_object_cache.layer_offscreen_cache,
    );

    // 当前根据shape_commands 生成的shape layer 用于记录是否多次引用了同一个shape, 避免重复生成
    let mut current_live_shape_depth_layers: HashSet<String> = HashSet::new();
    // 1. 处理需要绘制中间纹理的Shape
    let mut order = isize::MIN;
    for cache_draw in cache_draw {
        // 实现方案：
        // 为每一份 离屏纹理生成一个ViewTarget 并渲染到对应的通过自定义的ViewTarget中
        if cache_draw.dirty {
            let current_frame_shape_mesh_draws = process_offscreen_draw_commands(
                &cache_draw.commands,
                filter_texture_mesh,
                (color_materials, gradient_materials, bitmap_materials),
                shape_mesh_materials,
                layer_offscreen_shape_draw_cache,
                &mut current_live_shape_depth_layers,
            );
            if let Some(entity) = layer_offscreen_cache.get(&cache_draw.layer) {
                let Ok(mut offscreen_texture) = offscreen_textures.get_mut(*entity) else {
                    continue;
                };
                offscreen_texture.is_active = true;
                offscreen_texture.target = cache_draw.handle.into();
                offscreen_texture.size = cache_draw.size;
                offscreen_texture.scale = scale;
                offscreen_texture.filters = cache_draw.filters;
                commands
                    .entity(*entity)
                    .insert(OffscreenDrawCommands(current_frame_shape_mesh_draws));
            } else {
                order += 1;
                let entity = commands
                    .spawn((
                        OffscreenTexture {
                            target: cache_draw.handle.into(),
                            is_active: true,
                            size: cache_draw.size,
                            clear_color: cache_draw.clear_color,
                            order,
                            filters: cache_draw.filters,
                            scale,
                        },
                        OffscreenDrawCommands(current_frame_shape_mesh_draws),
                    ))
                    .id();
                layer_offscreen_cache.insert(cache_draw.layer, entity);
            }
        }
    }

    // 2. 处理不需要缓存的Shape
    // 记录当前帧Shape Command 中使用到的Shape层级用于复用实体。
    record_current_live_layer(&shape_commands, &mut current_live_shape_depth_layers);
    let mut commands = commands.entity(entity);
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
                let Some(shape_mesh_materials) = shape_mesh_materials.get(id) else {
                    continue;
                };
                if let Some(shape_material_handle_cache) = find_cached_shape_material(
                    id,
                    shape_depth_layer,
                    layer_shape_material_cache,
                    &mut current_live_shape_depth_layers,
                ) {
                    for (index, (entity, handle)) in shape_material_handle_cache.iter().enumerate()
                    {
                        z_index += index as f32 * 0.001;
                        let Ok(mut transform) = shape_meshes.get_mut(*entity) else {
                            continue;
                        };
                        update_shape_material(
                            &mut transform,
                            handle,
                            color_materials,
                            gradient_materials,
                            bitmap_materials,
                            swf_transform,
                            blend_mode,
                            z_index,
                        );
                        current_live_shape_entity.push(*entity);
                    }
                    continue;
                }
                // 该Shape没有生成过，需要生成
                let shape_material_handle_cache = layer_shape_material_cache
                    .entry(shape_depth_layer.to_owned())
                    .or_default();
                for (material_type, mesh) in shape_mesh_materials {
                    let mesh = mesh.clone();
                    match material_type {
                        ShapeMaterialType::Color(color) => {
                            spawn_shape_mesh(
                                &mut commands,
                                color_materials,
                                mesh,
                                *color,
                                swf_transform,
                                blend_mode,
                                z_index,
                                current_live_shape_entity,
                                |entity, handle| {
                                    shape_material_handle_cache
                                        .push((entity, MaterialType::Color(handle)));
                                },
                            );
                        }
                        ShapeMaterialType::Gradient(gradient) => {
                            spawn_shape_mesh(
                                &mut commands,
                                gradient_materials,
                                mesh,
                                gradient.clone(),
                                swf_transform,
                                blend_mode,
                                z_index,
                                current_live_shape_entity,
                                |entity, handle| {
                                    shape_material_handle_cache
                                        .push((entity, MaterialType::Gradient(handle)));
                                },
                            );
                        }
                        ShapeMaterialType::Bitmap(bitmap) => {
                            spawn_shape_mesh(
                                &mut commands,
                                bitmap_materials,
                                mesh,
                                bitmap.clone(),
                                swf_transform,
                                blend_mode,
                                z_index,
                                current_live_shape_entity,
                                |entity, handle| {
                                    shape_material_handle_cache
                                        .push((entity, MaterialType::Bitmap(handle)));
                                },
                            );
                        }
                    }
                }
            }
            ShapeCommand::RenderBitmap {
                bitmap_material,
                shape_depth_layer,
                size,
                ..
            } => {
                let raw_size = *size / scale.xy();
                spawn_or_update_bitmap(
                    layer_shape_material_cache,
                    shape_depth_layer,
                    shape_meshes,
                    bitmap_materials,
                    bitmap_material,
                    raw_size,
                    z_index,
                    current_live_shape_entity,
                    |bitmap_material_handle,
                     shape_material_handle_cache,
                     current_live_shape_entity| {
                        commands.with_children(|parent| {
                            let entity = parent
                                .spawn((
                                    Mesh2d(filter_texture_mesh.0.clone()),
                                    MeshMaterial2d(bitmap_material_handle.clone()),
                                    Transform::from_translation(Vec3::Z * z_index),
                                    ShapeMesh,
                                    NoFrustumCulling,
                                ))
                                .id();
                            shape_material_handle_cache
                                .push((entity, MaterialType::Bitmap(bitmap_material_handle)));
                            current_live_shape_entity.push(entity);
                        });
                    },
                );
            }
        }
    }

    fn update_shape_material(
        transform: &mut Transform,
        handle: &MaterialType,
        color_materials: &mut Assets<ColorMaterial>,
        gradient_materials: &mut Assets<GradientMaterial>,
        bitmap_materials: &mut Assets<BitmapMaterial>,
        swf_transform: &SwfTransform,
        blend_mode: &BlendMode,
        z_index: f32,
    ) {
        transform.translation.z = z_index;
        match handle {
            MaterialType::Color(color) => {
                update_material(color, color_materials, swf_transform, blend_mode);
            }
            MaterialType::Gradient(gradient) => {
                update_material(gradient, gradient_materials, swf_transform, blend_mode);
            }
            MaterialType::Bitmap(bitmap) => {
                update_material(bitmap, bitmap_materials, swf_transform, blend_mode);
            }
        }
    }

    fn spawn_or_update_bitmap(
        shape_material_cache: &mut HashMap<String, Vec<(Entity, MaterialType)>>,
        shape_depth_layer: &String,
        shape_meshes: &mut Query<&mut Transform, Or<(With<ShapeMesh>, With<OffscreenMesh>)>>,
        bitmap_materials: &mut Assets<BitmapMaterial>,
        bitmap_material: &BitmapMaterial,
        size: Vec2,
        z_index: f32,
        current_live_shape_entity: &mut Vec<Entity>,
        func: impl FnOnce(Handle<BitmapMaterial>, &mut Vec<(Entity, MaterialType)>, &mut Vec<Entity>),
    ) {
        if let Some(shape_material_handle_cache) = shape_material_cache.get(shape_depth_layer)
            && let Some((entity, bitmap_material_handle)) = shape_material_handle_cache.first()
            && let MaterialType::Bitmap(bitmap_material_handle) = bitmap_material_handle
        {
            let Ok(mut transform) = shape_meshes.get_mut(*entity) else {
                return;
            };
            transform.translation.z = z_index;
            let Some(entity_bitmap_material) = bitmap_materials.get_mut(bitmap_material_handle)
            else {
                return;
            };
            entity_bitmap_material.transform = bitmap_material.transform;
            entity_bitmap_material.transform.world_transform.x_axis.x = size.x;
            entity_bitmap_material.transform.world_transform.y_axis.y = size.y;
            entity_bitmap_material.blend_key = bitmap_material.blend_key;
            entity_bitmap_material.texture = bitmap_material.texture.clone();
            current_live_shape_entity.push(*entity);
        } else {
            // 该Shape没有生成过，需要生成
            let shape_material_handle_cache = shape_material_cache
                .entry(shape_depth_layer.to_owned())
                .or_default();
            let bitmap_material_handle = bitmap_materials.add(bitmap_material.clone());
            func(
                bitmap_material_handle.clone(),
                shape_material_handle_cache,
                current_live_shape_entity,
            );
        }
    }
}

#[inline]
#[allow(clippy::too_many_arguments)]
fn spawn_shape_mesh<T: SwfMaterial>(
    commands: &mut EntityCommands,
    materials: &mut Assets<T>,
    mesh: Handle<Mesh>,
    material: T,
    swf_transform: &SwfTransform,
    blend_mode: &BlendMode,
    z_index: f32,
    current_live_shape_entity: &mut Vec<Entity>,
    func: impl FnOnce(Entity, Handle<T>),
) {
    // let Some(material) = bitmap_materials.get_mut(bitmap.id()) else {
    //     continue;
    // };
    // material.transform = (*transform).into();

    let mut material = material;
    material.update_swf_material((*swf_transform).into());
    material.set_blend_key((*blend_mode).into());
    let handle = materials.add(material);
    commands.with_children(|parent| {
        let entity = parent
            .spawn((
                Mesh2d(mesh),
                MeshMaterial2d(handle.clone()),
                Transform::from_translation(Vec3::Z * z_index),
                ShapeMesh,
                // 由于Flash顶点特殊性不应用剔除
                NoFrustumCulling,
            ))
            .id();
        current_live_shape_entity.push(entity);
        func(entity, handle);
    });
}

#[inline]
fn handle_offscreen_draw<T: SwfMaterial>(
    materials: &mut Assets<T>,
    mut material: T,
    swf_transform: &SwfTransform,
    blend_mode: &BlendMode,
    mut func: impl FnMut(Handle<T>),
) {
    material.update_swf_material((*swf_transform).into());
    material.set_blend_key((*blend_mode).into());
    let handle = materials.add(material);
    func(handle);
}

#[inline]
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
fn find_cached_shape_material<'a, T>(
    id: &CharacterId,
    shape_depth_layer: &String,
    layer_cache: &'a HashMap<String, T>,
    current_live_shape_depth_layers: &mut HashSet<String>,
) -> Option<&'a T> {
    if let Some(cache) = layer_cache.get(shape_depth_layer) {
        Some(cache)
    } else {
        // 从shape_material_cache中获取key值最后一个“_"后的字符匹配id
        if let Some((k, cache)) = layer_cache
            .iter()
            .filter(|(k, _)| !current_live_shape_depth_layers.contains(k.as_str()))
            .find(|(key, _)| {
                key.split("_")
                    .last()
                    .map(|v| v == id.to_string())
                    .unwrap_or(false)
            })
        {
            current_live_shape_depth_layers.insert(k.to_owned());
            Some(cache)
        } else {
            None
        }
    }
}

fn record_current_live_layer(
    shape_commands: &[ShapeCommand],
    current_live_shape_depth_layers: &mut HashSet<String>,
) {
    shape_commands.iter().for_each(|shape_command| {
        if let ShapeCommand::RenderShape {
            shape_depth_layer, ..
        } = shape_command
        {
            current_live_shape_depth_layers.insert(shape_depth_layer.to_owned());
        }
    });
}

fn process_offscreen_draw_commands(
    commands: &[ShapeCommand],
    filter_texture_mesh: &FilterTextureMesh,
    (color_materials, gradient_materials, bitmap_materials): ShapeMaterialAssets<'_>,
    shape_mesh_materials: &HashMap<CharacterId, Vec<(ShapeMaterialType, Handle<Mesh>)>>,
    layer_offscreen_shape_draw_cache: &mut HashMap<String, Vec<ShapeMeshDraw>>,
    current_live_shape_depth_layers: &mut HashSet<String>,
) -> Vec<ShapeMeshDraw> {
    // 记录当前帧可以重复使用的层级
    record_current_live_layer(commands, current_live_shape_depth_layers);
    let mut current_frame_shape_mesh_draws = vec![];
    commands.iter().for_each(|command| match command {
        ShapeCommand::RenderShape {
            transform,
            id,
            shape_depth_layer,
            blend_mode,
        } => {
            if let Some(cache) = find_cached_shape_material(
                id,
                shape_depth_layer,
                layer_offscreen_shape_draw_cache,
                current_live_shape_depth_layers,
            ) {
                for shape_mesh_draw in cache.iter() {
                    match &shape_mesh_draw.material_type {
                        MaterialType::Color(color) => {
                            update_material(color, color_materials, transform, blend_mode);
                        }
                        MaterialType::Gradient(gradient) => {
                            update_material(gradient, gradient_materials, transform, blend_mode);
                        }
                        MaterialType::Bitmap(bitmap) => {
                            update_material(bitmap, bitmap_materials, transform, blend_mode);
                        }
                    }
                }
                current_frame_shape_mesh_draws.extend(cache.clone());
            } else {
                let Some(material_type_mesh_cache) = shape_mesh_materials.get(id) else {
                    return;
                };
                let shape_mesh_draws = layer_offscreen_shape_draw_cache
                    .entry(shape_depth_layer.to_owned())
                    .or_default();
                for (material_type, mesh) in material_type_mesh_cache {
                    match material_type {
                        ShapeMaterialType::Color(color) => {
                            handle_offscreen_draw(
                                color_materials,
                                *color,
                                transform,
                                blend_mode,
                                |material| {
                                    shape_mesh_draws.push(ShapeMeshDraw {
                                        mesh: mesh.clone(),
                                        material_type: MaterialType::Color(material),
                                        blend: BlendMaterialKey::from(*blend_mode),
                                    });
                                },
                            );
                        }
                        ShapeMaterialType::Gradient(gradient_material) => {
                            handle_offscreen_draw(
                                gradient_materials,
                                gradient_material.clone(),
                                transform,
                                blend_mode,
                                |material| {
                                    shape_mesh_draws.push(ShapeMeshDraw {
                                        mesh: mesh.clone(),
                                        material_type: MaterialType::Gradient(material),
                                        blend: BlendMaterialKey::from(*blend_mode),
                                    });
                                },
                            );
                        }
                        ShapeMaterialType::Bitmap(bitmap_material) => {
                            handle_offscreen_draw(
                                bitmap_materials,
                                bitmap_material.clone(),
                                transform,
                                blend_mode,
                                |material| {
                                    shape_mesh_draws.push(ShapeMeshDraw {
                                        mesh: mesh.clone(),
                                        material_type: MaterialType::Bitmap(material),
                                        blend: BlendMaterialKey::from(*blend_mode),
                                    });
                                },
                            );
                        }
                    }
                }
                current_frame_shape_mesh_draws.extend(shape_mesh_draws.clone());
            };
        }
        ShapeCommand::RenderBitmap {
            bitmap_material,
            shape_depth_layer,
            ..
        } => {
            if let Some(cache) = layer_offscreen_shape_draw_cache.get(shape_depth_layer)
                && let Some(shape_mesh_draw) = cache.first()
                && let MaterialType::Bitmap(handle) = &shape_mesh_draw.material_type
            {
                let Some(bitmap) = bitmap_materials.get_mut(handle) else {
                    return;
                };
                bitmap.transform = bitmap_material.transform;
                bitmap.blend_key = bitmap_material.blend_key;
                bitmap.texture = bitmap_material.texture.clone();
                current_frame_shape_mesh_draws.extend(cache.clone());
            } else {
                let cache = layer_offscreen_shape_draw_cache
                    .entry(shape_depth_layer.to_owned())
                    .or_default();
                let handle = bitmap_materials.add(bitmap_material.clone());
                cache.push(ShapeMeshDraw {
                    mesh: filter_texture_mesh.0.clone(),
                    material_type: MaterialType::Bitmap(handle.clone()),
                    blend: BlendMaterialKey::NORMAL,
                });
                current_frame_shape_mesh_draws.extend(cache.clone());
            }
        }
    });
    current_frame_shape_mesh_draws
}
