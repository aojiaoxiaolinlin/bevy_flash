use std::collections::btree_map::ValuesMut;

use bevy::{
    asset::{Assets, Handle},
    color::Color,
    ecs::{
        entity::{Entity, EntityHashMap},
        event::EntityEvent,
        query::Without,
        system::{Commands, Local, Query, Res, ResMut},
    },
    image::Image,
    log::{info, warn_once},
    math::{IVec2, Mat4, UVec2, Vec3},
    mesh::Mesh,
    platform::collections::HashMap,
    time::Time,
    transform::components::{GlobalTransform, Transform},
};

use swf::{CharacterId, Depth, Rectangle, Twips};

use crate::{
    assets::{Shape, Swf},
    commands::{DrawShapes, OffscreenDrawShapes, ShapeCommand},
    player::{Flash, FlashPlayer, FlashPlayerTimer, McRoot},
    render::{
        ColorMaterialHandle, FilterTextureMesh,
        blend_pipeline::BlendMode,
        filter_render::Filters,
        material::{BitmapMaterial, ColorMaterial, GradientMaterial},
        offscreen_render::OffscreenCamera,
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

/// Flash 动画完成事件，非循环播放时触发
#[derive(EntityEvent, Clone)]
pub struct FlashCompleteEvent {
    /// 实体
    entity: bevy::prelude::Entity,
    /// 当前播放的动画名
    name: Option<String>,
}

impl FlashCompleteEvent {
    /// 实体
    pub fn entity(&self) -> bevy::prelude::Entity {
        self.entity
    }
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

/// Flash 动画帧事件
#[derive(EntityEvent, Clone)]
pub struct FlashFrameEvent {
    /// 实体
    entity: bevy::prelude::Entity,
    /// 帧事件名
    name: String,
}
impl FlashFrameEvent {
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// 用于缓存每个实体对应的显示对象
#[derive(Default)]
pub struct DisplayObjectCache {
    morph_shape_frame_cache: HashMap<CharacterId, fnv::FnvHashMap<u16, Frame>>,
    layer_offscreen_cache: HashMap<String, Entity>,
    image_cache: HashMap<String, ImageCache>,

    /// 是否需要翻转 X 轴
    flip_x: bool,
    /// 是否需要翻转 Y 轴
    flip_y: bool,
}

#[derive(Debug)]
pub struct ImageCacheDraw {
    layer: String,
    handle: Handle<Image>,
    clear_color: Color,
    filters: Vec<Filter>,
    commands: Vec<ShapeCommand>,
    dirty: bool,
    size: UVec2,
}

pub struct RenderContext<'w> {
    // 系统资源
    pub shapes: &'w mut Assets<Shape>,
    pub meshes: &'w mut Assets<Mesh>,
    pub images: &'w mut Assets<Image>,
    pub gradients: &'w mut Assets<GradientMaterial>,
    pub bitmaps: &'w mut Assets<BitmapMaterial>,
    pub filter_texture_mesh: &'w FilterTextureMesh,
    pub color_material: &'w Handle<ColorMaterial>,

    // 渲染需要的数据
    pub transform_stack: &'w mut TransformStack,
    pub cache_draws: &'w mut Vec<ImageCacheDraw>,
    pub shape_handles: &'w HashMap<CharacterId, Handle<Shape>>,
    pub commands: Vec<ShapeCommand>,
    pub scale: Vec3,

    // 缓存相关
    /// 变形形状纹理缓存
    pub morph_shape_cache: &'w mut HashMap<CharacterId, fnv::FnvHashMap<u16, Frame>>,
    /// Image 缓存
    pub image_cache: &'w mut HashMap<String, ImageCache>,

    /// 是否需要翻转 X 轴
    pub flip_x: bool,
    /// 是否需要翻转 Y 轴
    pub flip_y: bool,

    /// 正在绘制的遮罩区域数量
    pub maskers_in_progress: u8,
    pub needs_stencil: bool,
}

impl<'w> RenderContext<'w> {
    pub fn new(
        shapes: &'w mut Assets<Shape>,
        meshes: &'w mut Assets<Mesh>,
        images: &'w mut Assets<Image>,
        gradients: &'w mut Assets<GradientMaterial>,
        bitmaps: &'w mut Assets<BitmapMaterial>,
        morph_shape_cache: &'w mut HashMap<CharacterId, fnv::FnvHashMap<u16, Frame>>,
        transform_stack: &'w mut TransformStack,
        image_cache: &'w mut HashMap<String, ImageCache>,
        cache_draws: &'w mut Vec<ImageCacheDraw>,
        shape_handles: &'w HashMap<CharacterId, Handle<Shape>>,
        filter_texture_mesh: &'w FilterTextureMesh,
        color_material: &'w Handle<ColorMaterial>,
        scale: Vec3,
        flip_x: bool,
        flip_y: bool,
    ) -> Self {
        Self {
            shapes,
            meshes,
            images,
            gradients,
            bitmaps,
            transform_stack,
            cache_draws,
            shape_handles,
            commands: Vec::new(),
            scale,
            morph_shape_cache,
            image_cache,
            filter_texture_mesh,
            color_material,
            flip_x,
            flip_y,
            needs_stencil: false,
            maskers_in_progress: 0,
        }
    }

    pub fn render_shape(
        &mut self,
        handle: Handle<Shape>,
        transform: SwfTransform,
        blend_mode: BlendMode,
    ) {
        let draw_shape = self
            .shapes
            .get(handle.id())
            .expect("Shape must exist.")
            .clone();
        self.commands.push(ShapeCommand::RenderShape {
            draw_shape,
            transform,
            blend_mode,
        });
    }

    #[inline]
    pub fn push_mask(&mut self) {
        self.needs_stencil = true;
        if self.maskers_in_progress == 0 {
            self.commands.push(ShapeCommand::PushMask);
        }
        self.maskers_in_progress += 1;
    }

    #[inline]
    pub fn activate_mask(&mut self) {
        self.needs_stencil = true;
        self.maskers_in_progress -= 1;
        if self.maskers_in_progress == 0 {
            self.commands.push(ShapeCommand::ActivateMask);
        }
    }

    #[inline]
    pub fn deactivate_mask(&mut self) {
        self.needs_stencil = true;
        if self.maskers_in_progress == 0 {
            self.commands.push(ShapeCommand::DeactivateMask);
        }
        self.maskers_in_progress += 1;
    }

    #[inline]
    pub fn pop_mask(&mut self) {
        self.needs_stencil = true;
        self.maskers_in_progress -= 1;
        if self.maskers_in_progress == 0 {
            self.commands.push(ShapeCommand::PopMask);
        }
    }
}

/// 推进Flash动画
pub fn advance_animation(
    time: Res<Time>,
    filter_texture_mesh: Res<FilterTextureMesh>,
    mut commands: Commands,
    mut player: Query<(
        Entity,
        &mut FlashPlayer,
        &mut FlashPlayerTimer,
        &mut McRoot,
        &mut Transform,
        &Flash,
        &GlobalTransform,
    )>,
    mut offscreen_renders: Query<(&mut OffscreenCamera, &mut Filters)>,
    mut shapes: ResMut<Assets<Shape>>,
    swf_res: Res<Assets<Swf>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    color_material: Res<ColorMaterialHandle>,
    mut gradients: ResMut<Assets<GradientMaterial>>,
    mut bitmaps: ResMut<Assets<BitmapMaterial>>,
    mut display_object_entity_caches: Local<EntityHashMap<DisplayObjectCache>>,
) {
    let mut current_live_player = vec![];
    // 1. 将动画的每一帧将离屏渲染实体列为不活跃
    mark_offscreen_textures_inactive(&mut offscreen_renders);
    // 2. 更新动画帧
    for (entity, mut player, mut timer, mut root, mut transform, swf, global_transform) in
        player.iter_mut()
    {
        current_live_player.push(entity);
        if timer
            .tick(time.delta().mul_f32(player.speed()))
            .just_finished()
        {
            // 处理动画完成事件
            if handle_animation_complete(&mut commands, entity, &mut player) {
                continue;
            }

            let Some(swf) = swf_res.get(swf.id()) else {
                continue;
            };

            // 处理循环播放逻辑
            handle_animation_loop(&mut player, &mut root, swf);

            // 更新动画帧并触发帧事件
            update_animation_frame(&mut commands, entity, &mut player, &mut root, swf);

            let display_object_cache = display_object_entity_caches.entry(entity).or_default();
            // 处理翻转缩放
            let flip_x = &mut display_object_cache.flip_x;
            let flip_y = &mut display_object_cache.flip_y;
            let global_scale = global_transform.scale();
            if transform.scale.x < 0.0 {
                transform.scale.x = transform.scale.x.abs();
                *flip_x = true;
            }
            if transform.scale.y < 0.0 {
                transform.scale.y = transform.scale.y.abs();
                *flip_y = true;
            }

            let morph_shape_cache: &mut _ = &mut display_object_cache.morph_shape_frame_cache;
            let image_cache = &mut display_object_cache.image_cache;

            // 处理DisplayList
            let mut cache_draws = vec![];
            let mut transform_stack = TransformStack::default();
            // 创建渲染上下文
            let mut context = RenderContext::new(
                shapes.as_mut(),
                meshes.as_mut(),
                images.as_mut(),
                gradients.as_mut(),
                bitmaps.as_mut(),
                morph_shape_cache,
                &mut transform_stack,
                image_cache,
                &mut cache_draws,
                swf.shapes(),
                filter_texture_mesh.as_ref(),
                &color_material.0,
                global_scale,
                *flip_x,
                *flip_y,
            );
            process_display_list(
                root.render_list_mut(),
                &mut context,
                swf::BlendMode::Normal,
                String::from("0"),
                true,
            );
            commands.entity(entity).insert(DrawShapes(context.commands));

            // 处理离屏绘制
            spawn_offscreen_texture(
                &mut commands,
                entity,
                cache_draws,
                &mut offscreen_renders,
                display_object_cache,
                global_scale,
            );
        }
    }
    display_object_entity_caches.retain(|entity, _| current_live_player.contains(entity));
}

struct CacheInfo {
    image_info: ImageCacheInfo,
    dirty: bool,
    base_transform: SwfTransform,
    bounds: Rectangle<Twips>,
    draw_offset: IVec2,
    filters: Vec<Filter>,
}

pub fn prepare_root_clip(
    mut commands: Commands,
    mut player: Query<(Entity, &mut FlashPlayer, &Flash), Without<McRoot>>,
    swf_res: Res<Assets<Swf>>,
) {
    for (entity, mut player, flash) in player.iter_mut() {
        let Some(swf) = swf_res.get(flash.id()) else {
            continue;
        };
        let mut root = McRoot(MovieClip::new(swf.movie()));
        player.play_target_animation(swf, &mut root);
        commands.entity(entity).insert(root);
    }
}

fn mark_offscreen_textures_inactive(
    offscreen_renders: &mut Query<(&mut OffscreenCamera, &mut Filters)>,
) {
    offscreen_renders
        .iter_mut()
        .for_each(|(mut offscreen_camera, _)| {
            offscreen_camera.is_active = false;
        });
}

fn handle_animation_complete(
    commands: &mut Commands,
    entity: Entity,
    player: &mut FlashPlayer,
) -> bool {
    if !player.is_looping() && player.is_completed() {
        // 触发完成事件
        if player.try_complete() {
            commands.trigger(FlashCompleteEvent {
                entity,
                name: player.current_animation().map(|s| s.into()),
            });
        }
        return true;
    }
    false
}

fn handle_animation_loop(player: &mut FlashPlayer, root: &mut McRoot, swf: &Swf) {
    if player.is_looping() && player.is_completed() {
        // 循环播放，跳回当前动画第一帧
        player.play_target_animation(swf, root);
    }
}

fn update_animation_frame(
    commands: &mut Commands,
    entity: Entity,
    player: &mut FlashPlayer,
    root: &mut MovieClip,
    swf: &Swf,
) {
    let characters = swf.characters();
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

fn process_display_list(
    display_list: ValuesMut<'_, u16, DisplayObject>,
    context: &mut RenderContext<'_>,
    blend_mode: swf::BlendMode,
    shape_depth_layer: String,
    is_root: bool,
) {
    // TODO:混合模式也有多个MC合成的情况，这里暂时没实现多MC合成的情况，暂时只实现单个图形的情况
    // 实现方案：参考 Ruffle 中的处理方式，由多个Shape合成的MC上实现Blend模式需要渲染到一个OffscreenCamera中，
    // 然后将OffscreenCamera渲染到屏幕上，这样做又需要使用OffscreenDrawCommand在渲染图中实现。
    let blend_mode = if display_list.len() > 1 {
        swf::BlendMode::Normal
    } else {
        blend_mode
    };

    let mut clip_depth = 0;
    let mut clip_depth_stack: Vec<(Depth, DisplayObject)> = vec![];
    for display_object in display_list {
        // 判断当前要渲染的子对象是否已经超出了当前遮罩的“管辖范围”。如果超出了，就必须把遮罩关掉
        while clip_depth > 0 && display_object.depth() > clip_depth {
            let (prev_clip_depth, mut prev_clip_object) = clip_depth_stack.pop().unwrap();
            clip_depth = prev_clip_depth;
            context.deactivate_mask();
            process_display_object(
                &mut prev_clip_object,
                context,
                blend_mode,
                &shape_depth_layer,
                is_root,
            );
            context.pop_mask();
        }
        if display_object.clip_depth() > 0 && display_object.allow_as_mask() {
            info!("Processing mask display object，{}", display_object.id());
            clip_depth_stack.push((clip_depth, display_object.clone()));
            clip_depth = display_object.clip_depth();
            // 1. 标记为遮罩
            context.push_mask();
            // 2. 渲染（画遮罩）
            process_display_object(
                display_object,
                context,
                blend_mode,
                &shape_depth_layer,
                is_root,
            );
            // 3. 标记遮罩活跃
            context.activate_mask();
        } else {
            info!("Processing display object，{}", display_object.id());
            // 作为普通显示对象处理
            process_display_object(
                display_object,
                context,
                blend_mode,
                &shape_depth_layer,
                is_root,
            );
        }
    }
    for (_, mut clip_display_object) in clip_depth_stack.into_iter().rev() {
        context.deactivate_mask();
        process_display_object(
            &mut clip_display_object,
            context,
            blend_mode,
            &shape_depth_layer,
            false,
        );
        context.pop_mask();
    }
}

fn process_display_object(
    display_object: &mut DisplayObject,
    context: &mut RenderContext<'_>,
    blend_mode: swf::BlendMode,
    shape_depth_layer: &str,
    is_root: bool,
) {
    let id = display_object.id();

    let shape_depth_layer = format!("{}_{}_{}", shape_depth_layer, display_object.depth(), id);

    let transform = if is_root {
        let mut transform = *display_object.transform();
        transform.matrix.a = if context.flip_x {
            -transform.matrix.a
        } else {
            transform.matrix.a
        };
        transform.matrix.d = if context.flip_y {
            -transform.matrix.d
        } else {
            transform.matrix.d
        };
        transform
    } else {
        *display_object.transform()
    };
    // 保存当前变换状态
    context.transform_stack.push(&transform);

    // 确定混合模式
    let blend_mode = determine_blend_mode(blend_mode, display_object);

    // 处理缓存和滤镜
    let cache_info = process_cache_and_filters(display_object, context, id, &shape_depth_layer);

    // 根据是否有缓存信息选择渲染方式
    if let Some(cache_info) = cache_info {
        render_with_cache(
            display_object,
            context,
            cache_info,
            blend_mode,
            &shape_depth_layer,
        );
    } else {
        // 直接渲染显示对象
        render_display_object(display_object, context, blend_mode, shape_depth_layer);
    }

    // TODO:处理复杂混合模式

    // 恢复变换状态
    context.transform_stack.pop();
}

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

fn process_cache_and_filters(
    display_object: &mut DisplayObject,
    context: &mut RenderContext<'_>,
    id: CharacterId,
    shape_depth_layer: &str,
) -> Option<CacheInfo> {
    // 获取基本变换和边界
    let base_transform = context.transform_stack.transform();
    let bounds =
        display_object.render_bounds_with_transform(&base_transform.matrix, false, context);

    // 处理滤镜
    let mut filters = display_object.filters();
    let swf_version = display_object.swf_version();
    filters.retain(|f| !f.impotent());

    display_object.recheck_cache(shape_depth_layer, context.image_cache);

    // 如果没有缓存，直接返回None
    let cache = context.image_cache.get_mut(shape_depth_layer)?;

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
            dirty: cache.dirty(),
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

fn render_with_cache(
    display_object: &mut DisplayObject,
    context: &mut RenderContext<'_>,
    cache_info: CacheInfo,
    blend_mode: swf::BlendMode,
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
            shape_depth_layer,
        );
    }

    // 将缓存的纹理作为位图渲染到主视图
    render_cached_texture_to_view(context, &cache_info, offset_x, offset_y, blend_mode);
}

fn render_to_offscreen_texture(
    display_object: &mut DisplayObject,
    context: &mut RenderContext<'_>,
    cache_info: &CacheInfo,
    offset_x: Twips,
    offset_y: Twips,
    blend_mode: swf::BlendMode,
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
        context.shapes,
        context.meshes,
        context.images,
        context.gradients,
        context.bitmaps,
        context.morph_shape_cache,
        &mut transform_stack,
        context.image_cache,
        context.cache_draws,
        context.shape_handles,
        context.filter_texture_mesh,
        context.color_material,
        context.scale,
        false,
        false,
    );

    // 渲染显示对象到离屏上下文
    render_display_object(
        display_object,
        &mut offscreen_context,
        blend_mode,
        shape_depth_layer.to_owned(),
    );

    // 将离屏上下文的绘制命令添加到缓存绘制列表
    offscreen_context.cache_draws.push(ImageCacheDraw {
        layer: shape_depth_layer.to_string(),
        handle: cache_info.image_info.handle(),
        clear_color: Color::NONE,
        commands: offscreen_context.commands,
        filters: cache_info.filters.clone(),
        dirty: true,
        size: cache_info.image_info.size(),
    });
}

fn render_cached_texture_to_view(
    context: &mut RenderContext<'_>,
    cache_info: &CacheInfo,
    offset_x: Twips,
    offset_y: Twips,
    blend_mode: swf::BlendMode,
) {
    // 获取当前变换矩阵和缩放
    let matrix = context.transform_stack.transform().matrix;
    let scale = context.scale;

    // 创建位图材质
    let bitmap_material = BitmapMaterial {
        texture: cache_info.image_info.handle(),
        texture_transform: Mat4::IDENTITY,
    };

    // 添加渲染位图命令
    context.commands.push(ShapeCommand::RenderBitmap {
        mesh: context.filter_texture_mesh.0.clone(),
        material: context.bitmaps.add(bitmap_material),
        transform: SwfTransform {
            matrix: Matrix {
                a: cache_info.image_info.size().x as f32 / scale.x,
                d: cache_info.image_info.size().y as f32 / scale.y,
                tx: matrix.tx + offset_x,
                ty: matrix.ty + offset_y,
                ..Default::default()
            },
            color_transform: cache_info.base_transform.color_transform,
        },
        blend_mode: BlendMode::from(blend_mode),
    });
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
                false,
            );
        }
        DisplayObject::Graphic(graphic) => {
            graphic.render_self(context, blend_mode);
        }
        DisplayObject::MorphShape(morph_shape) => {
            morph_shape.render_self(context, blend_mode);
        }
    }
}

fn spawn_offscreen_texture(
    commands: &mut Commands,
    entity: Entity,
    cache_draws: Vec<ImageCacheDraw>,
    offscreen_renders: &mut Query<(&mut OffscreenCamera, &mut Filters)>,
    display_object_cache: &mut DisplayObjectCache,
    scale: Vec3,
) {
    let layer_offscreen_cache = &mut display_object_cache.layer_offscreen_cache;

    let mut order = isize::MIN;

    for cache_draw in cache_draws.iter() {
        // 只处理需要更新的纹理
        if !cache_draw.dirty {
            continue;
        }

        // 更新或创建离屏纹理实体
        if let Some(entity) = layer_offscreen_cache.get(&cache_draw.layer) {
            let Ok((mut offscreen_camera, mut filters)) = offscreen_renders.get_mut(*entity) else {
                return;
            };

            // 更新离屏纹理属性
            offscreen_camera.is_active = true;
            offscreen_camera.target = cache_draw.handle.clone().into();
            offscreen_camera.size = cache_draw.size;
            offscreen_camera.scale = scale;

            filters.clear();
            filters.extend(cache_draw.filters.clone());

            // 更新绘制命令
            commands
                .entity(*entity)
                .insert(OffscreenDrawShapes(cache_draw.commands.clone()));
        } else {
            order += 1;
            commands.entity(entity).with_children(|parent| {
                let entity = parent
                    .spawn((
                        OffscreenCamera {
                            target: cache_draw.handle.clone().into(),
                            is_active: true,
                            size: cache_draw.size,
                            clear_color: cache_draw.clear_color,
                            order,
                            scale,
                        },
                        Filters(cache_draw.filters.clone()),
                        OffscreenDrawShapes(cache_draw.commands.clone()),
                    ))
                    .id();
                // 缓存新创建的实体
                layer_offscreen_cache.insert(cache_draw.layer.to_owned(), entity);
            });
        }
    }
}
