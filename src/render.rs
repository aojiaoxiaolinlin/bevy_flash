use std::sync::Arc;
use std::sync::atomic::AtomicUsize;

use bevy::asset::{RenderAssetUsages, weak_handle};
use bevy::color::LinearRgba;
use bevy::ecs::schedule::IntoScheduleConfigs;
use bevy::ecs::system::{EntityCommands, Local, Single};
use bevy::image::{BevyDefault, Image};
use bevy::math::{UVec2, Vec2, Vec3A, Vec4};
use bevy::platform_support::collections::hash_map::Entry;
use bevy::platform_support::collections::{HashMap, HashSet};
use bevy::reflect::Reflect;
use bevy::render::camera::{NormalizedRenderTarget, RenderTarget};
use bevy::render::extract_component::{ExtractComponent, ExtractComponentPlugin};
use bevy::render::mesh::{Indices, MeshAabb, PrimitiveTopology};
use bevy::render::primitives::Aabb;
use bevy::render::render_asset::{self, RenderAssets};
use bevy::render::render_graph::{InternedRenderSubGraph, RenderSubGraph};
use bevy::render::render_resource::{
    CachedRenderPipelineId, Extent3d, SpecializedRenderPipelines, TextureDescriptor,
    TextureDimension, TextureFormat, TextureUsages,
};
use bevy::render::renderer::RenderDevice;
use bevy::render::sync_world::{MainEntity, RenderEntity, SyncToRenderWorld};
use bevy::render::texture::{ColorAttachment, GpuImage, OutputColorAttachment, TextureCache};
use bevy::render::view::{MainTargetTextures, Msaa, ViewTarget, ViewTargetAttachments};
use bevy::render::{Extract, ExtractSchedule, Render, RenderSet};
use bevy::sprite::AlphaMode2d;
use bevy::transform::components::GlobalTransform;
use bevy::window::Window;
use bevy::{
    app::{App, Plugin, Update},
    asset::{Assets, Handle, load_internal_asset},
    math::{Mat4, Vec3},
    prelude::{
        Children, Commands, Component, Deref, DerefMut, Entity, Mesh, Mesh2d, Query,
        ReflectComponent, Res, ResMut, Shader, Transform, Visibility, With, Without,
    },
    render::RenderApp,
    sprite::{Material2dPlugin, MeshMaterial2d},
};
use blend_pipeline::BlendType;
use flash_an_runtime::core::filter::Filter;
use graph::{FlashFilterRenderPlugin, FlashFilterSubGraph, RenderPhases};
use material::{
    BitmapMaterial, GradientMaterial, GradientUniforms, SwfColorMaterial, SwfMaterial, SwfTransform,
};
use pipeline::{
    BEVEL_FILTER_SHADER_HANDLE, BLUR_FILTER_SHADER_HANDLE, COLOR_MATRIX_FILTER_SHADER_HANDLE,
    GLOW_FILTER_SHADER_HANDLE, INTERMEDIATE_TEXTURE_GRADIENT, INTERMEDIATE_TEXTURE_MESH,
    IntermediateTexturePipeline, specialize_meshes,
};
use raw_vertex::{Vertex, VertexColor};
use swf::{CharacterId, Rectangle as SwfRectangle, Twips};

use crate::assets::FlashAnimationSwfData;
use crate::bundle::{FlashAnimation, FlashShapeSpawnRecord, SwfGraph};
use crate::{FlashAnimationActiveInstance, ShapeDrawType, flash_update};

pub(crate) mod blend_pipeline;
mod graph;
pub(crate) mod material;
mod pipeline;
mod raw_vertex;

pub const SWF_COLOR_MATERIAL_SHADER_HANDLE: Handle<Shader> =
    weak_handle!("8c2a5b0f-3e6d-4f8a-b217-84d2f5e1c9b3");
pub const GRADIENT_MATERIAL_SHADER_HANDLE: Handle<Shader> =
    weak_handle!("5e9f1a78-9b34-4c15-8d7e-2a3b0f47d862");
pub const BITMAP_MATERIAL_SHADER_HANDLE: Handle<Shader> =
    weak_handle!("a34c7d82-1f5b-4a9e-93d8-6b7e20c45a1f");
pub const FLASH_COMMON_MATERIAL_SHADER_HANDLE: Handle<Shader> =
    weak_handle!("e53b9f82-6a4c-4d5b-91e7-4f2a63b8c5d9");

type SwfShapeMeshQuery = (
    Entity,
    &'static mut Aabb,
    Option<&'static MeshMaterial2d<SwfColorMaterial>>,
    Option<&'static MeshMaterial2d<GradientMaterial>>,
    Option<&'static MeshMaterial2d<BitmapMaterial>>,
    &'static SwfShapeMeshAabb,
);

pub struct FlashRenderPlugin;

impl Plugin for FlashRenderPlugin {
    fn build(&self, app: &mut App) {
        load_internal_asset!(
            app,
            FLASH_COMMON_MATERIAL_SHADER_HANDLE,
            "render/shaders/common.wgsl",
            Shader::from_wgsl
        );
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
        load_internal_asset!(
            app,
            INTERMEDIATE_TEXTURE_MESH,
            "render/shaders/intermediate_texture/color.wgsl",
            Shader::from_wgsl
        );
        load_internal_asset!(
            app,
            INTERMEDIATE_TEXTURE_GRADIENT,
            "render/shaders/intermediate_texture/gradient.wgsl",
            Shader::from_wgsl
        );
        load_internal_asset!(
            app,
            BLUR_FILTER_SHADER_HANDLE,
            "render/shaders/filters/blur.wgsl",
            Shader::from_wgsl
        );
        load_internal_asset!(
            app,
            COLOR_MATRIX_FILTER_SHADER_HANDLE,
            "render/shaders/filters/color_matrix.wgsl",
            Shader::from_wgsl
        );
        load_internal_asset!(
            app,
            GLOW_FILTER_SHADER_HANDLE,
            "render/shaders/filters/glow.wgsl",
            Shader::from_wgsl
        );
        load_internal_asset!(
            app,
            BEVEL_FILTER_SHADER_HANDLE,
            "render/shaders/filters/bevel.wgsl",
            Shader::from_wgsl
        );

        app.add_plugins(Material2dPlugin::<GradientMaterial>::default())
            .add_plugins(Material2dPlugin::<SwfColorMaterial>::default())
            .add_plugins(Material2dPlugin::<BitmapMaterial>::default())
            .add_plugins(FlashFilterRenderPlugin)
            .add_plugins(ExtractComponentPlugin::<FlashFilters>::default())
            .add_systems(Update, generate_swf_mesh.after(flash_update));

        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<RenderPhases>()
            .init_resource::<SpecializedRenderPipelines<IntermediateTexturePipeline>>()
            .add_systems(
                ExtractSchedule,
                (extract_intermediate_phase, extract_intermediate_texture),
            )
            .add_systems(
                Render,
                (
                    specialize_meshes.in_set(RenderSet::PrepareAssets),
                    prepare_view_attachments
                        .in_set(RenderSet::ManageViews)
                        .before(prepare_intermediate_texture_view_targets),
                    prepare_intermediate_texture_view_targets
                        .in_set(RenderSet::ManageViews)
                        .after(prepare_view_attachments)
                        .after(render_asset::prepare_assets::<GpuImage>),
                    queue_swf_vertex.in_set(RenderSet::Queue),
                ),
            );
    }

    fn finish(&self, app: &mut App) {
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app.init_resource::<IntermediateTexturePipeline>();
        }
    }
}

#[derive(Component, Default)]
pub struct SwfShapeChildMesh;

/// 用于记录网格的aabb，没帧不在重计算的Aabb。防止顶点动画实体被剔除
#[derive(Component, Default, Deref, DerefMut)]
pub struct SwfShapeMeshAabb(Aabb);

#[derive(Component, Clone, Debug, Default, ExtractComponent, DerefMut, Deref)]
pub struct FlashFilters(Vec<Filter>);

/// Configures the [`RenderGraph`](crate::render_graph::RenderGraph) name assigned to be run for a given [`IntermediateTexture`] entity.
#[derive(Component, Debug, Deref, DerefMut, Reflect, Clone)]
#[reflect(opaque)]
#[reflect(Component, Debug, Clone)]
pub struct FlashFilterRenderGraph(InternedRenderSubGraph);

impl FlashFilterRenderGraph {
    /// Creates a new [`FlashFilterRenderGraph`] from any string-like type.
    #[inline]
    pub fn new<T: RenderSubGraph>(name: T) -> Self {
        Self(name.intern())
    }
}

/// 用于需要进行滤镜处理得到中间纹理
#[derive(Component, Default, Clone)]
#[require(SyncToRenderWorld, FlashFilterRenderGraph::new(FlashFilterSubGraph))]
pub struct IntermediateTexture {
    /// target
    target: RenderTarget,
    /// 当前帧是否渲染
    is_active: bool,
    /// 全局变换缩放倍数，用于矢量缩放
    scale: Vec3,
    /// Graphic 原始bunds大小
    size: UVec2,
    /// 应用滤镜后的bunds大小
    filter_size: UVec2,
    /// 中间纹理包含的子实体（`Mesh2d`）
    view_entities: Vec<SwfVertex>,
    /// swf 的world transform 变换，原swf的变换数据由于具有倾斜功能，目前无法使用`bevy`的`Transform`代替
    world_transform: Mat4,
}

#[derive(Component, Clone)]
pub struct ExtractedIntermediateTexture {
    /// target
    target: Option<NormalizedRenderTarget>,
    /// 全局变换缩放倍数，用于矢量缩放
    scale: Vec3,
    /// Graphic 原始bunds大小
    size: UVec2,
    /// 应用滤镜后的bunds大小
    filter_size: UVec2,
    /// 中间纹理包含的子实体（`Mesh2d`）
    view_entities: Vec<SwfVertex>,
    /// swf 的world transform 变换，保证矢量缩放、旋转。
    world_transform: Mat4,
    render_graph: InternedRenderSubGraph,
}

#[allow(clippy::too_many_arguments)]
pub fn generate_swf_mesh(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut flash_assets: ResMut<Assets<FlashAnimationSwfData>>,
    mut swf_color_materials: ResMut<Assets<SwfColorMaterial>>,
    mut gradient_materials: ResMut<Assets<GradientMaterial>>,
    mut bitmap_materials: ResMut<Assets<BitmapMaterial>>,
    mut query: Query<(
        Entity,
        &FlashAnimation,
        &mut FlashShapeSpawnRecord,
        &mut FlashAnimationActiveInstance,
        &GlobalTransform,
        Option<&Children>,
    )>,
    mut flash_material_query: Query<SwfShapeMeshQuery, With<SwfShapeChildMesh>>,
    mut intermediate_textures: Query<
        (
            Entity,
            &mut IntermediateTexture,
            &mut FlashFilters,
            &mut Aabb,
            &SwfShapeMeshAabb,
            &MeshMaterial2d<BitmapMaterial>,
        ),
        Without<SwfShapeChildMesh>,
    >,
    mut shape_query: Query<
        (Entity, Option<&Children>, &mut Transform),
        (With<SwfGraph>, Without<SwfShapeChildMesh>),
    >,
    mut current_shape_entities: Local<Vec<Entity>>,
    mut marker_shape_ref: Local<HashMap<CharacterId, usize>>,
    window: Single<&Window>,
) {
    for (
        entity,
        flash_animation,
        mut flash_shape_record,
        mut active_instances,
        global_transform,
        children,
    ) in query.iter_mut()
    {
        current_shape_entities.clear();
        marker_shape_ref.clear();
        // 将当前flash动画下的中间纹理标记为不可见
        if let Some(children) = children {
            intermediate_textures
                .iter_mut()
                .filter(|(entity, _, _, _, _, _)| children.contains(entity))
                .for_each(|(_, mut intermediate_texture, _, _, _, _)| {
                    intermediate_texture.is_active = false;
                });
        }
        let scale = global_transform.scale();
        if let Some(flash_asset) = flash_assets.get_mut(flash_animation.swf_asset.id()) {
            let mut current_entity = Vec::new();
            let mut z_index = 1e-3;
            for active_instance in active_instances.iter_mut() {
                // 记录同一个实体的引用计数
                let ref_count = marker_shape_ref.entry(active_instance.id()).or_default();
                *ref_count += 1;
                // 每个 shape 提升一个数量级
                z_index += 2e-2;
                // 记录当前生成的实体
                current_entity.push(active_instance.id().clone());
                // 指定的shape id
                let id = active_instance.id();
                // flash 动画变换数据
                let swf_transform = SwfTransform {
                    world_transform: active_instance.transform(),
                    mult_color: Vec4::from_array(
                        active_instance.color_transform().mult_rgba_normalized(),
                    ),
                    add_color: Vec4::from_array(
                        active_instance.color_transform().add_rgba_normalized(),
                    ),
                };
                // flash 混合模式
                let blend: AlphaMode2d = BlendType::from(active_instance.blend()).into();
                // 获取当前实例的swf变换矩阵，用于计算filter_rect
                let matrix = active_instance.transform_matrix();
                // 提取当前实例的滤镜
                active_instance.filters_mut().retain(|f| !f.impotent());
                let filters = active_instance.filters_mut();

                let current_shape_entity;
                if let Some(entity) = flash_shape_record.get_entity(id, *ref_count) {
                    // shape实体已经生成。只需要更新其Mesh2d
                    let (shape_entity, shape_children, mut transform) = shape_query
                        .iter_mut()
                        .find(|(shape_entity, _, _)| shape_entity == entity)
                        .expect("找不到有鬼");
                    current_shape_entity = shape_entity;
                    transform.translation = Vec3::new(
                        -window.width() / (scale.x * 2.0),
                        window.height() / (scale.y * 2.0),
                        z_index,
                    );
                    // 更新中间纹理变换
                    if let Some((
                        _,
                        mut intermediate_texture,
                        mut flash_filters,
                        mut aabb,
                        swf_shape_mesh_aabb,
                        material_handle,
                    )) = intermediate_textures
                        .iter_mut()
                        .find(|(entity, _, _, _, _, _)| *entity == shape_entity)
                    {
                        let Some(bitmap_material) = bitmap_materials.get_mut(material_handle.id())
                        else {
                            continue;
                        };

                        // TODO: 暂时解决了示例滤镜的z轴问题。
                        transform.translation.z += 0.2;

                        let (_, shape) = flash_asset.shape_meshes.get(&id).expect("没有就是有Bug");
                        let scale = global_transform.scale();
                        let bounds = matrix * shape.shape_bounds.clone();
                        let size = get_graphic_raw_size(&bounds, scale);
                        let filter_rect = get_filter_rect(&bounds, filters, scale);
                        let width = filter_rect.width() as f32;
                        let height = filter_rect.height() as f32;
                        let filter_size =
                            UVec2::new((width * scale.x) as u32, (height * scale.y) as u32);
                        let tx = matrix.tx.to_pixels() as f32;
                        let ty = matrix.ty.to_pixels() as f32;
                        let offset_x = bounds.x_min.to_pixels() as f32 - tx;
                        let offset_y = bounds.y_min.to_pixels() as f32 - ty;
                        let world_transform = Mat4::from_cols_array_2d(&[
                            [matrix.a, matrix.b, 0.0, 0.0],
                            [matrix.c, matrix.d, 0.0, 0.0],
                            [0.0, 0.0, 1.0, 0.0],
                            [-offset_x, -offset_y, 0.0, 1.0],
                        ]);
                        let image = get_target_image(&filter_size);
                        let image_handle = images.add(image);

                        flash_filters.clear();
                        flash_filters.append(filters);
                        intermediate_texture.is_active = true;
                        intermediate_texture.target = image_handle.clone().into();
                        intermediate_texture.filter_size = filter_size;
                        intermediate_texture.world_transform = world_transform;
                        intermediate_texture.size = size;
                        intermediate_texture.scale = global_transform.scale();

                        let draw_offset =
                            Vec2::new(filter_rect.x_min as f32, filter_rect.y_min as f32);
                        let world_transform = Mat4::from_cols_array_2d(&[
                            [width, 0.0, 0.0, 0.0],
                            [0.0, height, 0.0, 0.0],
                            [0.0, 0.0, 1.0, 0.0],
                            [
                                tx + offset_x + draw_offset.x,
                                ty + offset_y + draw_offset.y,
                                0.0,
                                1.0,
                            ],
                        ]);

                        let swf_transform = SwfTransform {
                            world_transform,
                            ..swf_transform
                        };

                        //  更新AABB
                        let aabb_transform = Mat4::from_cols_array_2d(&[
                            [1.0, 0.0, 0.0, 0.0],
                            [0.0, -1.0, 0.0, 0.0],
                            [0.0, 0.0, 1.0, 0.0],
                            [1.0, 1.0, 0.0, 1.0],
                        ]) * swf_transform.world_transform;
                        aabb.center = aabb_transform.transform_point3a(swf_shape_mesh_aabb.center);
                        aabb.half_extents = Vec3A::new(width / 2.0, height / 2.0, 0.0);

                        bitmap_material.texture = image_handle;
                        bitmap_material.update_swf_material(swf_transform);
                    } else {
                        let Some(shape_children) = shape_children else {
                            continue;
                        };
                        shape_children.iter().for_each(|child| {
                            for (
                                material_entity,
                                mut aabb,
                                swf_color_material_handle,
                                swf_gradient_material_handle,
                                swf_bitmap_material_handle,
                                swf_shape_mesh_aabb,
                            ) in flash_material_query.iter_mut()
                            {
                                if material_entity == *child {
                                    //  更新AABB
                                    let aabb_transform = Mat4::from_cols_array_2d(&[
                                        [1.0, 0.0, 0.0, 0.0],
                                        [0.0, -1.0, 0.0, 0.0],
                                        [0.0, 0.0, 1.0, 0.0],
                                        [1.0, 1.0, 0.0, 1.0],
                                    ]) * swf_transform.world_transform;
                                    aabb.center = aabb_transform
                                        .transform_point3a(swf_shape_mesh_aabb.center);
                                    if let Some(handle) = swf_color_material_handle {
                                        update_swf_material(
                                            handle,
                                            &mut swf_color_materials,
                                            swf_transform.clone(),
                                            blend,
                                        );
                                        break;
                                    }
                                    if let Some(handle) = swf_gradient_material_handle {
                                        update_swf_material(
                                            handle,
                                            &mut gradient_materials,
                                            swf_transform.clone(),
                                            blend,
                                        );
                                        break;
                                    }
                                    if let Some(handle) = swf_bitmap_material_handle {
                                        update_swf_material(
                                            handle,
                                            &mut bitmap_materials,
                                            swf_transform.clone(),
                                            blend,
                                        );
                                        break;
                                    }
                                }
                            }
                        });
                    }
                } else {
                    // 不存在缓存实体
                    let mut shape_entity_command = commands.spawn((
                        SwfGraph,
                        // 将flash的画布原点移动到WebGPU原点
                        Transform::from_translation(Vec3::new(
                            -window.width() / (scale.x * 2.0),
                            window.height() / (scale.y * 2.0),
                            z_index,
                        )),
                    ));
                    let shape_entity = shape_entity_command.id();

                    let (shape_meshes, shape) =
                        flash_asset.shape_meshes.get(&id).expect("没有就是有Bug");
                    // 是否含有滤镜效果
                    if !filters.is_empty() {
                        // 用于渲染出中间纹理的数据
                        let mut view_entities = Vec::new();
                        shape_meshes.iter().for_each(|shape| {
                            let mesh = shape.mesh.clone();
                            view_entities.push(SwfVertex {
                                indices: mesh.indices,
                                pipeline_id: CachedRenderPipelineId::INVALID,
                                mesh_draw_type: shape_to_intermediate_texture_draw_type(
                                    &shape.draw_type,
                                    &mesh.positions,
                                    &mesh.colors,
                                ),
                            });
                        });
                        let scale = global_transform.scale();
                        let bounds = matrix * shape.shape_bounds.clone();
                        let size = get_graphic_raw_size(&bounds, scale);
                        let filter_rect = get_filter_rect(&bounds, filters, scale);
                        let width = filter_rect.width() as f32;
                        let height = filter_rect.height() as f32;
                        let filter_size =
                            UVec2::new((width * scale.x) as u32, (height * scale.y) as u32);

                        let tx = matrix.tx.to_pixels() as f32;
                        let ty = matrix.ty.to_pixels() as f32;
                        let offset_x = bounds.x_min.to_pixels() as f32 - tx;
                        let offset_y = bounds.y_min.to_pixels() as f32 - ty;
                        let world_transform = Mat4::from_cols_array_2d(&[
                            [matrix.a, matrix.b, 0.0, 0.0],
                            [matrix.c, matrix.d, 0.0, 0.0],
                            [0.0, 0.0, 1.0, 0.0],
                            [-offset_x, -offset_y, 0.0, 1.0],
                        ]);
                        let image = get_target_image(&filter_size);
                        let image_handle = images.add(image);
                        shape_entity_command.insert((
                            IntermediateTexture {
                                target: image_handle.clone().into(),
                                is_active: true,
                                scale,
                                size,
                                filter_size,
                                view_entities,
                                world_transform,
                            },
                            FlashFilters(filters.clone()),
                        ));
                        let draw_offset =
                            Vec2::new(filter_rect.x_min as f32, filter_rect.y_min as f32);
                        let world_transform = Mat4::from_cols_array_2d(&[
                            [width, 0.0, 0.0, 0.0],
                            [0.0, height, 0.0, 0.0],
                            [0.0, 0.0, 1.0, 0.0],
                            [
                                tx + offset_x + draw_offset.x,
                                ty + offset_y + draw_offset.y,
                                0.0,
                                1.0,
                            ],
                        ]);
                        let swf_transform = SwfTransform {
                            world_transform,
                            ..swf_transform
                        };
                        let (mesh, texture_transform) =
                            generate_rectangle_mesh_and_texture_transform();

                        let aabb = mesh.compute_aabb().unwrap_or_default();
                        shape_entity_command.insert((
                            Mesh2d(meshes.add(mesh)),
                            MeshMaterial2d(bitmap_materials.add(BitmapMaterial {
                                alpha_mode2d: blend,
                                texture: image_handle.clone(),
                                texture_transform,
                                transform: swf_transform.clone(),
                            })),
                            SwfShapeMeshAabb(aabb),
                            Transform::from_translation(Vec3::new(
                                -window.width() / (scale.x * 2.0),
                                window.height() / (scale.y * 2.0),
                                z_index,
                            )),
                        ));
                    } else {
                        // 生成网格实体
                        shape_meshes.iter().for_each(|shape_mesh| {
                            // 防止Shape中的绘制z冲突
                            z_index += 1e-3;
                            let swf_mesh = shape_mesh.mesh.clone();
                            let mut mesh = Mesh::new(
                                PrimitiveTopology::TriangleList,
                                RenderAssetUsages::default(),
                            );
                            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, swf_mesh.positions);
                            mesh.insert_indices(Indices::U32(swf_mesh.indices));

                            let swf_shape_mesh_aabb =
                                SwfShapeMeshAabb(mesh.compute_aabb().unwrap_or_default());

                            let transform =
                                Transform::from_translation(Vec3::new(0.0, 0.0, z_index));
                            match &shape_mesh.draw_type {
                                ShapeDrawType::Color(swf_color_material) => {
                                    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, swf_mesh.colors);
                                    spawn_mesh(
                                        &mut shape_entity_command,
                                        swf_color_material.clone(),
                                        &mut swf_color_materials,
                                        swf_transform.clone(),
                                        transform,
                                        meshes.add(mesh),
                                        swf_shape_mesh_aabb,
                                        blend,
                                    );
                                }
                                ShapeDrawType::Gradient(gradient_material) => {
                                    spawn_mesh(
                                        &mut shape_entity_command,
                                        gradient_material.clone(),
                                        &mut gradient_materials,
                                        swf_transform.clone(),
                                        transform,
                                        meshes.add(mesh),
                                        swf_shape_mesh_aabb,
                                        blend,
                                    );
                                }
                                ShapeDrawType::Bitmap(bitmap_material) => {
                                    spawn_mesh(
                                        &mut shape_entity_command,
                                        bitmap_material.clone(),
                                        &mut bitmap_materials,
                                        swf_transform.clone(),
                                        transform,
                                        meshes.add(mesh),
                                        swf_shape_mesh_aabb,
                                        blend,
                                    );
                                }
                            }
                        });
                    }

                    current_shape_entity = shape_entity;
                    flash_shape_record.mark_cached_shape(
                        active_instance.id(),
                        *ref_count,
                        shape_entity,
                    );
                    commands.entity(entity).add_child(shape_entity);
                }
                current_shape_entities.push(current_shape_entity);
            }

            // 每帧隐藏所有实体
            flash_shape_record
                .cache_entities()
                .iter()
                .for_each(|(_, entity)| {
                    commands.entity(*entity).insert(Visibility::Hidden);
                });
            // 将当前帧所含有的实体设置为可见
            current_shape_entities.iter().for_each(|entity| {
                commands.entity(*entity).insert(Visibility::Inherited);
            });
        }
    }
}

pub fn extract_intermediate_phase(
    query: Extract<Query<(RenderEntity, &IntermediateTexture)>>,
    mut render_phases: ResMut<RenderPhases>,
    mut live_entities: Local<HashSet<MainEntity>>,
) {
    live_entities.clear();
    for (main_entity, intermediate_texture) in query.iter() {
        if !intermediate_texture.is_active {
            continue;
        }
        render_phases.insert_or_clear(main_entity.into());
        live_entities.insert(main_entity.into());
    }
    render_phases.retain(|entity, _| live_entities.contains(entity));
}

pub fn queue_swf_vertex(
    mut query: Query<(Entity, &mut ExtractedIntermediateTexture)>,
    mut render_phases: ResMut<RenderPhases>,
) {
    for (entity, mut intermediate_texture) in query.iter_mut() {
        let main_entity: MainEntity = entity.into();
        let Some(render_phase) = render_phases.get_mut(&main_entity) else {
            continue;
        };
        render_phase.append(&mut intermediate_texture.view_entities);
    }
}

pub fn prepare_view_attachments(
    images: Res<RenderAssets<GpuImage>>,
    intermediate_textures: Query<&ExtractedIntermediateTexture>,
    mut view_target_attachments: ResMut<ViewTargetAttachments>,
) {
    for intermediate_texture in intermediate_textures.iter() {
        let Some(target) = &intermediate_texture.target else {
            continue;
        };
        match view_target_attachments.entry(target.clone()) {
            Entry::Occupied(_) => {}
            Entry::Vacant(entry) => {
                if let (Some(texture_view), Some(texture_format)) = match target {
                    bevy::render::camera::NormalizedRenderTarget::Window(_) => {
                        unreachable!("请使用Image作为中间纹理引用")
                    }
                    bevy::render::camera::NormalizedRenderTarget::Image(image_target) => {
                        let gpu_image = images.get(&image_target.handle);
                        (
                            gpu_image.map(|image| &image.texture_view),
                            gpu_image.map(|image| image.texture_format),
                        )
                    }
                    bevy::render::camera::NormalizedRenderTarget::TextureView(_) => {
                        unreachable!("请使用Image作为中间纹理引用")
                    }
                } {
                    entry.insert(OutputColorAttachment::new(
                        texture_view.clone(),
                        texture_format.add_srgb_suffix(),
                    ));
                } else {
                    continue;
                }
            }
        }
    }
}

pub fn prepare_intermediate_texture_view_targets(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    mut texture_cache: ResMut<TextureCache>,
    query: Query<(Entity, &ExtractedIntermediateTexture)>,
    view_target_attachments: Res<ViewTargetAttachments>,
) {
    let mut textures = <HashMap<_, _>>::default();
    for (entity, intermediate_texture) in query.iter() {
        let Some(target) = &intermediate_texture.target else {
            continue;
        };
        let Some(out_attachment) = view_target_attachments.get(target) else {
            continue;
        };
        let size = Extent3d {
            width: intermediate_texture.filter_size.x,
            height: intermediate_texture.filter_size.y,
            depth_or_array_layers: 1,
        };
        let main_texture_format = TextureFormat::bevy_default();
        let (a, b, sampled, main_texture) = textures
            .entry((intermediate_texture.filter_size, entity))
            .or_insert_with(|| {
                let descriptor = TextureDescriptor {
                    label: Some("intermediate_texture_id:"),
                    size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: TextureDimension::D2,
                    format: main_texture_format,
                    usage: TextureUsages::RENDER_ATTACHMENT
                        | TextureUsages::COPY_SRC
                        | TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                };

                let a = texture_cache.get(
                    &render_device,
                    TextureDescriptor {
                        label: Some("main_texture_a"),
                        ..descriptor
                    },
                );
                let b = texture_cache.get(
                    &render_device,
                    TextureDescriptor {
                        label: Some("main_texture_b"),
                        ..descriptor
                    },
                );
                let sampled = if Msaa::default().samples() > 1 {
                    let sampled = texture_cache.get(
                        &render_device,
                        TextureDescriptor {
                            label: Some("main_texture_sampled"),
                            size,
                            mip_level_count: 1,
                            sample_count: Msaa::default().samples(),
                            dimension: TextureDimension::D2,
                            format: main_texture_format,
                            usage: TextureUsages::RENDER_ATTACHMENT,
                            view_formats: descriptor.view_formats,
                        },
                    );
                    Some(sampled)
                } else {
                    None
                };
                let main_texture = Arc::new(AtomicUsize::new(0));
                (a, b, sampled, main_texture)
            });
        let main_textures = MainTargetTextures {
            a: ColorAttachment::new(a.clone(), sampled.clone(), Some(LinearRgba::NONE)),
            b: ColorAttachment::new(b.clone(), sampled.clone(), Some(LinearRgba::NONE)),
            main_texture: main_texture.clone(),
        };
        commands.entity(entity).insert(ViewTarget {
            main_texture: main_textures.main_texture.clone(),
            main_textures,
            main_texture_format,
            out_texture: out_attachment.clone(),
        });
    }
}

pub fn extract_intermediate_texture(
    mut commands: Commands,
    intermediate_textures: Extract<
        Query<(RenderEntity, &IntermediateTexture, &FlashFilterRenderGraph)>,
    >,
) {
    for (render_entity, intermediate_texture, render_graph) in intermediate_textures.iter() {
        if !intermediate_texture.is_active {
            commands
                .entity(render_entity)
                .remove::<ExtractedIntermediateTexture>();
            continue;
        }
        commands
            .entity(render_entity)
            .insert(ExtractedIntermediateTexture {
                target: intermediate_texture.target.normalize(None),
                scale: intermediate_texture.scale,
                size: intermediate_texture.size,
                filter_size: intermediate_texture.filter_size,
                view_entities: intermediate_texture.view_entities.clone(),
                world_transform: intermediate_texture.world_transform,
                render_graph: render_graph.0,
            });
    }
}

#[derive(Clone)]
pub struct SwfVertex {
    pub indices: Vec<u32>,
    pub mesh_draw_type: MeshDrawType,
    pub pipeline_id: CachedRenderPipelineId,
}

#[derive(Clone, Default)]
pub struct Gradient {
    pub vertex: Vec<Vertex>,
    pub gradient: GradientUniforms,
    pub texture: Handle<Image>,
    pub texture_transform: Mat4,
}

#[derive(Clone)]
pub enum MeshDrawType {
    Color(Vec<VertexColor>),
    Gradient(Gradient),
    Bitmap,
}
impl Default for MeshDrawType {
    fn default() -> Self {
        MeshDrawType::Color(Default::default())
    }
}

fn shape_to_intermediate_texture_draw_type(
    draw_type: &ShapeDrawType,
    position: &Vec<[f32; 3]>,
    colors: &Vec<[f32; 4]>,
) -> MeshDrawType {
    let draw_type = draw_type.clone();
    match draw_type {
        ShapeDrawType::Color(_) => MeshDrawType::Color(
            position
                .iter()
                .zip(colors.iter())
                .map(|(position, color)| VertexColor::new(*position, *color))
                .collect(),
        ),
        ShapeDrawType::Gradient(gradient_material) => MeshDrawType::Gradient(Gradient {
            vertex: position
                .iter()
                .map(|position| Vertex::new(*position))
                .collect(),
            texture: gradient_material
                .texture
                .clone()
                .expect("渐变纹理应该已经生成！！！"),
            gradient: gradient_material.gradient,
            texture_transform: gradient_material.texture_transform,
        }),
        ShapeDrawType::Bitmap(_) => MeshDrawType::Bitmap,
    }
}

#[inline]
fn update_swf_material<T: SwfMaterial>(
    handle: &Handle<T>,
    swf_materials: &mut ResMut<Assets<T>>,
    swf_transform: SwfTransform,
    alpha_mode2d: AlphaMode2d,
) {
    // 当缓存某实体后该实体在该系统尚未运行完成时会查询不到对应的材质，此时重新生成材质。
    if let Some(swf_material) = swf_materials.get_mut(handle) {
        swf_material.update_swf_material(swf_transform);
        swf_material.set_alpha_mode2d(alpha_mode2d);
    }
}

#[inline]
#[allow(clippy::too_many_arguments)]
fn spawn_mesh<T: SwfMaterial>(
    commands: &mut EntityCommands,
    mut swf_material: T,
    swf_materials: &mut ResMut<Assets<T>>,
    swf_transform: SwfTransform,
    transform: Transform,
    handle: Handle<Mesh>,
    swf_shape_mesh_aabb: SwfShapeMeshAabb,
    alpha_mode2d: AlphaMode2d,
) {
    swf_material.update_swf_material(swf_transform);
    swf_material.set_alpha_mode2d(alpha_mode2d);
    commands.with_children(|parent| {
        parent.spawn((
            Mesh2d(handle),
            MeshMaterial2d(swf_materials.add(swf_material)),
            transform,
            swf_shape_mesh_aabb,
            SwfShapeChildMesh,
        ));
    });
}

fn get_graphic_raw_size(bounds: &SwfRectangle<Twips>, global_scale: Vec3) -> UVec2 {
    let width = bounds.width().to_pixels().ceil().max(0.0) as f32;
    let height = bounds.height().to_pixels().ceil().max(0.0) as f32;
    let width = (width * global_scale.x) as u32;
    let height = (height * global_scale.y) as u32;
    UVec2::new(width, height)
}

fn get_filter_rect(
    bounds: &SwfRectangle<Twips>,
    filters: &mut Vec<Filter>,
    global_scale: Vec3,
) -> SwfRectangle<i32> {
    let scale_x = global_scale.x;
    let scale_y = global_scale.y;

    let width = bounds.width().to_pixels().ceil().max(0.0);
    let height = bounds.height().to_pixels().ceil().max(0.0);
    let mut filter_rect = SwfRectangle {
        x_min: Twips::ZERO,
        x_max: Twips::from_pixels_i32(width as i32),
        y_min: Twips::ZERO,
        y_max: Twips::from_pixels_i32(height as i32),
    };
    for filter in filters {
        filter.scale(scale_x, scale_y);
        filter_rect = filter.calculate_dest_rect(filter_rect);
    }

    let filter_rect = SwfRectangle {
        x_min: filter_rect.x_min.to_pixels().floor() as i32,
        x_max: filter_rect.x_max.to_pixels().ceil() as i32,
        y_min: filter_rect.y_min.to_pixels().floor() as i32,
        y_max: filter_rect.y_max.to_pixels().ceil() as i32,
    };

    filter_rect
}

fn get_target_image(filter_rect: &UVec2) -> Image {
    let size = Extent3d {
        width: filter_rect.x,
        height: filter_rect.y,
        depth_or_array_layers: 1,
    };

    let mut image = Image::new_uninit(
        size,
        TextureDimension::D2,
        TextureFormat::bevy_default(),
        RenderAssetUsages::default(),
    );
    image.texture_descriptor.usage =
        TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST | TextureUsages::RENDER_ATTACHMENT;
    image
}

fn generate_rectangle_mesh_and_texture_transform() -> (Mesh, Mat4) {
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ],
    );
    mesh.insert_indices(Indices::U32(vec![0, 1, 2, 0, 2, 3]));
    let texture_transform = Mat4::from_cols_array_2d(&[
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]);
    (mesh, texture_transform)
}
