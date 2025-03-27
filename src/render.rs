use std::{collections::BTreeMap, sync::Arc};

use bevy::asset::{RenderAssetUsages, weak_handle};
use bevy::color::LinearRgba;
use bevy::ecs::resource::Resource;
use bevy::ecs::schedule::IntoScheduleConfigs;
use bevy::ecs::system::EntityCommands;
use bevy::image::{BevyDefault, Image};
use bevy::math::{UVec2, Vec2};
use bevy::platform_support::collections::HashMap;
use bevy::render::extract_component::{ExtractComponent, ExtractComponentPlugin};
use bevy::render::graph::CameraDriverLabel;
use bevy::render::mesh::{Indices, MeshAabb, PrimitiveTopology};
use bevy::render::render_graph::RenderGraph;
use bevy::render::render_resource::{
    Extent3d, SpecializedRenderPipelines, TextureDescriptor, TextureDimension, TextureFormat,
    TextureUsages,
};
use bevy::render::renderer::RenderDevice;
use bevy::render::sync_world::MainEntity;
use bevy::render::texture::{ColorAttachment, TextureCache};
use bevy::render::view::Msaa;
use bevy::render::{Render, RenderSet};
use bevy::sprite::AlphaMode2d;
use bevy::transform::components::GlobalTransform;
use bevy::window::Window;
use bevy::{
    app::{App, Plugin, PostUpdate, Update},
    asset::{Assets, Handle, load_internal_asset},
    math::{Mat4, Vec3},
    prelude::{
        Children, Commands, Component, Entity, Mesh, Mesh2d, Query, Res, ResMut, Shader, Transform,
        Visibility, With, Without,
    },
    render::{
        RenderApp,
        view::{NoFrustumCulling, VisibilitySystems},
    },
    sprite::{Material2dPlugin, MeshMaterial2d},
};
use blend_pipeline::{BlendType, TrivialBlend};
use filter::{BLUR_FILTER_SHADER_HANDLE, Filter};
use intermediate_texture_driver_node::{
    IntermediateTextureDriverLabel, IntermediateTextureDriverNode, Vertex, VertexColor,
};
use material::{
    BitmapMaterial, GradientMaterial, GradientUniforms, SwfColorMaterial, SwfMaterial, SwfTransform,
};
use pipeline::{
    INTERMEDIATE_TEXTURE_GRADIENT, INTERMEDIATE_TEXTURE_MESH, IntermediateRenderPhases,
    IntermediateTexturePipeline, specialize_meshes,
};
use ruffle_render::transform::Transform as RuffleTransform;
use swf::{Rectangle as SwfRectangle, Twips};

use crate::ShapeDrawType;
use crate::assets::SwfMovie;
use crate::{
    bundle::{FlashAnimation, ShapeMark, ShapeMarkEntities, SwfGraph, SwfState},
    swf::display_object::{DisplayObject, TDisplayObject},
};

pub(crate) mod blend_pipeline;
pub(crate) mod filter;
mod intermediate_texture_driver_node;
pub(crate) mod material;
mod pipeline;
pub(crate) mod tessellator;

pub const SWF_COLOR_MATERIAL_SHADER_HANDLE: Handle<Shader> =
    weak_handle!("8c2a5b0f-3e6d-4f8a-b217-84d2f5e1c9b3");
pub const GRADIENT_MATERIAL_SHADER_HANDLE: Handle<Shader> =
    weak_handle!("5e9f1a78-9b34-4c15-8d7e-2a3b0f47d862");
pub const BITMAP_MATERIAL_SHADER_HANDLE: Handle<Shader> =
    weak_handle!("a34c7d82-1f5b-4a9e-93d8-6b7e20c45a1f");
pub const FLASH_COMMON_MATERIAL_SHADER_HANDLE: Handle<Shader> =
    weak_handle!("e53b9f82-6a4c-4d5b-91e7-4f2a63b8c5d9");

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

        app.add_plugins(Material2dPlugin::<GradientMaterial>::default())
            .add_plugins(Material2dPlugin::<SwfColorMaterial>::default())
            .add_plugins(Material2dPlugin::<BitmapMaterial>::default())
            .add_plugins(ExtractComponentPlugin::<IntermediateTexture>::default())
            .add_plugins(ExtractComponentPlugin::<SwfVertex>::default())
            .add_systems(Update, generate_swf_mesh)
            .add_systems(
                PostUpdate,
                (
                    calculate_shape_bounds.in_set(VisibilitySystems::CalculateBounds),
                    collect_intermediate_texture_children,
                ),
            );

        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<IntermediateTextures>()
            .init_resource::<IntermediateRenderPhases>()
            .init_resource::<SpecializedRenderPipelines<IntermediateTexturePipeline>>()
            .add_systems(
                Render,
                (
                    prepare_intermediate_texture.in_set(RenderSet::PrepareResources),
                    specialize_meshes.in_set(RenderSet::PrepareAssets),
                ),
            );
        let intermediate_texture_driver_node =
            IntermediateTextureDriverNode::new(render_app.world_mut());
        let mut render_graph = render_app.world_mut().resource_mut::<RenderGraph>();
        render_graph.add_node(
            IntermediateTextureDriverLabel,
            intermediate_texture_driver_node,
        );
        render_graph.add_node_edge(IntermediateTextureDriverLabel, CameraDriverLabel);
    }

    fn finish(&self, app: &mut App) {
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app.init_resource::<IntermediateTexturePipeline>();
        }
    }
}

type SwfShapeMeshQuery = (
    Entity,
    &'static mut Transform,
    Option<&'static MeshMaterial2d<SwfColorMaterial>>,
    Option<&'static MeshMaterial2d<GradientMaterial>>,
    Option<&'static MeshMaterial2d<BitmapMaterial>>,
    &'static mut SwfShapeMesh,
);

/// 用于记录Swf Transform 的矩阵变换，从而计算正确的Aabb，防止被剔除
#[derive(Component, Default)]
pub struct SwfShapeMesh {
    transform: Mat4,
}

/// 用于需要进行滤镜处理得到中间纹理
#[derive(Component, Default, Clone, ExtractComponent)]
pub struct IntermediateTexture {
    /// 当前帧是否渲染
    is_draw: bool,
    /// 全局变换缩放倍数，用于矢量缩放
    scale: Vec2,
    /// Graphic 原始bunds大小
    size: UVec2,
    /// 应用滤镜后的bunds大小
    filter_rect: UVec2,
    /// 中间纹理包含的子实体（`Mesh2d`）
    children: Vec<Entity>,
    /// swf 的world transform 变换，保证矢量缩放、旋转。
    world_transform: Mat4,
}

pub fn collect_intermediate_texture_children(
    mut query: Query<(&mut IntermediateTexture, &Children)>,
) {
    query.iter_mut().for_each(|(mut parent, children)| {
        parent.children = children.to_vec();
    });
}

#[allow(clippy::too_many_arguments)]
pub fn generate_swf_mesh(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut swf_movies: ResMut<Assets<SwfMovie>>,
    mut color_materials: ResMut<Assets<SwfColorMaterial>>,
    mut gradient_materials: ResMut<Assets<GradientMaterial>>,
    mut bitmap_materials: ResMut<Assets<BitmapMaterial>>,
    mut query: Query<(Entity, &mut FlashAnimation, &GlobalTransform)>,
    mut entities_material_query: Query<SwfShapeMeshQuery>,
    mut graphic_query: Query<(Entity, &Children, Option<&mut IntermediateTexture>), With<SwfGraph>>,
) {
    for (entity, mut flash_animation, global_transform) in query.iter_mut() {
        match flash_animation.status {
            SwfState::Loading => {
                continue;
            }
            SwfState::Ready => {
                flash_animation
                    .shape_mark_entities
                    .clear_current_frame_entity();
                if let Some(swf_movie) = swf_movies.get_mut(flash_animation.swf_movie.id()) {
                    let render_list = swf_movie.movie_clip.raw_container().render_list();
                    let parent_clip_transform = swf_movie.movie_clip.base().transform().clone();
                    let display_objects = swf_movie
                        .movie_clip
                        .raw_container_mut()
                        .display_objects_mut();

                    graphic_query
                        .iter_mut()
                        .for_each(|(_, _, intermediate_texture)| {
                            if let Some(mut intermediate_texture) = intermediate_texture {
                                intermediate_texture.is_draw = false
                            }
                        });
                    let mut z_index = 0.0;
                    exec_render_list(
                        entity,
                        global_transform,
                        &mut graphic_query,
                        &mut commands,
                        &mut meshes,
                        &mut images,
                        &mut color_materials,
                        &mut gradient_materials,
                        &mut bitmap_materials,
                        &mut entities_material_query,
                        &mut flash_animation.shape_mark_entities,
                        render_list,
                        display_objects,
                        &parent_clip_transform,
                        &mut z_index,
                        BlendType::Trivial(TrivialBlend::Normal),
                        Vec::new(),
                    );
                    // 每帧隐藏所有实体
                    flash_animation
                        .shape_mark_entities
                        .graphic_entities()
                        .iter()
                        .for_each(|(_, entity)| {
                            commands.entity(*entity).insert(Visibility::Hidden);
                        });
                    // 将当前帧所含有的实体设置为可见
                    flash_animation
                        .shape_mark_entities
                        .current_frame_entities()
                        .iter()
                        .for_each(|shape_mark| {
                            let entity = flash_animation
                                .shape_mark_entities
                                .entity(shape_mark)
                                .unwrap();
                            commands.entity(*entity).insert(Visibility::Inherited);
                        });
                    flash_animation.status = SwfState::Loading;
                }
            }
        }
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct TextureCacheInfo {
    filter_rect: UVec2,
    main_entity: MainEntity,
}

#[derive(Resource, Default)]
pub struct IntermediateTextures(HashMap<TextureCacheInfo, ColorAttachment>);

pub fn prepare_intermediate_texture(
    render_device: Res<RenderDevice>,
    mut texture_cache: ResMut<TextureCache>,
    mut intermediate_textures: ResMut<IntermediateTextures>,
    query: Query<(MainEntity, &IntermediateTexture)>,
) {
    for (entity, intermediate_texture) in query.iter() {
        let descriptor = TextureDescriptor {
            label: Some("intermediate_texture_id:"),
            size: Extent3d {
                width: intermediate_texture.filter_rect.x,
                height: intermediate_texture.filter_rect.y,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: Msaa::default().samples(),
            dimension: TextureDimension::D2,
            format: TextureFormat::bevy_default(),
            usage: TextureUsages::RENDER_ATTACHMENT
                | TextureUsages::COPY_SRC
                | TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        };
        let cache_texture = texture_cache.get(&render_device, descriptor);

        intermediate_textures
            .0
            .entry(TextureCacheInfo {
                filter_rect: intermediate_texture.filter_rect,
                main_entity: entity.into(),
            })
            .or_insert(ColorAttachment::new(
                cache_texture,
                None,
                Some(LinearRgba::NONE),
            ));
    }
}

#[derive(Component, ExtractComponent, Clone, Default)]
pub struct SwfVertex {
    pub indices: Vec<u32>,
    pub mesh_draw_type: MeshDrawType,
}

#[derive(Clone, Default)]
pub struct Gradient {
    pub vertex: Vec<Vertex>,
    pub gradient: GradientUniforms,
    pub texture: Handle<Image>,
    pub texture_transform: Mat4,
}
// #[derive(Clone, Default)]
// pub struct Bitmap {
//     pub vertex: Vec<Vertex>,
//     pub texture: Handle<Image>,
//     pub texture_transform: Mat4,
// }

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

#[allow(clippy::too_many_arguments)]
pub fn exec_render_list(
    parent_entity: Entity,
    global_transform: &GlobalTransform,
    graphic_query: &mut Query<
        '_,
        '_,
        (Entity, &Children, Option<&mut IntermediateTexture>),
        With<SwfGraph>,
    >,
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    images: &mut ResMut<Assets<Image>>,
    color_materials: &mut ResMut<Assets<SwfColorMaterial>>,
    gradient_materials: &mut ResMut<Assets<GradientMaterial>>,
    bitmap_materials: &mut ResMut<Assets<BitmapMaterial>>,
    entities_material_query: &mut Query<'_, '_, SwfShapeMeshQuery>,
    shape_mark_entities: &mut ShapeMarkEntities,
    render_list: Arc<Vec<u128>>,
    display_objects: &BTreeMap<u128, DisplayObject>,
    parent_clip_transform: &RuffleTransform,
    z_index: &mut f32,
    blend_type: BlendType,
    mut filters: Vec<Filter>,
) {
    for display_object in render_list.iter() {
        if let Some(display_object) = display_objects.get(display_object) {
            match display_object {
                DisplayObject::Graphic(graphic) => {
                    let current_transform = graphic.base().transform();
                    let matrix = parent_clip_transform.matrix * current_transform.matrix;
                    let swf_transform: SwfTransform = RuffleTransform {
                        matrix,
                        color_transform: parent_clip_transform.color_transform
                            * current_transform.color_transform,
                    }
                    .into();
                    // 记录当前帧生成的graphic实体
                    let mut shape_mark = ShapeMark {
                        graphic_ref_count: 1,
                        depth: graphic.depth(),
                        id: graphic.character_id(),
                    };
                    while shape_mark_entities
                        .current_frame_entities()
                        .iter()
                        .any(|x| *x == shape_mark)
                    {
                        shape_mark.graphic_ref_count += 1;
                    }
                    *z_index += graphic.depth() as f32 / 100.0;
                    if let Some(&existing_entity) = shape_mark_entities.entity(&shape_mark) {
                        // 如果存在缓存实体
                        if let Some((_, graphic_children, intermediate_texture)) = graphic_query
                            .iter_mut()
                            .find(|(entity, _, _)| *entity == existing_entity)
                        {
                            if let Some(mut intermediate_texture) = intermediate_texture {
                                intermediate_texture.is_draw = true;
                            }
                            graphic_children.iter().for_each(|child| {
                                for (
                                    material_entity,
                                    mut transform,
                                    swf_color_material_handle,
                                    swf_gradient_material_handle,
                                    swf_bitmap_material_handle,
                                    mut swf_shape_mesh,
                                ) in entities_material_query.iter_mut()
                                {
                                    if material_entity == *child {
                                        *z_index += 0.001;
                                        transform.translation.z = *z_index;
                                        if let Some(handle) = swf_color_material_handle {
                                            update_swf_material(
                                                (handle, swf_shape_mesh.as_mut()),
                                                color_materials,
                                                swf_transform.clone(),
                                            );
                                            break;
                                        }
                                        if let Some(handle) = swf_gradient_material_handle {
                                            update_swf_material(
                                                (handle, swf_shape_mesh.as_mut()),
                                                gradient_materials,
                                                swf_transform.clone(),
                                            );
                                            break;
                                        }
                                        if let Some(handle) = swf_bitmap_material_handle {
                                            update_swf_material(
                                                (handle, swf_shape_mesh.as_mut()),
                                                bitmap_materials,
                                                swf_transform.clone(),
                                            );
                                            break;
                                        }
                                    }
                                }
                            });
                        }
                    } else {
                        // 不存在缓存实体
                        let mut graphic_entity_command = commands.spawn(SwfGraph);
                        let graphic_entity = graphic_entity_command.id();
                        shape_mark_entities.add_entities_pool(shape_mark, graphic_entity);

                        if !filters.is_empty() {
                            let scale = global_transform.scale();

                            let bounds = matrix * graphic.bounds.clone();
                            let filter_rect = get_filter_rect(&bounds, &mut filters, scale);

                            let offset_x =
                                bounds.x_min.to_pixels() as f32 - matrix.tx.to_pixels() as f32;
                            let offset_y =
                                bounds.y_min.to_pixels() as f32 - matrix.ty.to_pixels() as f32;
                            let world_transform = Mat4::from_cols_array_2d(&[
                                [matrix.a, matrix.b, 0.0, 0.0],
                                [matrix.c, matrix.d, 0.0, 0.0],
                                [0.0, 0.0, 1.0, 0.0],
                                [-offset_x, -offset_y, 0.0, 1.0],
                            ]);
                            graphic_entity_command.insert(IntermediateTexture {
                                is_draw: true,
                                scale: Vec2::new(scale.x, scale.y),
                                size: get_graphic_raw_size(&bounds, scale),
                                filter_rect,
                                children: vec![],
                                world_transform,
                            });
                            graphic.shape_mesh().iter().for_each(|shape| {
                                let mesh = shape.mesh.clone();
                                graphic_entity_command.with_children(|parent| {
                                    parent.spawn(SwfVertex {
                                        indices: mesh.indices,
                                        mesh_draw_type: shape_to_intermediate_texture_draw_type(
                                            &shape.draw_type,
                                            &mesh.positions,
                                            &mesh.colors,
                                        ),
                                    });
                                });
                            });
                        } else {
                            graphic.shape_mesh().iter().for_each(|shape| {
                                let swf_mesh = shape.mesh.clone();
                                let mut mesh = Mesh::new(
                                    PrimitiveTopology::TriangleList,
                                    RenderAssetUsages::default(),
                                );
                                mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, swf_mesh.positions);
                                mesh.insert_indices(Indices::U32(swf_mesh.indices));
                                *z_index += 0.001;
                                let transform =
                                    Transform::from_translation(Vec3::new(0.0, 0.0, *z_index));
                                match &shape.draw_type {
                                    ShapeDrawType::Color(swf_color_material) => {
                                        mesh.insert_attribute(
                                            Mesh::ATTRIBUTE_COLOR,
                                            swf_mesh.colors,
                                        );
                                        spawn_mesh(
                                            &mut graphic_entity_command,
                                            swf_color_material.clone(),
                                            color_materials,
                                            swf_transform.clone(),
                                            transform,
                                            meshes.add(mesh),
                                            blend_type.clone().into(),
                                        );
                                    }
                                    ShapeDrawType::Gradient(gradient_material) => {
                                        spawn_mesh(
                                            &mut graphic_entity_command,
                                            gradient_material.clone(),
                                            gradient_materials,
                                            swf_transform.clone(),
                                            transform,
                                            meshes.add(mesh),
                                            blend_type.clone().into(),
                                        );
                                    }
                                    ShapeDrawType::Bitmap(bitmap_material) => {
                                        spawn_mesh(
                                            &mut graphic_entity_command,
                                            bitmap_material.clone(),
                                            bitmap_materials,
                                            swf_transform.clone(),
                                            transform,
                                            meshes.add(mesh),
                                            blend_type.clone().into(),
                                        );
                                    }
                                }
                            });
                        }
                        commands.entity(parent_entity).add_child(graphic_entity);
                    }
                    shape_mark_entities.record_current_frame_entity(shape_mark);
                }
                DisplayObject::MovieClip(movie_clip) => {
                    let current_transform = RuffleTransform {
                        matrix: parent_clip_transform.matrix * movie_clip.base().transform().matrix,
                        color_transform: parent_clip_transform.color_transform
                            * movie_clip.base().transform().color_transform,
                    };
                    let blend_type = BlendType::from(movie_clip.blend_mode());
                    let filters = movie_clip.filters();
                    exec_render_list(
                        parent_entity,
                        global_transform,
                        graphic_query,
                        commands,
                        meshes,
                        images,
                        color_materials,
                        gradient_materials,
                        bitmap_materials,
                        entities_material_query,
                        shape_mark_entities,
                        movie_clip.raw_container().render_list(),
                        movie_clip.raw_container().display_objects(),
                        &current_transform,
                        z_index,
                        blend_type,
                        filters,
                    );
                }
            }
        }
    }
}

#[inline]
fn update_swf_material<T: SwfMaterial>(
    exists_material: (&Handle<T>, &mut SwfShapeMesh),
    swf_materials: &mut ResMut<Assets<T>>,
    swf_transform: SwfTransform,
) {
    // 当缓存某实体后该实体在该系统尚未运行完成时会查询不到对应的材质，此时重新生成材质。
    if let Some(swf_material) = swf_materials.get_mut(exists_material.0) {
        let swf_shape_mesh = exists_material.1;
        swf_shape_mesh.transform = swf_transform.world_transform;
        swf_material.update_swf_material(swf_transform);
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
    alpha_mode2d: AlphaMode2d,
) {
    swf_material.update_swf_material(swf_transform);
    swf_material.set_alpha_mode2d(alpha_mode2d);
    let aabb_transform = swf_material.world_transform();
    commands.with_children(|parent| {
        parent.spawn((
            Mesh2d(handle),
            MeshMaterial2d(swf_materials.add(swf_material)),
            transform,
            SwfShapeMesh {
                transform: aabb_transform,
            },
        ));
    });
}

fn get_graphic_raw_size(bounds: &SwfRectangle<Twips>, global_scale: Vec3) -> UVec2 {
    let width = bounds.width().to_pixels().ceil().max(0.0) as f32;
    let height = bounds.height().to_pixels().ceil().max(0.0) as f32;
    let width = width * global_scale.x;
    let height = height * global_scale.y;
    UVec2::new(width as u32, height as u32)
}

fn get_filter_rect(
    bounds: &SwfRectangle<Twips>,
    filters: &mut Vec<Filter>,
    global_scale: Vec3,
) -> UVec2 {
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
    let width = filter_rect.width() as f32 * scale_x;
    let height = filter_rect.height() as f32 * scale_y;
    UVec2::new(width as u32, height as u32)
}

pub fn calculate_shape_bounds(
    mut commands: Commands,
    meshes: Res<Assets<Mesh>>,
    shape_meshes: Query<
        (Entity, &Mesh2d, &SwfShapeMesh, &GlobalTransform),
        Without<NoFrustumCulling>,
    >,
    query_window: Query<&Window>,
) {
    let mut calculate = |(entity, mesh_handle, swf_shape_mesh, global_transform): (
        Entity,
        &Mesh2d,
        &SwfShapeMesh,
        &GlobalTransform,
    ),
                         size: Vec2| {
        if let Some(mesh) = meshes.get(&mesh_handle.0) {
            if let Some(mut aabb) = mesh.compute_aabb() {
                let swf_transform = Mat4::from_cols_array_2d(&[
                    [1.0, 0.0, 0.0, 0.0],
                    [0.0, -1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                    [
                        size.x / (-2.0 * global_transform.scale().x.abs()),
                        size.y / (2.0 * global_transform.scale().x.abs()),
                        0.0,
                        1.0,
                    ],
                ]) * swf_shape_mesh.transform;
                aabb.center = swf_transform.transform_point3a(aabb.center);
                commands.entity(entity).try_insert(aabb);
            }
        }
    };
    shape_meshes.iter().for_each(|item| {
        if let Ok(window) = query_window.single() {
            calculate(item, window.size());
        }
    });
}
