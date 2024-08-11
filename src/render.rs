use bevy::{
    app::{App, Plugin, PostUpdate},
    asset::{Assets, Handle},
    ecs::entity::EntityHashMap,
    prelude::{IntoSystemConfigs, Query, ResMut, Resource, Shader},
    render::{ExtractSchedule, RenderApp},
};
use ruffle_render::transform::Transform;
use tessellator::ShapeTessellator;

use crate::{
    assets::SwfMovie,
    swf::{
        characters::Character,
        display_object::{render_base, TDisplayObject},
    },
    system::FlashSystem,
};

/// 使用UUID指定,SWF着色器Handle
pub const SWF_HANDLE: Handle<Shader> =
    Handle::weak_from_u128(251354789657743035148351631714426867038);

pub(crate) mod commands;
mod pipeline;
mod tessellator;

pub struct ExtractedSWFShape {}
#[derive(Resource, Default)]
pub struct ExtractedSWFShapes {
    pub swf_sprites: EntityHashMap<ExtractedSWFShape>,
}

pub struct FlashRenderPlugin;

impl Plugin for FlashRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostUpdate, pre_render);
    }
}

/// 从SWF中提取形状，提前转为`Mesh2d`。swf加载完成后执行 。

fn pre_render(query: Query<&Handle<SwfMovie>>, mut swf_movie: ResMut<Assets<SwfMovie>>) {
    for swf_handle in query.iter() {
        if let Some(swf_movie) = swf_movie.get_mut(swf_handle.id()) {
            let root_movie_clip = swf_movie.root_movie_clip.clone();
            render_base(root_movie_clip.into(), Transform::default());
        }
    }
}
