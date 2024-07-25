use bevy::{
    ecs::{
        query::ROQueryItem,
        system::{lifetimeless::Read, SystemParamItem},
    },
    prelude::Component,
    render::{
        render_phase::{
            PhaseItem, RenderCommand, RenderCommandResult, SetItemPipeline, TrackedRenderPass,
        },
        render_resource::BindGroup,
        view::ViewUniformOffset,
    },
};

pub type DrawFlashCommand = (SetItemPipeline, SetFlashViewBindGroup<0>);

#[derive(Component)]
pub struct FlashViewBindGroup {
    value: BindGroup,
}

pub struct SetFlashViewBindGroup<const T: usize>;

impl<P: PhaseItem, const I: usize> RenderCommand<P> for SetFlashViewBindGroup<I> {
    type Param = ();

    type ViewQuery = (Read<ViewUniformOffset>, Read<FlashViewBindGroup>);

    type ItemQuery = ();

    fn render<'w>(
        _item: &P,
        (view_uniform, flash_view_bind_group): ROQueryItem<'w, Self::ViewQuery>,
        _entity: Option<ROQueryItem<'w, Self::ItemQuery>>,
        _param: SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        pass.set_bind_group(I, &flash_view_bind_group.value, &[view_uniform.offset]);
        RenderCommandResult::Success
    }
}
