use bevy::{
    asset::{weak_handle, Handle},
    core_pipeline::fullscreen_vertex_shader::fullscreen_shader_vertex_state,
    ecs::{component::Component, resource::Resource, world::FromWorld},
    image::BevyDefault,
    render::{
        extract_component::ExtractComponent,
        render_graph::{RenderLabel, ViewNode},
        render_resource::{
            binding_types::{sampler, texture_2d},
            BindGroupLayout, BindGroupLayoutEntries, CachedRenderPipelineId, ColorTargetState,
            ColorWrites, FragmentState, MultisampleState, PipelineCache, PrimitiveState,
            RenderPipelineDescriptor, Sampler, SamplerBindingType, SamplerDescriptor, Shader,
            ShaderStages, TextureFormat, TextureSampleType, VertexState,
        },
        renderer::RenderDevice,
        view::ViewTarget,
    },
};
use swf::{Rectangle, Twips};

pub const BLUR_FILTER_SHADER_HANDLE: Handle<Shader> =
    weak_handle!("f59e3d1c-7a24-4b8c-82a3-1d94e6f2c705");

#[derive(Debug, Clone, PartialEq)]
pub enum Filter {
    BevelFilter(swf::BevelFilter),
    BlurFilter(swf::BlurFilter),
    ColorMatrixFilter(swf::ColorMatrixFilter),
    ConvolutionFilter(swf::ConvolutionFilter),
    DropShadowFilter(swf::DropShadowFilter),
    GlowFilter(swf::GlowFilter),
    GradientBevelFilter(swf::GradientFilter),
    GradientGlowFilter(swf::GradientFilter),
}

impl Filter {
    pub fn scale(&mut self, x: f32, y: f32) {
        match self {
            Filter::BevelFilter(filter) => filter.scale(x, y),
            Filter::BlurFilter(filter) => filter.scale(x, y),
            Filter::DropShadowFilter(filter) => filter.scale(x, y),
            Filter::GlowFilter(filter) => filter.scale(x, y),
            Filter::GradientBevelFilter(filter) => filter.scale(x, y),
            Filter::GradientGlowFilter(filter) => filter.scale(x, y),
            _ => {}
        }
    }

    pub fn calculate_dest_rect(&self, source_rect: Rectangle<Twips>) -> Rectangle<Twips> {
        match self {
            Filter::BlurFilter(filter) => filter.calculate_dest_rect(source_rect),
            Filter::GlowFilter(filter) => filter.calculate_dest_rect(source_rect),
            Filter::DropShadowFilter(filter) => filter.calculate_dest_rect(source_rect),
            Filter::BevelFilter(filter) => filter.calculate_dest_rect(source_rect),
            _ => source_rect,
        }
    }

    /// Checks if this filter is impotent.
    /// Impotent filters will have no effect if applied, and can safely be skipped.
    pub fn impotent(&self) -> bool {
        // TODO: There's more cases here, find them!
        match self {
            Filter::BlurFilter(filter) => filter.impotent(),
            Filter::ColorMatrixFilter(filter) => filter.impotent(),
            _ => false,
        }
    }
}

impl From<&swf::Filter> for Filter {
    fn from(value: &swf::Filter) -> Self {
        match value {
            swf::Filter::DropShadowFilter(filter) => {
                Filter::DropShadowFilter(filter.as_ref().to_owned())
            }
            swf::Filter::BlurFilter(filter) => Filter::BlurFilter(filter.as_ref().to_owned()),
            swf::Filter::GlowFilter(filter) => Filter::GlowFilter(filter.as_ref().to_owned()),
            swf::Filter::BevelFilter(filter) => Filter::BevelFilter(filter.as_ref().to_owned()),
            swf::Filter::GradientGlowFilter(filter) => {
                Filter::GradientGlowFilter(filter.as_ref().to_owned())
            }
            swf::Filter::ConvolutionFilter(filter) => {
                Filter::ConvolutionFilter(filter.as_ref().to_owned())
            }
            swf::Filter::ColorMatrixFilter(filter) => {
                Filter::ColorMatrixFilter(filter.as_ref().to_owned())
            }
            swf::Filter::GradientBevelFilter(filter) => {
                Filter::GradientBevelFilter(filter.as_ref().to_owned())
            }
        }
    }
}

#[derive(Component, Clone, Debug, Default, ExtractComponent)]
pub struct FlashFilters(Vec<Filter>);

/// Flash Filter处理标签
#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderLabel)]
pub struct FlashFilterLabel;

/// Flash 滤镜后期处理节点
#[derive(Default)]
pub struct FlashFilterNode;

impl ViewNode for FlashFilterNode {
    type ViewQuery = (&'static ViewTarget, &'static FlashFilters);

    fn run<'w>(
        &self,
        graph: &mut bevy::render::render_graph::RenderGraphContext,
        render_context: &mut bevy::render::renderer::RenderContext<'w>,
        (view_target, flash_filters): bevy::ecs::query::QueryItem<'w, Self::ViewQuery>,
        world: &'w bevy::ecs::world::World,
    ) -> Result<(), bevy::render::render_graph::NodeRunError> {
        let filter_pipeline_cache = world.resource::<PipelineCache>();
        let blur_filter_pipeline = world.resource::<FlashFilterPipeline>();

        let Some(blur_filter_pipeline) =
            filter_pipeline_cache.get_render_pipeline(blur_filter_pipeline.blur_filter_pipeline_id)
        else {
            return Ok(());
        };

        for filter in flash_filters.0.iter() {
            // let post_process = view_target.post_process_write();

            match filter {
                Filter::BlurFilter(blur_filter) => {
                    dbg!(blur_filter.blur_x);
                    dbg!(blur_filter.blur_y);
                }
                _ => {}
            }
        }

        Ok(())
    }
}

#[derive(Resource)]
struct FlashFilterPipeline {
    blur_filter_layout: BindGroupLayout,
    blur_filter_pipeline_id: CachedRenderPipelineId,
    sampler: Sampler,
}

impl FromWorld for FlashFilterPipeline {
    fn from_world(world: &mut bevy::ecs::world::World) -> Self {
        let render_device = world.resource::<RenderDevice>();

        let blur_filter_layout = render_device.create_bind_group_layout(
            "flash_blur_filter_bind_group_layout",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
                ),
            ),
        );

        let sampler = render_device.create_sampler(&SamplerDescriptor::default());

        let blur_filter_pipeline_id =
            world
                .resource_mut::<PipelineCache>()
                .queue_render_pipeline(RenderPipelineDescriptor {
                    label: Some("flash_blur_filter_pipeline".into()),
                    layout: vec![blur_filter_layout.clone()],
                    push_constant_ranges: vec![],
                    vertex: fullscreen_shader_vertex_state(),
                    fragment: Some(FragmentState {
                        shader: BLUR_FILTER_SHADER_HANDLE,
                        shader_defs: vec![],
                        entry_point: "fragment".into(),
                        targets: vec![Some(ColorTargetState {
                            format: TextureFormat::bevy_default(),
                            blend: None,
                            write_mask: ColorWrites::ALL,
                        })],
                    }),
                    primitive: PrimitiveState::default(),
                    depth_stencil: None,
                    multisample: MultisampleState::default(),
                    zero_initialize_workgroup_memory: false,
                });
        Self {
            blur_filter_layout,
            blur_filter_pipeline_id,
            sampler,
        }
    }
}
