pub(crate) mod blend_pipeline;
mod graph;
pub(crate) mod material;
pub(crate) mod offscreen_texture;
pub mod part_mesh2d;
mod pipeline;
mod texture_attachment;

use std::{hash::Hash, marker::PhantomData, usize};

use bevy::{
    app::{App, Plugin, PostUpdate},
    asset::{
        AssetApp, AssetEventSystems, AssetId, AssetServer, Assets, Handle, RenderAssetUsages,
        load_internal_asset,
    },
    camera::visibility::add_visibility_class,
    core_pipeline::core_2d::Transparent2d,
    ecs::{
        component::Tick,
        entity::Entity,
        lifecycle::RemovedComponents,
        query::{Changed, Or, With},
        resource::Resource,
        schedule::IntoScheduleConfigs,
        system::{
            Commands, Local, Query, Res, ResMut, SystemChangeTick, SystemParamItem,
            lifetimeless::SRes,
        },
        world::{FromWorld, World},
    },
    log::{error, info},
    math::{Affine3A, FloatOrd, Mat3, Vec3},
    mesh::{Indices, Mesh, MeshVertexBufferLayoutRef, PrimitiveTopology},
    platform::collections::HashMap,
    prelude::{AssetChanged, Deref, DerefMut},
    render::{
        Extract, ExtractSchedule, Render, RenderApp, RenderStartup, RenderSystems,
        camera::extract_cameras,
        mesh::RenderMesh,
        render_asset::{
            PrepareAssetError, RenderAsset, RenderAssetPlugin, RenderAssets, prepare_assets,
        },
        render_phase::{
            AddRenderCommand, DrawFunctionId, DrawFunctions, PhaseItem, PhaseItemExtraIndex,
            RenderCommand, RenderCommandResult, SetItemPipeline, TrackedRenderPass,
            ViewSortedRenderPhases,
        },
        render_resource::{
            AsBindGroupError, BindGroup, BindGroupLayout, BindingResources, CachedRenderPipelineId,
            PipelineCache, RenderPipelineDescriptor, SpecializedMeshPipeline,
            SpecializedMeshPipelineError, SpecializedMeshPipelines,
        },
        renderer::RenderDevice,
        sync_world::{MainEntity, MainEntityHashMap},
        view::{ExtractedView, RenderVisibleEntities},
    },
    shader::{Shader, ShaderDefVal, ShaderRef},
    sprite_render::{
        EntitiesNeedingSpecialization, EntitySpecializationTicks, MATERIAL_2D_BIND_GROUP_INDEX,
        Material2d, Material2dBindGroupId, Material2dKey, Mesh2dPipelineKey, Mesh2dTransforms,
        MeshFlags, SetMesh2dViewBindGroup, ViewKeyCache, ViewSpecializationTicks,
        alpha_mode_pipeline_key,
    },
    transform::components::GlobalTransform,
    utils::Parallel,
};

use graph::FlashFilterRenderPlugin;
use indexmap::IndexMap;
use material::{BitmapMaterial, ColorMaterial, GradientMaterial};

use crate::{
    assets::MaterialType,
    commands::{DrawShapes, ShapeCommand},
    player::Flash,
    render::{
        material::{
            BITMAP_MATERIAL_SHADER_HANDLE, FLASH_COMMON_MATERIAL_SHADER_HANDLE,
            GRADIENT_MATERIAL_SHADER_HANDLE, SWF_COLOR_MATERIAL_SHADER_HANDLE,
        },
        offscreen_texture::{ExtractedOffscreenTexture, OffscreenTexturePlugin},
        part_mesh2d::{
            ColorTransformUniform, DrawPartMesh2d, PartMesh2dPipeline, PartMesh2dRenderPlugin,
            RenderPartMesh2dInstance, RenderPartMesh2dInstances, SetPartMesh2dBindGroup,
            init_part_mesh_2d_pipeline,
        },
        pipeline::{
            BEVEL_FILTER_SHADER_HANDLE, BLUR_FILTER_SHADER_HANDLE,
            COLOR_MATRIX_FILTER_SHADER_HANDLE, GLOW_FILTER_SHADER_HANDLE,
            OFFSCREEN_COMMON_SHADER_HANDLE, OFFSCREEN_MESH2D_BITMAP_SHADER_HANDLE,
            OFFSCREEN_MESH2D_GRADIENT_SHADER_HANDLE, OFFSCREEN_MESH2D_SHADER_HANDLE,
            init_bevel_filter_pipeline, init_blur_filter_pipeline,
            init_color_matrix_filter_pipeline, init_glow_filter_pipeline,
        },
    },
};

const VIEW_MATRIX: Mat3 = Mat3::from_cols(
    Vec3::new(1.0, 0.0, 0.0),
    Vec3::new(0.0, -1.0, 0.0),
    Vec3::new(0.0, 0.0, 1.0),
);

pub struct FlashRenderPlugin;

impl Plugin for FlashRenderPlugin {
    fn build(&self, app: &mut App) {
        load_shaders(app);

        // 注册 Flash 组件的生命周期钩子和添加为可渲染组件
        app.world_mut()
            .register_component_hooks::<Flash>()
            .on_add(add_visibility_class::<Flash>);

        app.add_plugins((
            PartMesh2dRenderPlugin,
            ShapePartMaterial2dPlugin::<GradientMaterial>::default(),
            ShapePartMaterial2dPlugin::<ColorMaterial>::default(),
            ShapePartMaterial2dPlugin::<BitmapMaterial>::default(),
            OffscreenTexturePlugin,
            FlashFilterRenderPlugin,
        ))
        .init_resource::<FilterTextureMesh>()
        .init_resource::<ColorMaterialHandle>();

        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };

        render_app
            .add_systems(
                RenderStartup,
                (
                    init_blur_filter_pipeline,
                    init_color_matrix_filter_pipeline,
                    init_glow_filter_pipeline,
                    init_bevel_filter_pipeline,
                ),
            )
            .add_systems(ExtractSchedule, extract_part_mesh2d_and_material);
    }
}

/// 用于滤镜纹理渲染的Mesh，一个固定的矩形
/// 用于滤镜纹理渲染的固定矩形网格
#[derive(Resource, Debug, Clone)]
pub struct FilterTextureMesh(pub Handle<Mesh>);

impl FromWorld for FilterTextureMesh {
    fn from_world(world: &mut bevy::ecs::world::World) -> Self {
        let mut meshes = world.resource_mut::<Assets<Mesh>>();
        let mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
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

#[derive(Resource, Debug, Clone)]
pub struct ColorMaterialHandle(pub Handle<ColorMaterial>);
impl FromWorld for ColorMaterialHandle {
    fn from_world(world: &mut World) -> Self {
        let mut materials = world.resource_mut::<Assets<ColorMaterial>>();
        Self(materials.add(ColorMaterial::default()))
    }
}

#[derive(Resource, Deref, DerefMut)]
pub struct RenderPartMaterial2dInstances<M: Material2d>(
    MainEntityHashMap<IndexMap<usize, AssetId<M>>>,
);

impl<M: Material2d> Default for RenderPartMaterial2dInstances<M> {
    fn default() -> Self {
        Self(Default::default())
    }
}

fn extract_part_mesh2d_and_material(
    mut render_part_mesh_instances: ResMut<RenderPartMesh2dInstances>,
    mut render_part_material_gradient_instances: ResMut<
        RenderPartMaterial2dInstances<GradientMaterial>,
    >,
    mut render_part_material_bitmap_instances: ResMut<
        RenderPartMaterial2dInstances<BitmapMaterial>,
    >,
    mut render_part_material_color_instances: ResMut<RenderPartMaterial2dInstances<ColorMaterial>>,
    query: Extract<Query<(Entity, &DrawShapes, &GlobalTransform)>>,
) {
    render_part_mesh_instances.clear();
    render_part_material_gradient_instances.clear();
    render_part_material_bitmap_instances.clear();

    for (entity, draw_shapes, global_transform) in query.iter() {
        let mut mesh_instances = IndexMap::default();
        let mut material_color_instances = IndexMap::default();
        let mut material_gradient_instances = IndexMap::default();
        let mut material_bitmap_instances = IndexMap::default();

        let mut index = 0;
        for shape_command in draw_shapes.iter() {
            match shape_command {
                ShapeCommand::RenderShape {
                    draw_shape,
                    transform,
                    blend_mode,
                } => {
                    let color_transform = ColorTransformUniform::from(transform.color_transform);

                    let transform = global_transform.affine()
                        * Affine3A::from_mat3(VIEW_MATRIX)
                        * Affine3A::from(transform.matrix);

                    draw_shape.iter().for_each(|mesh_draw| {
                        index += 1;

                        let mesh_asset_id = mesh_draw.mesh.id();
                        match &mesh_draw.material_type {
                            MaterialType::Color(material) => {
                                mesh_instances.insert(
                                    index,
                                    RenderPartMesh2dInstance {
                                        mesh_asset_id,
                                        material_bind_group_id: Material2dBindGroupId::default(),
                                        transforms: Mesh2dTransforms {
                                            world_from_local: (&transform).into(),
                                            flags: MeshFlags::empty().bits(),
                                        },
                                        color_transform,
                                    },
                                );
                                material_color_instances.insert(index, material.id());
                            }
                            MaterialType::Gradient(material) => {
                                mesh_instances.insert(
                                    index,
                                    RenderPartMesh2dInstance {
                                        mesh_asset_id,
                                        material_bind_group_id: Material2dBindGroupId::default(),
                                        transforms: Mesh2dTransforms {
                                            world_from_local: (&transform).into(),
                                            flags: MeshFlags::empty().bits(),
                                        },
                                        color_transform,
                                    },
                                );
                                material_gradient_instances.insert(index, material.id());
                            }
                            MaterialType::Bitmap(material) => {
                                mesh_instances.insert(
                                    index,
                                    RenderPartMesh2dInstance {
                                        mesh_asset_id,
                                        material_bind_group_id: Material2dBindGroupId::default(),
                                        transforms: Mesh2dTransforms {
                                            world_from_local: (&transform).into(),
                                            flags: MeshFlags::empty().bits(),
                                        },
                                        color_transform,
                                    },
                                );
                                material_bitmap_instances.insert(index, material.id());
                            }
                        }
                    });
                }
                ShapeCommand::RenderBitmap {
                    mesh,
                    material,
                    transform,
                    ..
                } => {
                    index += 1;

                    let color_transform = ColorTransformUniform::from(transform.color_transform);

                    let transform = global_transform.affine()
                        * Affine3A::from_mat3(VIEW_MATRIX)
                        * Affine3A::from(transform.matrix);

                    mesh_instances.insert(
                        index,
                        RenderPartMesh2dInstance {
                            mesh_asset_id: mesh.id(),
                            material_bind_group_id: Material2dBindGroupId::default(),
                            transforms: Mesh2dTransforms {
                                world_from_local: (&transform).into(),
                                flags: MeshFlags::empty().bits(),
                            },
                            color_transform,
                        },
                    );
                    material_bitmap_instances.insert(index, material.id());
                }
            }
        }
        render_part_mesh_instances.insert(entity.into(), mesh_instances);

        render_part_material_gradient_instances.insert(entity.into(), material_gradient_instances);
        render_part_material_bitmap_instances.insert(entity.into(), material_bitmap_instances);
        render_part_material_color_instances.insert(entity.into(), material_color_instances);
    }
}

pub struct ShapePartMaterial2dPlugin<M: Material2d>(PhantomData<M>);
impl<M: Material2d> Default for ShapePartMaterial2dPlugin<M> {
    fn default() -> Self {
        Self(Default::default())
    }
}

impl<M: Material2d> Plugin for ShapePartMaterial2dPlugin<M>
where
    M::Data: PartialEq + Eq + Hash + Clone,
{
    fn build(&self, app: &mut App) {
        app.init_asset::<M>()
            .add_plugins(RenderAssetPlugin::<PreparedPartMaterial2d<M>>::default())
            .init_resource::<EntitiesNeedingSpecialization<M>>()
            .add_systems(
                PostUpdate,
                check_entities_needing_specialization::<M>.after(AssetEventSystems),
            );

        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<RenderPartMaterial2dInstances<M>>()
            .init_resource::<EntitySpecializationTicks<M>>()
            .init_resource::<SpecializedPartMaterial2dPipelineCache<M>>()
            .init_resource::<SpecializedMeshPipelines<PartMaterial2dPipeline<M>>>()
            .add_render_command::<Transparent2d, DrawPartMaterial2d<M>>()
            .add_systems(
                ExtractSchedule,
                extract_entities_needs_specialization::<M>.after(extract_cameras),
            )
            .add_systems(
                RenderStartup,
                init_part_material_2d_pipeline::<M>.after(init_part_mesh_2d_pipeline),
            )
            .add_systems(
                Render,
                (
                    specialize_part_material2d::<M>
                        .in_set(RenderSystems::PrepareMeshes)
                        .after(prepare_assets::<PreparedPartMaterial2d<M>>)
                        .after(prepare_assets::<RenderMesh>),
                    queue_part_material2d_meshes::<M>
                        .in_set(RenderSystems::QueueMeshes)
                        .after(prepare_assets::<RenderMesh>)
                        .after(prepare_assets::<PreparedPartMaterial2d<M>>),
                ),
            );
    }
}

#[derive(Resource, Deref, DerefMut)]
pub struct SpecializedPartMaterial2dPipelineCache<M> {
    #[deref]
    map: MainEntityHashMap<SpecializedPartMaterial2dViewPipeline<M>>,
    marker: PhantomData<M>,
}

#[derive(Deref, DerefMut)]
pub struct SpecializedPartMaterial2dViewPipeline<M> {
    #[deref]
    map: MainEntityHashMap<(Tick, HashMap<usize, CachedRenderPipelineId>)>,
    marker: PhantomData<M>,
}

impl<M> Default for SpecializedPartMaterial2dPipelineCache<M> {
    fn default() -> Self {
        Self {
            map: HashMap::default(),
            marker: PhantomData,
        }
    }
}

impl<M> Default for SpecializedPartMaterial2dViewPipeline<M> {
    fn default() -> Self {
        Self {
            map: HashMap::default(),
            marker: PhantomData,
        }
    }
}
/// TODO:是否可以考虑不要这个？如果影响性能。因为DrawShapes 的变动和最终需要的管线是不相关的。可以默认地认为就是没帧都要变化吗？
/// 虽然材质的数量是一定的但是由于混合模式的存在，所以渲染管线可能会有很多个，3种材质，6种混合模式，18个渲染管线
pub fn check_entities_needing_specialization<M>(
    needs_specialization: Query<
        Entity,
        (
            Or<(Changed<DrawShapes>, AssetChanged<Flash>)>,
            With<DrawShapes>,
        ),
    >,
    mut par_local: Local<Parallel<Vec<Entity>>>,
    mut entities_needing_specialization: ResMut<EntitiesNeedingSpecialization<M>>,
) where
    M: Material2d,
{
    entities_needing_specialization.clear();

    needs_specialization
        .par_iter()
        .for_each(|entity| par_local.borrow_local_mut().push(entity));

    par_local.drain_into(&mut entities_needing_specialization);
}

pub fn extract_entities_needs_specialization<M>(
    entities_needing_specialization: Extract<Res<EntitiesNeedingSpecialization<M>>>,
    mut entity_specialization_ticks: ResMut<EntitySpecializationTicks<M>>,
    mut removed_mesh_material_components: Extract<RemovedComponents<Flash>>,
    mut specialized_part_material_pipeline_cache: ResMut<SpecializedPartMaterial2dPipelineCache<M>>,
    views: Query<&MainEntity, With<ExtractedView>>,
    ticks: SystemChangeTick,
) where
    M: Material2d,
{
    // Clean up any despawned entities, we do this first in case the removed material was re-added
    // the same frame, thus will appear both in the removed components list and have been added to
    // the `EntitiesNeedingSpecialization` collection by triggering the `Changed` filter
    for entity in removed_mesh_material_components.read() {
        entity_specialization_ticks.remove(&MainEntity::from(entity));
        for view in views {
            if let Some(cache) = specialized_part_material_pipeline_cache.get_mut(view) {
                cache.remove(&MainEntity::from(entity));
            }
        }
    }
    for entity in entities_needing_specialization.iter() {
        // Update the entity's specialization tick with this run's tick
        entity_specialization_ticks.insert((*entity).into(), ticks.this_run());
    }
}

/// Render pipeline data for a given [`Material2d`]
#[derive(Resource)]
pub struct PartMaterial2dPipeline<M: Material2d> {
    pub mesh2d_pipeline: PartMesh2dPipeline,
    pub material2d_layout: BindGroupLayout,
    pub vertex_shader: Option<Handle<Shader>>,
    pub fragment_shader: Option<Handle<Shader>>,
    marker: PhantomData<M>,
}

impl<M: Material2d> Clone for PartMaterial2dPipeline<M> {
    fn clone(&self) -> Self {
        Self {
            mesh2d_pipeline: self.mesh2d_pipeline.clone(),
            material2d_layout: self.material2d_layout.clone(),
            vertex_shader: self.vertex_shader.clone(),
            fragment_shader: self.fragment_shader.clone(),
            marker: PhantomData,
        }
    }
}

impl<M: Material2d> SpecializedMeshPipeline for PartMaterial2dPipeline<M>
where
    M::Data: PartialEq + Eq + Hash + Clone,
{
    type Key = Material2dKey<M>;

    fn specialize(
        &self,
        key: Self::Key,
        layout: &MeshVertexBufferLayoutRef,
    ) -> Result<RenderPipelineDescriptor, SpecializedMeshPipelineError> {
        let mut descriptor = self.mesh2d_pipeline.specialize(key.mesh_key, layout)?;
        descriptor.vertex.shader_defs.push(ShaderDefVal::UInt(
            "MATERIAL_BIND_GROUP".into(),
            MATERIAL_2D_BIND_GROUP_INDEX as u32,
        ));
        if let Some(ref mut fragment) = descriptor.fragment {
            fragment.shader_defs.push(ShaderDefVal::UInt(
                "MATERIAL_BIND_GROUP".into(),
                MATERIAL_2D_BIND_GROUP_INDEX as u32,
            ));
        }
        if let Some(vertex_shader) = &self.vertex_shader {
            descriptor.vertex.shader = vertex_shader.clone();
        }

        if let Some(fragment_shader) = &self.fragment_shader {
            descriptor.fragment.as_mut().unwrap().shader = fragment_shader.clone();
        }
        descriptor.layout = vec![
            self.mesh2d_pipeline.view_layout.clone(),
            self.mesh2d_pipeline.mesh_layout.clone(),
            self.material2d_layout.clone(),
        ];

        M::specialize(&mut descriptor, layout, key)?;
        Ok(descriptor)
    }
}

pub fn init_part_material_2d_pipeline<M: Material2d>(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    asset_server: Res<AssetServer>,
    part_mesh_2d_pipeline: Res<PartMesh2dPipeline>,
) {
    let material2d_layout = M::bind_group_layout(&render_device);

    commands.insert_resource(PartMaterial2dPipeline::<M> {
        mesh2d_pipeline: part_mesh_2d_pipeline.clone(),
        material2d_layout,
        vertex_shader: match M::vertex_shader() {
            ShaderRef::Default => None,
            ShaderRef::Handle(handle) => Some(handle),
            ShaderRef::Path(path) => Some(asset_server.load(path)),
        },
        fragment_shader: match M::fragment_shader() {
            ShaderRef::Default => None,
            ShaderRef::Handle(handle) => Some(handle),
            ShaderRef::Path(path) => Some(asset_server.load(path)),
        },
        marker: PhantomData,
    });
}

fn specialize_part_material2d<M: Material2d>(
    material2d_pipeline: Res<PartMaterial2dPipeline<M>>,
    mut pipelines: ResMut<SpecializedMeshPipelines<PartMaterial2dPipeline<M>>>,
    pipeline_cache: Res<PipelineCache>,
    (render_meshes, render_materials): (
        Res<RenderAssets<RenderMesh>>,
        Res<RenderAssets<PreparedPartMaterial2d<M>>>,
    ),
    mut render_part_mesh_instances: ResMut<RenderPartMesh2dInstances>,
    render_part_material_instances: Res<RenderPartMaterial2dInstances<M>>,
    transparent_render_phase: Res<ViewSortedRenderPhases<Transparent2d>>,
    views: Query<(&MainEntity, &ExtractedView, &RenderVisibleEntities)>,
    view_key_cache: Res<ViewKeyCache>,
    entity_specialization_ticks: Res<EntitySpecializationTicks<M>>,
    view_specialization_ticks: Res<ViewSpecializationTicks>,
    ticks: SystemChangeTick,
    mut specialized_part_material_pipeline_cache: ResMut<SpecializedPartMaterial2dPipelineCache<M>>,
) where
    M::Data: PartialEq + Eq + Hash + Clone,
{
    if render_part_material_instances.is_empty() {
        return;
    }
    for (view_entity, view, visible_entities) in &views {
        if !transparent_render_phase.contains_key(&view.retained_view_entity) {
            continue;
        }

        let Some(view_key) = view_key_cache.get(view_entity) else {
            continue;
        };
        let view_tick = view_specialization_ticks.get(view_entity).unwrap();

        let view_specialized_part_material_pipeline_cache =
            specialized_part_material_pipeline_cache
                .entry(*view_entity)
                .or_default();

        for (_, visible_entity) in visible_entities.iter::<Flash>() {
            let Some(mesh_instances) = render_part_mesh_instances.get_mut(visible_entity) else {
                continue;
            };

            let Some(material_instances) = render_part_material_instances.get(visible_entity)
            else {
                continue;
            };
            let Some(entity_tick) = entity_specialization_ticks.get(visible_entity) else {
                continue;
            };

            let last_specialized_tick = view_specialized_part_material_pipeline_cache
                .get(visible_entity)
                .map(|(tick, _)| *tick);
            let needs_specialization = last_specialized_tick.is_none_or(|tick| {
                view_tick.is_newer_than(tick, ticks.this_run())
                    || entity_tick.is_newer_than(tick, ticks.this_run())
            });
            if !needs_specialization {
                continue;
            }
            let mut pipeline_ids = HashMap::default();

            for (index, mesh_instance) in mesh_instances.iter_mut() {
                let Some(material_asset_id) = material_instances.get(index) else {
                    continue;
                };
                let Some(mesh) = render_meshes.get(mesh_instance.mesh_asset_id) else {
                    continue;
                };
                let Some(material_2d) = render_materials.get(*material_asset_id) else {
                    continue;
                };
                let mesh_key = *view_key
                    | Mesh2dPipelineKey::from_primitive_topology(mesh.primitive_topology())
                    | material_2d.properties.mesh_pipeline_key_bits;
                let pipeline_id = pipelines.specialize(
                    &pipeline_cache,
                    &material2d_pipeline,
                    Material2dKey {
                        mesh_key,
                        bind_group_data: material_2d.key.clone(),
                    },
                    &mesh.layout,
                );
                let pipeline_id = match pipeline_id {
                    Ok(id) => id,
                    Err(err) => {
                        error!("{}", err);
                        continue;
                    }
                };
                pipeline_ids.insert(*index, pipeline_id);
            }
            view_specialized_part_material_pipeline_cache
                .insert(*visible_entity, (ticks.this_run(), pipeline_ids));
        }
    }
}

fn queue_part_material2d_meshes<M: Material2d>(
    (render_meshes, render_materials): (
        Res<RenderAssets<RenderMesh>>,
        Res<RenderAssets<PreparedPartMaterial2d<M>>>,
    ),
    mut render_part_mesh_instances: ResMut<RenderPartMesh2dInstances>,
    render_part_material_instances: Res<RenderPartMaterial2dInstances<M>>,
    mut transparent_render_phases: ResMut<ViewSortedRenderPhases<Transparent2d>>,
    views: Query<(&MainEntity, &ExtractedView, &RenderVisibleEntities)>,
    specialized_part_material_pipeline_cache: Res<SpecializedPartMaterial2dPipelineCache<M>>,
) where
    M::Data: PartialEq + Eq + Hash + Clone,
{
    if render_part_material_instances.is_empty() || render_part_mesh_instances.is_empty() {
        return;
    }
    for (view_entity, view, visible_entities) in views.iter() {
        let Some(view_speicalized_part_material_pipeline_cache) =
            specialized_part_material_pipeline_cache.get(view_entity)
        else {
            continue;
        };

        let Some(transparent_phase) = transparent_render_phases.get_mut(&view.retained_view_entity)
        else {
            continue;
        };

        for (render_entity, visible_entity) in visible_entities.iter::<Flash>() {
            let Some((_, pipeline_ids)) = view_speicalized_part_material_pipeline_cache
                .get(visible_entity)
                .map(|(current_change_tick, pipeline_id)| {
                    (*current_change_tick, pipeline_id.clone())
                })
            else {
                continue;
            };

            let Some(mesh_instances) = render_part_mesh_instances.get_mut(visible_entity) else {
                continue;
            };

            let Some(material_instances) = render_part_material_instances.get(visible_entity)
            else {
                continue;
            };

            for (index, mesh_instance) in mesh_instances.iter_mut() {
                let Some(material_asset_id) = material_instances.get(index) else {
                    continue;
                };
                let Some(mesh) = render_meshes.get(mesh_instance.mesh_asset_id) else {
                    continue;
                };
                let Some(material_2d) = render_materials.get(*material_asset_id) else {
                    continue;
                };
                let pipeline_id = pipeline_ids
                    .get(index)
                    .expect("获取对应的pipeline_id失败！");

                mesh_instance.material_bind_group_id = material_2d.get_bind_group_id();
                let mesh_z = mesh_instance.transforms.world_from_local.translation.z;

                transparent_phase.add(Transparent2d {
                    sort_key: FloatOrd(
                        mesh_z + material_2d.properties.depth_bias + *index as f32 * 0.001,
                    ),
                    entity: (*render_entity, *visible_entity),
                    pipeline: *pipeline_id,
                    draw_function: material_2d.properties.draw_function_id,
                    batch_range: 0..1,
                    extracted_index: *index,
                    extra_index: PhaseItemExtraIndex::None,
                    indexed: mesh.indexed(),
                });
            }
        }
    }
}

pub trait PartPhaseItem: PhaseItem {
    /// 对应 `RenderFlashShapeInstances` 中的索引
    fn extracted_index(&self) -> usize;
}

impl PartPhaseItem for Transparent2d {
    fn extracted_index(&self) -> usize {
        self.extracted_index
    }
}

pub type DrawPartMaterial2d<M> = (
    SetItemPipeline,
    SetMesh2dViewBindGroup<0>,
    SetPartMesh2dBindGroup<1>,
    SetPartMaterial2dBindGroup<M, MATERIAL_2D_BIND_GROUP_INDEX>,
    DrawPartMesh2d,
);

pub struct SetPartMaterial2dBindGroup<M: Material2d, const I: usize>(PhantomData<M>);
impl<P: PartPhaseItem, M: Material2d, const I: usize> RenderCommand<P>
    for SetPartMaterial2dBindGroup<M, I>
{
    type Param = (
        SRes<RenderAssets<PreparedPartMaterial2d<M>>>,
        SRes<RenderPartMaterial2dInstances<M>>,
    );
    type ViewQuery = ();
    type ItemQuery = ();

    #[inline]
    fn render<'w>(
        item: &P,
        _view: (),
        _item_query: Option<()>,
        (materials, material_instances): SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let materials = materials.into_inner();
        let material_instances = material_instances.into_inner();
        let Some(material_instances) = material_instances.get(&item.main_entity()) else {
            return RenderCommandResult::Skip;
        };
        let Some(material_instance) = material_instances.get(&item.extracted_index()) else {
            return RenderCommandResult::Skip;
        };
        let Some(material2d) = materials.get(*material_instance) else {
            return RenderCommandResult::Skip;
        };
        pass.set_bind_group(I, &material2d.bind_group, &[]);
        RenderCommandResult::Success
    }
}

pub struct PartMaterial2dProperties {
    pub depth_bias: f32,

    pub mesh_pipeline_key_bits: Mesh2dPipelineKey,

    pub draw_function_id: DrawFunctionId,
}

/// 仅仅支持[`Transparent2d`] item
pub struct PreparedPartMaterial2d<M: Material2d> {
    pub bindings: BindingResources,
    pub bind_group: BindGroup,
    pub key: M::Data,
    pub properties: PartMaterial2dProperties,
}

impl<M: Material2d> PreparedPartMaterial2d<M> {
    pub fn get_bind_group_id(&self) -> Material2dBindGroupId {
        Material2dBindGroupId(Some(self.bind_group.id()))
    }
}

impl<M: Material2d> RenderAsset for PreparedPartMaterial2d<M> {
    type SourceAsset = M;

    type Param = (
        SRes<RenderDevice>,
        SRes<PartMaterial2dPipeline<M>>,
        SRes<DrawFunctions<Transparent2d>>,
        M::Param,
    );

    fn prepare_asset(
        material: Self::SourceAsset,
        _: AssetId<Self::SourceAsset>,
        (render_device, pipeline, opaque_draw_functions, material_param): &mut SystemParamItem<
            Self::Param,
        >,
        _: Option<&Self>,
    ) -> Result<Self, PrepareAssetError<Self::SourceAsset>> {
        let bind_group_data = material.bind_group_data();
        match material.as_bind_group(&pipeline.material2d_layout, render_device, material_param) {
            Ok(prepared) => {
                let mut mesh_pipeline_key_bits = Mesh2dPipelineKey::empty();
                mesh_pipeline_key_bits.insert(alpha_mode_pipeline_key(material.alpha_mode()));
                let draw_function_id = opaque_draw_functions.read().id::<DrawPartMaterial2d<M>>();

                Ok(PreparedPartMaterial2d {
                    bindings: prepared.bindings,
                    bind_group: prepared.bind_group,
                    key: bind_group_data,
                    properties: PartMaterial2dProperties {
                        depth_bias: material.depth_bias(),
                        mesh_pipeline_key_bits,
                        draw_function_id,
                    },
                })
            }
            Err(AsBindGroupError::RetryNextUpdate) => {
                Err(PrepareAssetError::RetryNextUpdate(material))
            }
            Err(other) => Err(PrepareAssetError::AsBindGroupError(other)),
        }
    }
}

fn load_shaders(app: &mut App) {
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
        OFFSCREEN_COMMON_SHADER_HANDLE,
        "render/shaders/offscreen_mesh2d/offscreen_common.wgsl",
        Shader::from_wgsl
    );
    load_internal_asset!(
        app,
        OFFSCREEN_MESH2D_SHADER_HANDLE,
        "render/shaders/offscreen_mesh2d/color.wgsl",
        Shader::from_wgsl
    );
    load_internal_asset!(
        app,
        OFFSCREEN_MESH2D_GRADIENT_SHADER_HANDLE,
        "render/shaders/offscreen_mesh2d/gradient.wgsl",
        Shader::from_wgsl
    );
    load_internal_asset!(
        app,
        OFFSCREEN_MESH2D_BITMAP_SHADER_HANDLE,
        "render/shaders/offscreen_mesh2d/bitmap.wgsl",
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
}
