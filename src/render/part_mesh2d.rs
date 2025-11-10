use bevy::{
    app::{App, Plugin},
    asset::{AssetId, Handle},
    core_pipeline::{
        core_2d::{CORE_2D_DEPTH_FORMAT, Transparent2d},
        tonemapping::get_lut_bind_group_layout_entries,
    },
    ecs::{
        entity::Entity,
        resource::Resource,
        schedule::IntoScheduleConfigs,
        system::{
            Commands, Res, ResMut, StaticSystemParam, SystemParam, SystemParamItem,
            lifetimeless::SRes,
        },
    },
    image::BevyDefault,
    math::Vec4,
    mesh::{Mesh, MeshVertexBufferLayoutRef},
    prelude::{Deref, DerefMut},
    render::{
        Render, RenderApp, RenderStartup, RenderSystems,
        batching::no_gpu_preprocessing::BatchedInstanceBuffer,
        globals::GlobalsUniform,
        mesh::{RenderMesh, RenderMeshBufferInfo, allocator::MeshAllocator},
        render_asset::RenderAssets,
        render_phase::{
            CachedRenderPipelinePhaseItem, DrawFunctionId, PhaseItemExtraIndex, RenderCommand,
            RenderCommandResult, SortedPhaseItem, SortedRenderPhase, TrackedRenderPass,
            ViewSortedRenderPhases,
        },
        render_resource::{
            BindGroup, BindGroupEntries, BindGroupLayout, BindGroupLayoutEntries, BlendState,
            CachedRenderPipelineId, ColorTargetState, ColorWrites, CompareFunction, DepthBiasState,
            DepthStencilState, FragmentState, FrontFace, GpuArrayBuffer, GpuArrayBufferable,
            MultisampleState, PolygonMode, PrimitiveState, RenderPipelineDescriptor, ShaderStages,
            ShaderType, SpecializedMeshPipeline, SpecializedMeshPipelineError,
            SpecializedMeshPipelines, StencilFaceState, StencilState, TextureFormat, VertexState,
            binding_types::uniform_buffer,
        },
        renderer::{RenderDevice, RenderQueue},
        sync_world::{MainEntity, MainEntityHashMap},
        view::{ViewTarget, ViewUniform},
    },
    shader::{Shader, ShaderDefVal},
    sprite_render::{
        Material2dBindGroupId, Mesh2dPipeline, Mesh2dPipelineKey, Mesh2dTransforms, Mesh2dUniform,
        init_mesh_2d_pipeline,
    },
    utils::default,
};
use indexmap::IndexMap;
use nonmax::NonMaxU32;
use swf::ColorTransform;

use crate::render::PartPhaseItem;

pub struct PartMesh2dRenderPlugin;

impl Plugin for PartMesh2dRenderPlugin {
    fn build(&self, app: &mut App) {
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };

        render_app
            .init_resource::<RenderPartMesh2dInstances>()
            .init_resource::<SpecializedMeshPipelines<PartMesh2dPipeline>>();
        render_app
            .add_systems(
                RenderStartup,
                (
                    init_batched_instance_buffer,
                    init_part_mesh_2d_pipeline.after(init_mesh_2d_pipeline),
                ),
            )
            .add_systems(
                Render,
                (
                    batch_and_prepare_part_sorted_render_phase::<Transparent2d, PartMesh2dPipeline>
                        .in_set(RenderSystems::PrepareResources),
                    write_batched_part_instance_buffer::<PartMesh2dPipeline>
                        .in_set(RenderSystems::PrepareResourcesFlush),
                    prepare_part_mesh2d_bind_group.in_set(RenderSystems::PrepareBindGroups),
                    clear_batched_cpu_part_instance_buffers::<PartMesh2dPipeline>
                        .in_set(RenderSystems::Cleanup)
                        .after(RenderSystems::Render),
                ),
            );
    }
}

#[derive(Debug, Clone, Copy, ShaderType)]
pub struct ColorTransformUniform {
    pub mult_color: Vec4,
    pub add_color: Vec4,
}
impl Default for ColorTransformUniform {
    fn default() -> Self {
        Self {
            mult_color: Vec4::ONE,
            add_color: Vec4::ZERO,
        }
    }
}

impl From<ColorTransform> for ColorTransformUniform {
    fn from(value: ColorTransform) -> Self {
        Self {
            mult_color: Vec4::from_array(value.mult_rgba_normalized()),
            add_color: Vec4::from_array(value.add_rgba_normalized()),
        }
    }
}

pub struct RenderPartMesh2dInstance {
    pub mesh_asset_id: AssetId<Mesh>,
    pub material_bind_group_id: Material2dBindGroupId,
    pub transforms: Mesh2dTransforms,
    pub color_transform: ColorTransformUniform,
}

#[derive(Default, Resource, Deref, DerefMut)]
pub struct RenderPartMesh2dInstances(MainEntityHashMap<IndexMap<usize, RenderPartMesh2dInstance>>);

/// Render pipeline data for a given [`PartMaterial2d`]
#[derive(Resource, Clone)]
pub struct PartMesh2dPipeline {
    pub view_layout: BindGroupLayout,
    pub mesh_layout: BindGroupLayout,
    pub shader: Handle<Shader>,
    pub per_object_buffer_batch_size: Option<u32>,
}
pub fn init_part_mesh_2d_pipeline(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    mesh2d_pipeline: Res<Mesh2dPipeline>,
) {
    let tonemapping_lut_entries = get_lut_bind_group_layout_entries();
    let view_layout = render_device.create_bind_group_layout(
        "mesh2d_view_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::VERTEX_FRAGMENT,
            (
                uniform_buffer::<ViewUniform>(true),
                uniform_buffer::<GlobalsUniform>(false),
                tonemapping_lut_entries[0].visibility(ShaderStages::FRAGMENT),
                tonemapping_lut_entries[1].visibility(ShaderStages::FRAGMENT),
            ),
        ),
    );

    let mesh_layout = render_device.create_bind_group_layout(
        "part_mesh2d_layout",
        &BindGroupLayoutEntries::single(
            ShaderStages::VERTEX_FRAGMENT,
            GpuArrayBuffer::<PartMesh2dUniform>::binding_layout(&render_device),
        ),
    );

    commands.insert_resource(PartMesh2dPipeline {
        view_layout,
        mesh_layout,
        per_object_buffer_batch_size: GpuArrayBuffer::<Mesh2dUniform>::batch_size(&render_device),
        shader: mesh2d_pipeline.shader.clone(),
    });
}

#[derive(Resource)]
pub struct PartMesh2dBindGroup {
    pub value: BindGroup,
}

pub fn prepare_part_mesh2d_bind_group(
    mut commands: Commands,
    part_mesh2d_pipeline: Res<PartMesh2dPipeline>,
    render_device: Res<RenderDevice>,
    part_mesh2d_uniforms: Res<BatchedInstanceBuffer<PartMesh2dUniform>>,
) {
    if let Some(binding) = part_mesh2d_uniforms.instance_data_binding() {
        commands.insert_resource(PartMesh2dBindGroup {
            value: render_device.create_bind_group(
                "part_mesh2d_bind_group",
                &part_mesh2d_pipeline.mesh_layout,
                &BindGroupEntries::single(binding),
            ),
        });
    }
}

pub fn init_batched_instance_buffer(mut commands: Commands, render_device: Res<RenderDevice>) {
    commands.insert_resource(BatchedInstanceBuffer::<PartMesh2dUniform>::new(
        &render_device,
    ));
}

impl SpecializedMeshPipeline for PartMesh2dPipeline {
    type Key = Mesh2dPipelineKey;

    fn specialize(
        &self,
        key: Self::Key,
        layout: &MeshVertexBufferLayoutRef,
    ) -> Result<RenderPipelineDescriptor, SpecializedMeshPipelineError> {
        let mut shader_defs = Vec::new();
        let mut vertex_attributes = Vec::new();

        if layout.0.contains(Mesh::ATTRIBUTE_POSITION) {
            shader_defs.push("VERTEX_POSITIONS".into());
            vertex_attributes.push(Mesh::ATTRIBUTE_POSITION.at_shader_location(0));
        }

        if layout.0.contains(Mesh::ATTRIBUTE_NORMAL) {
            shader_defs.push("VERTEX_NORMALS".into());
            vertex_attributes.push(Mesh::ATTRIBUTE_NORMAL.at_shader_location(1));
        }

        if layout.0.contains(Mesh::ATTRIBUTE_UV_0) {
            shader_defs.push("VERTEX_UVS".into());
            vertex_attributes.push(Mesh::ATTRIBUTE_UV_0.at_shader_location(2));
        }

        if layout.0.contains(Mesh::ATTRIBUTE_TANGENT) {
            shader_defs.push("VERTEX_TANGENTS".into());
            vertex_attributes.push(Mesh::ATTRIBUTE_TANGENT.at_shader_location(3));
        }

        if layout.0.contains(Mesh::ATTRIBUTE_COLOR) {
            shader_defs.push("VERTEX_COLORS".into());
            vertex_attributes.push(Mesh::ATTRIBUTE_COLOR.at_shader_location(4));
        }

        if key.contains(Mesh2dPipelineKey::TONEMAP_IN_SHADER) {
            shader_defs.push("TONEMAP_IN_SHADER".into());
            shader_defs.push(ShaderDefVal::UInt(
                "TONEMAPPING_LUT_TEXTURE_BINDING_INDEX".into(),
                2,
            ));
            shader_defs.push(ShaderDefVal::UInt(
                "TONEMAPPING_LUT_SAMPLER_BINDING_INDEX".into(),
                3,
            ));

            let method = key.intersection(Mesh2dPipelineKey::TONEMAP_METHOD_RESERVED_BITS);

            match method {
                Mesh2dPipelineKey::TONEMAP_METHOD_NONE => {
                    shader_defs.push("TONEMAP_METHOD_NONE".into());
                }
                Mesh2dPipelineKey::TONEMAP_METHOD_REINHARD => {
                    shader_defs.push("TONEMAP_METHOD_REINHARD".into());
                }
                Mesh2dPipelineKey::TONEMAP_METHOD_REINHARD_LUMINANCE => {
                    shader_defs.push("TONEMAP_METHOD_REINHARD_LUMINANCE".into());
                }
                Mesh2dPipelineKey::TONEMAP_METHOD_ACES_FITTED => {
                    shader_defs.push("TONEMAP_METHOD_ACES_FITTED".into());
                }
                Mesh2dPipelineKey::TONEMAP_METHOD_AGX => {
                    shader_defs.push("TONEMAP_METHOD_AGX".into());
                }
                Mesh2dPipelineKey::TONEMAP_METHOD_SOMEWHAT_BORING_DISPLAY_TRANSFORM => {
                    shader_defs.push("TONEMAP_METHOD_SOMEWHAT_BORING_DISPLAY_TRANSFORM".into());
                }
                Mesh2dPipelineKey::TONEMAP_METHOD_BLENDER_FILMIC => {
                    shader_defs.push("TONEMAP_METHOD_BLENDER_FILMIC".into());
                }
                Mesh2dPipelineKey::TONEMAP_METHOD_TONY_MC_MAPFACE => {
                    shader_defs.push("TONEMAP_METHOD_TONY_MC_MAPFACE".into());
                }
                _ => {}
            }
            // Debanding is tied to tonemapping in the shader, cannot run without it.
            if key.contains(Mesh2dPipelineKey::DEBAND_DITHER) {
                shader_defs.push("DEBAND_DITHER".into());
            }
        }

        if key.contains(Mesh2dPipelineKey::MAY_DISCARD) {
            shader_defs.push("MAY_DISCARD".into());
        }

        let vertex_buffer_layout = layout.0.get_layout(&vertex_attributes)?;

        let format = match key.contains(Mesh2dPipelineKey::HDR) {
            true => ViewTarget::TEXTURE_FORMAT_HDR,
            false => TextureFormat::bevy_default(),
        };

        let (depth_write_enabled, label, blend);
        if key.contains(Mesh2dPipelineKey::BLEND_ALPHA) {
            label = "transparent_mesh2d_pipeline";
            blend = Some(BlendState::ALPHA_BLENDING);
            depth_write_enabled = false;
        } else {
            label = "opaque_mesh2d_pipeline";
            blend = None;
            depth_write_enabled = true;
        }

        Ok(RenderPipelineDescriptor {
            vertex: VertexState {
                shader: self.shader.clone(),
                shader_defs: shader_defs.clone(),
                buffers: vec![vertex_buffer_layout],
                ..default()
            },
            fragment: Some(FragmentState {
                shader: self.shader.clone(),
                shader_defs,
                targets: vec![Some(ColorTargetState {
                    format,
                    blend,
                    write_mask: ColorWrites::ALL,
                })],
                ..default()
            }),
            layout: vec![self.view_layout.clone(), self.mesh_layout.clone()],
            primitive: PrimitiveState {
                front_face: FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: PolygonMode::Fill,
                conservative: false,
                topology: key.primitive_topology(),
                strip_index_format: None,
            },
            depth_stencil: Some(DepthStencilState {
                format: CORE_2D_DEPTH_FORMAT,
                depth_write_enabled,
                depth_compare: CompareFunction::GreaterEqual,
                stencil: StencilState {
                    front: StencilFaceState::IGNORE,
                    back: StencilFaceState::IGNORE,
                    read_mask: 0,
                    write_mask: 0,
                },
                bias: DepthBiasState {
                    constant: 0,
                    slope_scale: 0.0,
                    clamp: 0.0,
                },
            }),
            multisample: MultisampleState {
                count: key.msaa_samples(),
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            label: Some(label.into()),
            ..default()
        })
    }
}

pub struct SetPartMesh2dBindGroup<const I: usize>;
impl<P: PartPhaseItem, const I: usize> RenderCommand<P> for SetPartMesh2dBindGroup<I> {
    type Param = SRes<PartMesh2dBindGroup>;
    type ViewQuery = ();
    type ItemQuery = ();

    #[inline]
    fn render<'w>(
        item: &P,
        _view: (),
        _item_query: Option<()>,
        part_mesh2d_bind_group: SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let mut dynamic_offsets: [u32; 1] = Default::default();
        let mut offset_count = 0;
        if let PhaseItemExtraIndex::DynamicOffset(dynamic_offset) = item.extra_index() {
            dynamic_offsets[offset_count] = dynamic_offset;
            offset_count += 1;
        }
        pass.set_bind_group(
            I,
            &part_mesh2d_bind_group.into_inner().value,
            &dynamic_offsets[..offset_count],
        );
        RenderCommandResult::Success
    }
}

pub struct DrawPartMesh2d;
impl<PP: PartPhaseItem> RenderCommand<PP> for DrawPartMesh2d {
    type Param = (
        SRes<RenderAssets<RenderMesh>>,
        SRes<RenderPartMesh2dInstances>,
        SRes<MeshAllocator>,
    );

    type ViewQuery = ();

    type ItemQuery = ();

    fn render<'w>(
        item: &PP,
        _view: (),
        _entity: Option<()>,
        (meshes, render_instances, mesh_allocator): SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let meshes = meshes.into_inner();
        let render_instances = render_instances.into_inner();
        let mesh_allocator = mesh_allocator.into_inner();

        let Some(instances) = render_instances.get(&item.main_entity()) else {
            return RenderCommandResult::Skip;
        };

        let Some(RenderPartMesh2dInstance { mesh_asset_id, .. }) =
            instances.get(&item.extracted_index())
        else {
            return RenderCommandResult::Skip;
        };

        let Some(gpu_mesh) = meshes.get(*mesh_asset_id) else {
            return RenderCommandResult::Skip;
        };

        let Some(vertex_buffer_slice) = mesh_allocator.mesh_vertex_slice(mesh_asset_id) else {
            return RenderCommandResult::Skip;
        };

        pass.set_vertex_buffer(0, vertex_buffer_slice.buffer.slice(..));

        let batch_range = item.batch_range();
        match &gpu_mesh.buffer_info {
            RenderMeshBufferInfo::Indexed {
                index_format,
                count,
            } => {
                let Some(index_buffer_slice) = mesh_allocator.mesh_index_slice(mesh_asset_id)
                else {
                    return RenderCommandResult::Skip;
                };

                pass.set_index_buffer(index_buffer_slice.buffer.slice(..), 0, *index_format);

                pass.draw_indexed(
                    index_buffer_slice.range.start..(index_buffer_slice.range.start + count),
                    vertex_buffer_slice.range.start as i32,
                    batch_range.clone(),
                );
            }
            RenderMeshBufferInfo::NonIndexed => {
                pass.draw(vertex_buffer_slice.range, batch_range.clone());
            }
        }

        RenderCommandResult::Success
    }
}

impl GetBatchPartData for PartMesh2dPipeline {
    type Param = (
        SRes<RenderPartMesh2dInstances>,
        SRes<RenderAssets<RenderMesh>>,
        SRes<MeshAllocator>,
    );
    type CompareData = (Material2dBindGroupId, AssetId<Mesh>);
    type BufferData = PartMesh2dUniform;

    fn get_batch_part_data(
        (part_mesh_instances, _, _): &SystemParamItem<Self::Param>,
        (_entity, main_entity, index): (Entity, MainEntity, usize),
    ) -> Option<(Self::BufferData, Option<Self::CompareData>)> {
        let mesh_instances = part_mesh_instances.get(&main_entity)?;
        let mesh_instance = mesh_instances.get(&index)?;
        Some((
            mesh2d_uniform_from_components(mesh_instance, 0),
            Some((
                mesh_instance.material_bind_group_id,
                mesh_instance.mesh_asset_id,
            )),
        ))
    }
}
#[derive(ShaderType, Clone, Copy)]
pub struct PartMesh2dUniform {
    // Affine 4x3 matrix transposed to 3x4
    pub world_from_local: [Vec4; 3],
    // 3x3 matrix packed in mat2x4 and f32 as:
    //   [0].xyz, [1].x,
    //   [1].yz, [2].xy
    //   [2].z
    pub local_from_world_transpose_a: [Vec4; 2],
    pub local_from_world_transpose_b: f32,
    pub flags: u32,
    pub tag: u32,

    pub mult_color: Vec4,
    pub add_color: Vec4,
}

fn mesh2d_uniform_from_components(
    mesh_instance: &RenderPartMesh2dInstance,
    tag: u32,
) -> PartMesh2dUniform {
    let (mesh_transforms, color_transform) =
        (&mesh_instance.transforms, mesh_instance.color_transform);
    let (local_from_world_transpose_a, local_from_world_transpose_b) =
        mesh_transforms.world_from_local.inverse_transpose_3x3();
    PartMesh2dUniform {
        world_from_local: mesh_transforms.world_from_local.to_transpose(),
        local_from_world_transpose_a,
        local_from_world_transpose_b,
        flags: mesh_transforms.flags,
        tag,
        mult_color: color_transform.mult_color,
        add_color: color_transform.add_color,
    }
}

// 下面为批处理相关

/// 该 trait 因包含一个没有接收者（receiver）的方法 get_batch_part_data 而不支持动态分发（dyn-compatible）—— 即无法作为 dyn Trait 类型使用。
/// 这是一个用于通过阶段项目（phase items）获取批处理绘制命令所需数据的 trait。
/// 这是一个简化版本，仅支持排序（sorting）而不支持分箱（binning），且仅支持 CPU 处理，不涉及 GPU 预处理。
/// 若需要这些更复杂的特性，请参考  [`GetFullBatchData`].
pub trait GetBatchPartData {
    /// The system parameters [`GetBatchData::get_batch_data`] needs in
    /// order to compute the batch data.
    type Param: SystemParam + 'static;
    /// Data used for comparison between phase items. If the pipeline id, draw
    /// function id, per-instance data buffer dynamic offset and this data
    /// matches, the draws can be batched.
    type CompareData: PartialEq;
    /// The per-instance data to be inserted into the
    /// [`crate::render_resource::GpuArrayBuffer`] containing these data for all
    /// instances.
    type BufferData: GpuArrayBufferable + Sync + Send + 'static;
    /// Get the per-instance data to be inserted into the
    /// [`crate::render_resource::GpuArrayBuffer`]. If the instance can be
    /// batched, also return the data used for comparison when deciding whether
    /// draws can be batched, else return None for the `CompareData`.
    ///
    /// This is only called when building instance data on CPU. In the GPU
    /// instance data building path, we use
    /// [`GetFullBatchData::get_index_and_compare_data`] instead.
    fn get_batch_part_data(
        param: &SystemParamItem<Self::Param>,
        query_item: (Entity, MainEntity, usize),
    ) -> Option<(Self::BufferData, Option<Self::CompareData>)>;
}

/// 在禁用 GPU 实例缓冲区构建时，对已排序的渲染阶段（sorted render phase）中的项目进行批处理。
/// 核心是对比每个阶段项目的绘制元数据，尝试将多个绘制操作合并为一个批处理任务。
fn batch_and_prepare_part_sorted_render_phase<I, GBPD>(
    batched_instance_buffer: ResMut<BatchedInstanceBuffer<GBPD::BufferData>>,
    mut phases: ResMut<ViewSortedRenderPhases<I>>,
    param: StaticSystemParam<GBPD::Param>,
) where
    I: CachedRenderPipelinePhaseItem + SortedPhaseItem + PartPhaseItem,
    GBPD: GetBatchPartData,
{
    let system_param_item = param.into_inner();

    // We only process CPU-built batch data in this function.
    let batched_instance_buffer = batched_instance_buffer.into_inner();
    for phase in phases.values_mut() {
        batch_and_prepare_sorted_render_phase::<I, GBPD>(phase, |item| {
            let (buffer_data, compare_data) = GBPD::get_batch_part_data(
                &system_param_item,
                (item.entity(), item.main_entity(), item.extracted_index()),
            )?;

            let buffer_index = batched_instance_buffer.push(buffer_data);

            let index = buffer_index.index;
            let (batch_range, extra_index) = item.batch_range_and_extra_index_mut();
            *batch_range = index..index + 1;
            *extra_index = PhaseItemExtraIndex::maybe_dynamic_offset(buffer_index.dynamic_offset);

            compare_data
        });
    }
}
/// 使两个绘制命令可合并所需的相等数据（即 “可合并元数据”），
///
/// 其定义基于以下假设：
/// - 进入渲染阶段的实体必须已完成资源准备（如管线、材质、网格等均已加载并适配 GPU），确保绘制命令的基础资源是就绪且一致的。
/// - 对于特定绘制函数，同一渲染阶段（phase）内的视图绑定（View bindings，如视图投影矩阵等相机相关数据）是固定的
/// — 因为渲染阶段通常按视图（如每个相机对应一个阶段）划分，避免跨视图的绑定差异影响合并。
/// - `batch_and_prepare_render_phase` 是唯一执行批处理的系统，且全权负责准备每个对象的绘制数据。
///   因此，网格绑定（mesh binding）和动态偏移量（dynamic offsets）的变化仅由该系统导致
///   （例如：因 uniform 缓冲区最大绑定尺寸限制，需将数据拆分到同一缓冲区的不同绑定位置）。
#[derive(PartialEq)]
struct BatchMeta<T: PartialEq> {
    /// The pipeline id encompasses all pipeline configuration including vertex
    /// buffers and layouts, shaders and their specializations, bind group
    /// layouts, etc.
    pipeline_id: CachedRenderPipelineId,
    /// The draw function id defines the `RenderCommands` that are called to
    /// set the pipeline and bindings, and make the draw command
    draw_function_id: DrawFunctionId,
    dynamic_offset: Option<NonMaxU32>,
    user_data: T,
}

impl<T: PartialEq> BatchMeta<T> {
    fn new(item: &impl CachedRenderPipelinePhaseItem, user_data: T) -> Self {
        BatchMeta {
            pipeline_id: item.cached_pipeline(),
            draw_function_id: item.draw_function(),
            dynamic_offset: match item.extra_index() {
                PhaseItemExtraIndex::DynamicOffset(dynamic_offset) => {
                    NonMaxU32::new(dynamic_offset)
                }
                PhaseItemExtraIndex::None | PhaseItemExtraIndex::IndirectParametersIndex { .. } => {
                    None
                }
            },
            user_data,
        }
    }
}

/// 对已排序的渲染阶段（sorted render phase）中的项目进行批处理。
/// 具体来说，就是对比每个阶段项目的绘制所需元数据（metadata），并尝试将多个绘制操作合并为一个批处理任务。
/// 从 [`gpu_preprocessing::batch_and_prepare_sorted_render_phase`] 和
/// [`no_gpu_preprocessing::batch_and_prepare_sorted_render_phase`].
/// 提取的通用代码。
fn batch_and_prepare_sorted_render_phase<I, GBPD>(
    phase: &mut SortedRenderPhase<I>,
    mut process_item: impl FnMut(&mut I) -> Option<GBPD::CompareData>,
) where
    I: CachedRenderPipelinePhaseItem + SortedPhaseItem,
    GBPD: GetBatchPartData,
{
    let items = phase.items.iter_mut().map(|item| {
        let batch_data = match process_item(item) {
            Some(compare_data) if I::AUTOMATIC_BATCHING => Some(BatchMeta::new(item, compare_data)),
            _ => None,
        };
        (item.batch_range_mut(), batch_data)
    });

    items.reduce(|(start_range, prev_batch_meta), (range, batch_meta)| {
        if batch_meta.is_some() && prev_batch_meta == batch_meta {
            start_range.end = range.end;
            (start_range, prev_batch_meta)
        } else {
            (range, batch_meta)
        }
    });
}

/// A system that clears out the [`BatchedInstanceBuffer`] for the frame.
///
/// This needs to run before the CPU batched instance buffers are used.
fn clear_batched_cpu_part_instance_buffers<GBPD>(
    cpu_batched_instance_buffer: Option<ResMut<BatchedInstanceBuffer<GBPD::BufferData>>>,
) where
    GBPD: GetBatchPartData,
{
    if let Some(mut cpu_batched_instance_buffer) = cpu_batched_instance_buffer {
        cpu_batched_instance_buffer.clear();
    }
}

fn write_batched_part_instance_buffer<GBPD>(
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    mut cpu_batched_instance_buffer: ResMut<BatchedInstanceBuffer<GBPD::BufferData>>,
) where
    GBPD: GetBatchPartData,
{
    cpu_batched_instance_buffer.write_buffer(&render_device, &render_queue);
}
