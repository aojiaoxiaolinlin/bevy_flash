pub mod assets;
mod commands;
pub mod player;
mod render;
pub mod swf_runtime;

use std::collections::btree_map::ValuesMut;

use crate::{
    assets::{ShapeMaterialType, Swf, SwfLoader},
    commands::{MaterialType, OffscreenDrawCommands, ShapeCommand, ShapeMeshDraw},
    player::{Flash, FlashPlayer, McRoot},
    render::{
        FilterTextureMesh, FlashRenderPlugin,
        blend_pipeline::BlendMode,
        material::{
            BitmapMaterial, BlendMaterialKey, ColorMaterial, GradientMaterial, SwfMaterial,
        },
        offscreen_texture::OffscreenTexture,
    },
    swf_runtime::{
        display_object::{DisplayObject, ImageCache, ImageCacheInfo, TDisplayObject},
        filter::Filter,
        matrix::Matrix,
        morph_shape::Frame,
        movie_clip::MovieClip,
        transform::{Transform as SwfTransform, TransformStack},
    },
};

use bevy::{
    app::{App, Plugin, PostUpdate},
    asset::{AssetApp, Assets, Handle},
    camera::visibility::{NoFrustumCulling, Visibility},
    color::Color,
    ecs::{
        component::Component,
        entity::{Entity, EntityHashMap},
        event::EntityEvent,
        hierarchy::ChildOf,
        query::{With, Without},
        resource::Resource,
        schedule::IntoScheduleConfigs,
        system::{Commands, EntityCommands, Local, Query, Res, ResMut},
    },
    image::Image,
    log::warn_once,
    math::{IVec2, Mat4, UVec2, Vec2, Vec3, Vec3Swizzles},
    mesh::{Mesh, Mesh2d},
    platform::collections::{HashMap, HashSet},
    prelude::{Deref, DerefMut},
    sprite_render::MeshMaterial2d,
    time::{Time, Timer, TimerMode},
    transform::components::{GlobalTransform, Transform},
};

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
            .init_resource::<FlashPlayerTimer>()
            .add_systems(PostUpdate, (prepare_root_clip, advance_animation).chain());
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
#[derive(EntityEvent, Clone)]
pub struct FlashCompleteEvent {
    /// 实体
    entity: Entity,
    /// 当前播放的动画名
    name: Option<String>,
}

impl FlashCompleteEvent {
    /// 实体
    pub fn entity(&self) -> Entity {
        self.entity
    }
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

/// 为 Flash 动画添加帧事件
#[derive(EntityEvent, Clone)]
pub struct FlashFrameEvent {
    /// 实体
    entity: Entity,
    /// 帧事件名
    name: String,
}
impl FlashFrameEvent {
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// 为Player实体添加Root MovieClip 组件
fn prepare_root_clip(
    mut commands: Commands,
    mut player: Query<(Entity, &mut FlashPlayer, &Flash), Without<McRoot>>,
    swf_res: Res<Assets<Swf>>,
) {
    for (entity, mut player, flash) in player.iter_mut() {
        let Some(swf) = swf_res.get(flash.id()) else {
            continue;
        };
        let mut root = McRoot(MovieClip::new(swf.swf_movie.clone()));
        player.play_target_animation(swf, &mut root);
        commands.entity(entity).insert(root);
    }
}

/// 将所有离屏渲染实体标记为不活跃
fn mark_offscreen_textures_inactive(offscreen_textures: &mut Query<&mut OffscreenTexture>) {
    offscreen_textures
        .iter_mut()
        .for_each(|mut offscreen_texture| {
            offscreen_texture.is_active = false;
        });
}

/// 处理动画完成事件
fn handle_animation_complete(
    commands: &mut Commands,
    entity: Entity,
    player: &mut FlashPlayer,
) -> bool {
    if !player.is_looping() && player.is_completed() {
        // 触发完成事件
        if !player.completed() {
            commands.trigger(FlashCompleteEvent {
                entity,
                name: player.current_animation().map(|s| s.into()),
            });
            player.set_completed(true);
        }
        return true;
    }
    false
}

/// 处理循环播放逻辑
fn handle_animation_loop(player: &mut FlashPlayer, root: &mut McRoot, swf: &Swf) {
    if player.is_looping() && player.is_completed() {
        // 循环播放，跳回当前动画第一帧
        player.play_target_animation(swf, root);
    }
}

/// 更新动画帧并触发帧事件
fn update_animation_frame(
    commands: &mut Commands,
    entity: Entity,
    player: &mut FlashPlayer,
    root: &mut MovieClip,
    swf: &Swf,
) {
    let characters = &swf.characters();
    // 进入一帧
    root.enter_frame(characters);
    player.incr_frame();

    // 触发帧事件
    if let Some(event) = swf.frame_events().get(&root.current_frame()) {
        commands.trigger(FlashFrameEvent {
            entity,
            name: event.as_ref().into(),
        });
    }
}

/// 创建渲染上下文
fn create_render_context<'a>(
    transform_stack: &'a mut TransformStack,
    cache_draws: &'a mut Vec<ImageCacheDraw>,
    swf: &'a mut Swf,
    meshes: &'a mut Assets<Mesh>,
    images: &'a mut Assets<Image>,
    gradient_materials: &'a mut Assets<GradientMaterial>,
    bitmap_materials: &'a mut Assets<BitmapMaterial>,
    morph_shape_cache: &'a mut HashMap<CharacterId, fnv::FnvHashMap<u16, Frame>>,
    image_cache: &'a mut HashMap<CharacterId, ImageCache>,
    global_scale: Vec3,
) -> RenderContext<'a> {
    RenderContext::new(
        transform_stack,
        global_scale,
        cache_draws,
        Vec::new(),
        &mut swf.shape_mesh_materials,
        meshes,
        images,
        gradient_materials,
        bitmap_materials,
        morph_shape_cache,
        image_cache,
    )
}

/// 更新实体可见性
fn update_entity_visibility(
    entity: Entity,
    shape_meshes: &mut Query<(&ChildOf, &mut Transform, &mut Visibility), With<ShapeMesh>>,
) {
    shape_meshes
        .iter_mut()
        .filter(|(child_of, _, _)| child_of.parent() == entity)
        .for_each(|(_, _, mut visibility)| {
            *visibility = Visibility::Hidden;
        });
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
        &mut McRoot,
        &Flash,
        &GlobalTransform,
    )>,
    mut shape_meshes: Query<(&ChildOf, &mut Transform, &mut Visibility), With<ShapeMesh>>,
    mut offscreen_textures: Query<&mut OffscreenTexture>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut color_materials: ResMut<Assets<ColorMaterial>>,
    mut gradient_materials: ResMut<Assets<GradientMaterial>>,
    mut bitmap_materials: ResMut<Assets<BitmapMaterial>>,
    mut display_object_entity_caches: Local<EntityHashMap<DisplayObjectCache>>,
) {
    let mut current_live_player = vec![];
    // 1. 将动画的每一帧将离屏渲染实体列为不活跃
    mark_offscreen_textures_inactive(&mut offscreen_textures);
    if timer.tick(time.delta()).just_finished() {
        // 2. 更新动画帧
        for (entity, mut player, mut root, swf, global_transform) in player.iter_mut() {
            current_live_player.push(entity);

            // 处理动画完成事件
            if handle_animation_complete(&mut commands, entity, &mut player) {
                continue;
            }

            // 将所有形状实体标记为不活跃
            update_entity_visibility(entity, &mut shape_meshes);
            let Some(swf) = swf_res.get_mut(swf.id()) else {
                continue;
            };

            // 处理循环播放逻辑
            handle_animation_loop(&mut player, &mut root, swf);

            // 更新动画帧并触发帧事件
            update_animation_frame(&mut commands, entity, &mut player, &mut root, swf);

            let global_scale = global_transform.scale();

            let display_object_cache = display_object_entity_caches.entry(entity).or_default();
            let morph_shape_cache: &mut _ = &mut display_object_cache.morph_shape_frame_cache;
            let image_cache = &mut display_object_cache.image_cache;

            // 处理DisplayList
            let mut cache_draws = vec![];
            let mut transform_stack = TransformStack::default();

            // 创建渲染上下文
            let mut context = create_render_context(
                &mut transform_stack,
                &mut cache_draws,
                swf,
                meshes.as_mut(),
                images.as_mut(),
                gradient_materials.as_mut(),
                bitmap_materials.as_mut(),
                morph_shape_cache,
                image_cache,
                global_scale,
            );
            process_display_list(
                root.render_list_mut(),
                &mut context,
                swf::BlendMode::Normal,
                String::from("0"),
            );

            let shape_commands = context.commands;
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
                &mut offscreen_textures,
                display_object_cache,
                global_scale,
            );
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

/// 处理显示对象列表，遍历并渲染每个显示对象
fn process_display_list(
    display_list: ValuesMut<'_, u16, DisplayObject>,
    context: &mut RenderContext<'_>,
    blend_mode: swf::BlendMode,
    shape_depth_layer: String,
) {
    for display_object in display_list {
        let id = display_object.id();
        let shape_depth_layer = format!("{}_{}_{}", shape_depth_layer, display_object.depth(), id);

        // 保存当前变换状态
        context
            .transform_stack
            .push(display_object.base().transform());

        // 确定混合模式
        let blend_mode = determine_blend_mode(blend_mode, display_object);

        // 处理缓存和滤镜
        let cache_info = process_cache_and_filters(display_object, context, id);

        // 根据是否有缓存信息选择渲染方式
        if let Some(cache_info) = cache_info {
            render_with_cache(
                display_object,
                context,
                cache_info,
                blend_mode,
                id,
                &shape_depth_layer,
            );
        } else {
            // 直接渲染显示对象
            render_child(display_object, context, blend_mode, &shape_depth_layer);
        }

        // TODO:处理复杂混合模式

        // 恢复变换状态
        context.transform_stack.pop();
    }
}

/// 确定要使用的混合模式
fn determine_blend_mode(
    parent_blend_mode: swf::BlendMode,
    display_object: &DisplayObject,
) -> swf::BlendMode {
    if parent_blend_mode == swf::BlendMode::Normal {
        display_object.blend_mode()
    } else {
        parent_blend_mode
    }
}

/// 处理显示对象的缓存和滤镜，返回缓存信息
fn process_cache_and_filters(
    display_object: &mut DisplayObject,
    context: &mut RenderContext<'_>,
    id: CharacterId,
) -> Option<CacheInfo> {
    // 获取基本变换和边界
    let base_transform = context.transform_stack.transform();
    let bounds =
        display_object.render_bounds_with_transform(&base_transform.matrix, false, context);

    // 处理滤镜
    let mut filters = display_object.filters();
    let swf_version = display_object.swf_version();
    filters.retain(|f| !f.impotent());

    // 检查缓存
    display_object.recheck_cache(id, context.image_caches);

    // 如果没有缓存，直接返回None
    let cache = context.image_caches.get_mut(&id)?;

    // 计算尺寸
    let width = bounds.width().to_pixels().ceil().max(0.);
    let height = bounds.height().to_pixels().ceil().max(0.);

    // 检查尺寸是否在限制范围内
    if width > u16::MAX as f64 || height > u16::MAX as f64 {
        warn_once!("缓存大小超出限制，已清除缓存, {}, ({width} x {height}", id);
        cache.clear();
        return None;
    }

    let width = width as u16;
    let height = height as u16;

    // 计算滤镜矩形
    let filter_rect = calculate_filter_rect(width, height, &mut filters, context.scale);

    // 计算绘制偏移和实际尺寸
    let draw_offset = IVec2::new(filter_rect.x_min, filter_rect.y_min);
    let actual_width = (filter_rect.width() as f32 * context.scale.x) as u16;
    let actual_height = (filter_rect.height() as f32 * context.scale.y) as u16;

    // 更新缓存
    if cache.is_dirty(&base_transform.matrix, width, height) || display_object.cache_dirty() {
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

        cache.image_info().map(|image_info| CacheInfo {
            image_info,
            dirty: true,
            base_transform,
            bounds,
            draw_offset,
            filters,
        })
    } else {
        cache.image_info().map(|image_info| CacheInfo {
            image_info,
            dirty: false,
            base_transform,
            bounds,
            draw_offset,
            filters,
        })
    }
}

/// 计算滤镜矩形
fn calculate_filter_rect(
    width: u16,
    height: u16,
    filters: &mut Vec<Filter>,
    scale: Vec3,
) -> Rectangle<i32> {
    // 初始化滤镜矩形
    let mut filter_rect = Rectangle {
        x_min: Twips::ZERO,
        y_min: Twips::ZERO,
        x_max: Twips::from_pixels_i32(width as i32),
        y_max: Twips::from_pixels_i32(height as i32),
    };

    // 应用滤镜缩放和计算目标矩形
    for filter in filters {
        filter.scale(scale.x, scale.y);
        filter_rect = filter.calculate_dest_rect(filter_rect);
    }

    // 转换为像素坐标的矩形
    Rectangle {
        x_min: filter_rect.x_min.to_pixels().floor() as i32,
        y_min: filter_rect.y_min.to_pixels().floor() as i32,
        x_max: filter_rect.x_max.to_pixels().ceil() as i32,
        y_max: filter_rect.y_max.to_pixels().ceil() as i32,
    }
}

/// 使用缓存渲染显示对象
fn render_with_cache(
    display_object: &mut DisplayObject,
    context: &mut RenderContext<'_>,
    cache_info: CacheInfo,
    blend_mode: swf::BlendMode,
    id: CharacterId,
    shape_depth_layer: &str,
) {
    // 计算偏移
    let offset_x = cache_info.bounds.x_min - cache_info.base_transform.matrix.tx
        + Twips::from_pixels_i32(cache_info.draw_offset.x);
    let offset_y = cache_info.bounds.y_min - cache_info.base_transform.matrix.ty
        + Twips::from_pixels_i32(cache_info.draw_offset.y);

    // 如果缓存是脏的，需要重新渲染到离屏纹理
    if cache_info.dirty {
        render_to_offscreen_texture(
            display_object,
            context,
            &cache_info,
            offset_x,
            offset_y,
            blend_mode,
            id,
            shape_depth_layer,
        );
    }

    // 将缓存的纹理作为位图渲染到主视图
    render_cached_texture_to_view(
        context,
        &cache_info,
        offset_x,
        offset_y,
        blend_mode,
        shape_depth_layer,
    );
}

/// 渲染显示对象到离屏纹理
fn render_to_offscreen_texture(
    display_object: &mut DisplayObject,
    context: &mut RenderContext<'_>,
    cache_info: &CacheInfo,
    offset_x: Twips,
    offset_y: Twips,
    blend_mode: swf::BlendMode,
    id: CharacterId,
    shape_depth_layer: &str,
) {
    // 创建新的变换栈
    let mut transform_stack = TransformStack::new();
    transform_stack.push(&SwfTransform {
        color_transform: Default::default(),
        matrix: Matrix {
            tx: -offset_x,
            ty: -offset_y,
            ..cache_info.base_transform.matrix
        },
    });

    // 创建离屏渲染上下文
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

    // 渲染显示对象到离屏上下文
    render_child(
        display_object,
        &mut offscreen_context,
        blend_mode,
        &shape_depth_layer,
    );

    // 将离屏上下文的绘制命令添加到缓存绘制列表
    offscreen_context.cache_draws.push(ImageCacheDraw {
        layer: shape_depth_layer.to_string(),
        id,
        handle: cache_info.image_info.handle(),
        clear_color: Color::NONE,
        commands: offscreen_context.commands,
        filters: cache_info.filters.clone(),
        dirty: true,
        size: cache_info.image_info.size(),
    });
}

/// 将缓存的纹理渲染到主视图
fn render_cached_texture_to_view(
    context: &mut RenderContext<'_>,
    cache_info: &CacheInfo,
    offset_x: Twips,
    offset_y: Twips,
    blend_mode: swf::BlendMode,
    shape_depth_layer: &str,
) {
    // 获取当前变换矩阵和缩放
    let matrix = context.transform_stack.transform().matrix;
    let scale = context.scale;

    // 创建位图材质
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

    // 添加渲染位图命令
    context.commands.push(ShapeCommand::RenderBitmap {
        bitmap_material,
        shape_depth_layer: shape_depth_layer.to_string(),
        size: cache_info.image_info.size().as_vec2(),
    });
}

fn render_child(
    child: &mut DisplayObject,
    context: &mut RenderContext<'_>,
    blend_mode: swf::BlendMode,
    shape_depth_layer: &str,
) {
    if child.clip_depth() > 0 && child.allow_as_mask() {
        warn_once!("Mass is not supported. TODO!");
        // 作为遮罩处理
        // 1. 标记为遮罩TODO:
        // 2. 渲染
        // 3. 标记遮罩活跃
    } else {
        render_display_object(child, context, blend_mode, shape_depth_layer.to_string());
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

/// 处理形状的生成和更新，包括离屏纹理和直接渲染的形状
///
/// 这个函数负责：
/// 1. 处理需要绘制到中间纹理的形状（离屏渲染）
/// 2. 处理直接渲染的形状（更新已有形状或创建新形状）
/// 3. 维护形状的缓存和复用
#[allow(clippy::too_many_arguments)]
fn spawn_or_update_shape(
    commands: &mut Commands,
    entity: Entity,
    filter_texture_mesh: &FilterTextureMesh,
    (color_materials, gradient_materials, bitmap_materials): ShapeMaterialAssets<'_>,
    shape_meshes: &mut Query<(&ChildOf, &mut Transform, &mut Visibility), With<ShapeMesh>>,
    cache_draw: Vec<ImageCacheDraw>,
    shape_commands: Vec<ShapeCommand>,
    shape_mesh_materials: &HashMap<CharacterId, Vec<(ShapeMaterialType, Handle<Mesh>)>>,
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

    // 1. 处理需要绘制中间纹理的Shape（离屏渲染）
    process_offscreen_textures(
        commands,
        &cache_draw,
        filter_texture_mesh,
        (color_materials, gradient_materials, bitmap_materials),
        shape_mesh_materials,
        layer_offscreen_shape_draw_cache,
        layer_offscreen_cache,
        &mut current_live_shape_depth_layers,
        offscreen_textures,
        scale,
    );

    // 2. 处理不需要缓存的Shape（直接渲染）
    process_direct_shapes(
        commands,
        entity,
        filter_texture_mesh,
        (color_materials, gradient_materials, bitmap_materials),
        shape_meshes,
        &shape_commands,
        shape_mesh_materials,
        layer_shape_material_cache,
        &mut current_live_shape_depth_layers,
        scale,
    );
}

/// 处理需要绘制到中间纹理的形状（离屏渲染）
#[allow(clippy::too_many_arguments)]
fn process_offscreen_textures(
    commands: &mut Commands,
    cache_draws: &[ImageCacheDraw],
    filter_texture_mesh: &FilterTextureMesh,
    (color_materials, gradient_materials, bitmap_materials): ShapeMaterialAssets<'_>,
    shape_mesh_materials: &HashMap<CharacterId, Vec<(ShapeMaterialType, Handle<Mesh>)>>,
    layer_offscreen_shape_draw_cache: &mut HashMap<String, Vec<ShapeMeshDraw>>,
    layer_offscreen_cache: &mut HashMap<String, Entity>,
    current_live_shape_depth_layers: &mut HashSet<String>,
    offscreen_textures: &mut Query<&mut OffscreenTexture>,
    scale: Vec3,
) {
    let mut order = isize::MIN;

    for cache_draw in cache_draws {
        // 只处理需要更新的纹理
        if !cache_draw.dirty {
            continue;
        }

        // 生成当前帧的形状网格绘制命令
        let current_frame_shape_mesh_draws = process_offscreen_draw_commands(
            &cache_draw.commands,
            filter_texture_mesh,
            (color_materials, gradient_materials, bitmap_materials),
            shape_mesh_materials,
            layer_offscreen_shape_draw_cache,
            current_live_shape_depth_layers,
        );

        // 更新或创建离屏纹理实体
        if let Some(entity) = layer_offscreen_cache.get(&cache_draw.layer) {
            update_existing_offscreen_texture(
                commands,
                *entity,
                offscreen_textures,
                cache_draw,
                current_frame_shape_mesh_draws,
                scale,
            );
        } else {
            create_new_offscreen_texture(
                commands,
                layer_offscreen_cache,
                &cache_draw.layer,
                cache_draw,
                current_frame_shape_mesh_draws,
                &mut order,
                scale,
            );
        }
    }
}

/// 更新已存在的离屏纹理实体
fn update_existing_offscreen_texture(
    commands: &mut Commands,
    entity: Entity,
    offscreen_textures: &mut Query<&mut OffscreenTexture>,
    cache_draw: &ImageCacheDraw,
    current_frame_shape_mesh_draws: Vec<ShapeMeshDraw>,
    scale: Vec3,
) {
    let Ok(mut offscreen_texture) = offscreen_textures.get_mut(entity) else {
        return;
    };

    // 更新离屏纹理属性
    offscreen_texture.is_active = true;
    offscreen_texture.target = cache_draw.handle.clone().into();
    offscreen_texture.size = cache_draw.size;
    offscreen_texture.scale = scale;
    offscreen_texture.filters = cache_draw.filters.clone();

    // 更新绘制命令
    commands
        .entity(entity)
        .insert(OffscreenDrawCommands(current_frame_shape_mesh_draws));
}

/// 创建新的离屏纹理实体
fn create_new_offscreen_texture(
    commands: &mut Commands,
    layer_offscreen_cache: &mut HashMap<String, Entity>,
    layer_key: &str,
    cache_draw: &ImageCacheDraw,
    current_frame_shape_mesh_draws: Vec<ShapeMeshDraw>,
    order: &mut isize,
    scale: Vec3,
) {
    *order += 1;

    // 创建新的离屏纹理实体
    let entity = commands
        .spawn((
            OffscreenTexture {
                target: cache_draw.handle.clone().into(),
                is_active: true,
                size: cache_draw.size,
                clear_color: cache_draw.clear_color,
                order: *order,
                filters: cache_draw.filters.clone(),
                scale,
            },
            OffscreenDrawCommands(current_frame_shape_mesh_draws),
        ))
        .id();

    // 缓存新创建的实体
    layer_offscreen_cache.insert(layer_key.to_owned(), entity);
}

/// 处理直接渲染的形状（不需要离屏渲染）
#[allow(clippy::too_many_arguments)]
fn process_direct_shapes(
    commands: &mut Commands,
    entity: Entity,
    filter_texture_mesh: &FilterTextureMesh,
    (color_materials, gradient_materials, bitmap_materials): ShapeMaterialAssets<'_>,
    shape_meshes: &mut Query<(&ChildOf, &mut Transform, &mut Visibility), With<ShapeMesh>>,
    shape_commands: &[ShapeCommand],
    shape_mesh_materials: &HashMap<CharacterId, Vec<(ShapeMaterialType, Handle<Mesh>)>>,
    layer_shape_material_cache: &mut HashMap<String, Vec<(Entity, MaterialType)>>,
    current_live_shape_depth_layers: &mut HashSet<String>,
    scale: Vec3,
) {
    // 记录当前帧Shape Command 中使用到的Shape层级用于复用实体
    record_current_live_layer(shape_commands, current_live_shape_depth_layers);

    let mut commands = commands.entity(entity);
    let mut z_index = 0.;

    // 处理每个形状命令
    for (index, shape_command) in shape_commands.iter().enumerate() {
        z_index += index as f32 * 0.001;

        match shape_command {
            ShapeCommand::RenderShape { .. } => {
                process_render_shape_command(
                    &mut commands,
                    shape_command,
                    shape_mesh_materials,
                    layer_shape_material_cache,
                    current_live_shape_depth_layers,
                    shape_meshes,
                    (color_materials, gradient_materials, bitmap_materials),
                    &mut z_index,
                );
            }
            ShapeCommand::RenderBitmap { .. } => {
                process_render_bitmap_command(
                    &mut commands,
                    shape_command,
                    layer_shape_material_cache,
                    shape_meshes,
                    bitmap_materials,
                    filter_texture_mesh,
                    z_index,
                    scale,
                );
            }
        }
    }
}

/// 处理RenderShape命令
#[allow(clippy::too_many_arguments)]
fn process_render_shape_command(
    commands: &mut EntityCommands,
    shape_command: &ShapeCommand,
    shape_mesh_materials: &HashMap<CharacterId, Vec<(ShapeMaterialType, Handle<Mesh>)>>,
    layer_shape_material_cache: &mut HashMap<String, Vec<(Entity, MaterialType)>>,
    current_live_shape_depth_layers: &mut HashSet<String>,
    shape_meshes: &mut Query<(&ChildOf, &mut Transform, &mut Visibility), With<ShapeMesh>>,
    materials: ShapeMaterialAssets<'_>,
    z_index: &mut f32,
) {
    let (color_materials, gradient_materials, bitmap_materials) = materials;

    if let ShapeCommand::RenderShape {
        transform: swf_transform,
        id,
        shape_depth_layer,
        blend_mode,
    } = shape_command
    {
        // 获取形状的网格材质
        let Some(shape_mesh_materials) = shape_mesh_materials.get(id) else {
            return;
        };

        // 尝试查找缓存的形状材质
        if let Some(shape_material_handle_cache) = find_cached_shape_material(
            id,
            shape_depth_layer,
            layer_shape_material_cache,
            current_live_shape_depth_layers,
        ) {
            // 更新已有形状
            update_cached_shapes(
                shape_material_handle_cache,
                shape_meshes,
                color_materials,
                gradient_materials,
                bitmap_materials,
                swf_transform,
                blend_mode,
                z_index,
            );
            return;
        }

        // 该Shape没有生成过，需要生成
        create_new_shapes(
            commands,
            shape_mesh_materials,
            shape_depth_layer,
            layer_shape_material_cache,
            color_materials,
            gradient_materials,
            bitmap_materials,
            swf_transform,
            blend_mode,
            *z_index,
        );
    }
}

/// 更新已缓存的形状
#[allow(clippy::too_many_arguments)]
fn update_cached_shapes(
    shape_material_handle_cache: &[(Entity, MaterialType)],
    shape_meshes: &mut Query<(&ChildOf, &mut Transform, &mut Visibility), With<ShapeMesh>>,
    color_materials: &mut Assets<ColorMaterial>,
    gradient_materials: &mut Assets<GradientMaterial>,
    bitmap_materials: &mut Assets<BitmapMaterial>,
    swf_transform: &SwfTransform,
    blend_mode: &BlendMode,
    z_index: &mut f32,
) {
    for (index, (entity, handle)) in shape_material_handle_cache.iter().enumerate() {
        *z_index += index as f32 * 0.001;

        let Ok((_, mut transform, mut visibility)) = shape_meshes.get_mut(*entity) else {
            continue;
        };
        transform.translation.z = *z_index;
        *visibility = Visibility::Inherited;
        update_shape_material(
            handle,
            color_materials,
            gradient_materials,
            bitmap_materials,
            swf_transform,
            blend_mode,
        );
    }
}

/// 创建新的形状
#[allow(clippy::too_many_arguments)]
fn create_new_shapes(
    commands: &mut EntityCommands,
    shape_mesh_materials: &[(ShapeMaterialType, Handle<Mesh>)],
    shape_depth_layer: &String,
    layer_shape_material_cache: &mut HashMap<String, Vec<(Entity, MaterialType)>>,
    color_materials: &mut Assets<ColorMaterial>,
    gradient_materials: &mut Assets<GradientMaterial>,
    bitmap_materials: &mut Assets<BitmapMaterial>,
    swf_transform: &SwfTransform,
    blend_mode: &BlendMode,
    z_index: f32,
) {
    // 获取或创建形状材质缓存
    let shape_material_handle_cache = layer_shape_material_cache
        .entry(shape_depth_layer.to_owned())
        .or_default();

    // 为每种材质类型创建形状
    for (material_type, mesh) in shape_mesh_materials {
        let mesh = mesh.clone();

        match material_type {
            ShapeMaterialType::Color(color) => {
                spawn_shape_mesh(
                    commands,
                    color_materials,
                    mesh,
                    *color,
                    swf_transform,
                    blend_mode,
                    z_index,
                    |entity, handle| {
                        shape_material_handle_cache.push((entity, MaterialType::Color(handle)));
                    },
                );
            }
            ShapeMaterialType::Gradient(gradient) => {
                spawn_shape_mesh(
                    commands,
                    gradient_materials,
                    mesh,
                    gradient.clone(),
                    swf_transform,
                    blend_mode,
                    z_index,
                    |entity, handle| {
                        shape_material_handle_cache.push((entity, MaterialType::Gradient(handle)));
                    },
                );
            }
            ShapeMaterialType::Bitmap(bitmap) => {
                spawn_shape_mesh(
                    commands,
                    bitmap_materials,
                    mesh,
                    bitmap.clone(),
                    swf_transform,
                    blend_mode,
                    z_index,
                    |entity, handle| {
                        shape_material_handle_cache.push((entity, MaterialType::Bitmap(handle)));
                    },
                );
            }
        }
    }
}

/// 处理RenderBitmap命令
#[allow(clippy::too_many_arguments)]
fn process_render_bitmap_command(
    commands: &mut EntityCommands,
    shape_command: &ShapeCommand,
    layer_shape_material_cache: &mut HashMap<String, Vec<(Entity, MaterialType)>>,
    shape_meshes: &mut Query<(&ChildOf, &mut Transform, &mut Visibility), With<ShapeMesh>>,
    bitmap_materials: &mut Assets<BitmapMaterial>,
    filter_texture_mesh: &FilterTextureMesh,
    z_index: f32,
    scale: Vec3,
) {
    if let ShapeCommand::RenderBitmap {
        bitmap_material,
        shape_depth_layer,
        size,
    } = shape_command
    {
        let raw_size = *size / scale.xy();

        spawn_or_update_bitmap(
            layer_shape_material_cache,
            shape_depth_layer,
            shape_meshes,
            bitmap_materials,
            bitmap_material,
            raw_size,
            z_index,
            |bitmap_material_handle, shape_material_handle_cache| {
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
                });
            },
        );
    }
}

/// 更新形状材质
fn update_shape_material(
    handle: &MaterialType,
    color_materials: &mut Assets<ColorMaterial>,
    gradient_materials: &mut Assets<GradientMaterial>,
    bitmap_materials: &mut Assets<BitmapMaterial>,
    swf_transform: &SwfTransform,
    blend_mode: &BlendMode,
) {
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

/// 更新或创建位图
fn spawn_or_update_bitmap(
    shape_material_cache: &mut HashMap<String, Vec<(Entity, MaterialType)>>,
    shape_depth_layer: &String,
    shape_meshes: &mut Query<(&ChildOf, &mut Transform, &mut Visibility), With<ShapeMesh>>,
    bitmap_materials: &mut Assets<BitmapMaterial>,
    bitmap_material: &BitmapMaterial,
    size: Vec2,
    z_index: f32,
    func: impl FnOnce(Handle<BitmapMaterial>, &mut Vec<(Entity, MaterialType)>),
) {
    // 尝试查找并更新已有位图
    if let Some(shape_material_handle_cache) = shape_material_cache.get(shape_depth_layer)
        && let Some((entity, bitmap_material_handle)) = shape_material_handle_cache.first()
        && let MaterialType::Bitmap(bitmap_material_handle) = bitmap_material_handle
    {
        update_existing_bitmap(
            entity,
            bitmap_material_handle,
            shape_meshes,
            bitmap_materials,
            bitmap_material,
            size,
            z_index,
        );
    } else {
        // 创建新位图
        create_new_bitmap(
            shape_material_cache,
            shape_depth_layer,
            bitmap_materials,
            bitmap_material,
            func,
        );
    }
}

/// 更新已有位图
fn update_existing_bitmap(
    entity: &Entity,
    bitmap_material_handle: &Handle<BitmapMaterial>,
    shape_meshes: &mut Query<(&ChildOf, &mut Transform, &mut Visibility), With<ShapeMesh>>,
    bitmap_materials: &mut Assets<BitmapMaterial>,
    bitmap_material: &BitmapMaterial,
    size: Vec2,
    z_index: f32,
) {
    // 更新变换
    let Ok((_, mut transform, mut visibility)) = shape_meshes.get_mut(*entity) else {
        return;
    };
    transform.translation.z = z_index;
    *visibility = Visibility::Inherited;

    // 更新材质
    let Some(entity_bitmap_material) = bitmap_materials.get_mut(bitmap_material_handle) else {
        return;
    };

    entity_bitmap_material.transform = bitmap_material.transform;
    entity_bitmap_material.transform.world_transform.x_axis.x = size.x;
    entity_bitmap_material.transform.world_transform.y_axis.y = size.y;
    entity_bitmap_material.blend_key = bitmap_material.blend_key;
    entity_bitmap_material.texture = bitmap_material.texture.clone();
}

/// 创建新位图
fn create_new_bitmap(
    shape_material_cache: &mut HashMap<String, Vec<(Entity, MaterialType)>>,
    shape_depth_layer: &String,
    bitmap_materials: &mut Assets<BitmapMaterial>,
    bitmap_material: &BitmapMaterial,
    func: impl FnOnce(Handle<BitmapMaterial>, &mut Vec<(Entity, MaterialType)>),
) {
    // 获取或创建形状材质缓存
    let shape_material_handle_cache = shape_material_cache
        .entry(shape_depth_layer.to_owned())
        .or_default();

    // 添加位图材质并创建实体
    let bitmap_material_handle = bitmap_materials.add(bitmap_material.clone());
    func(bitmap_material_handle.clone(), shape_material_handle_cache);
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
