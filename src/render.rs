use bevy::{
    app::{App, Plugin},
    asset::{AssetId, Handle},
    ecs::entity::EntityHashMap,
    prelude::{Entity, GlobalTransform, Query, ResMut, Resource, Shader, ViewVisibility},
    render::{Extract, ExtractSchedule, RenderApp},
};

use glam::Vec2;

use crate::{assets::SwfMovie, bundle::SwfSprite};
/// 使用UUID指定,SWF着色器Handle
pub const SWF_HANDLE: Handle<Shader> =
    Handle::weak_from_u128(251354789657743035148351631714426867038);

pub(crate) mod commands;
mod pipeline;

pub struct ExtractedSWFShape {
    /// 用于定位图形的全局变换
    pub global_transform: GlobalTransform,
    /// 用于修改图形的大小
    pub custom_size: Option<Vec2>,
    /// 图形的SWf资源ID,
    pub swf_handle_id: AssetId<SwfMovie>,
    /// 沿着 x 轴翻转。
    pub flip_x: bool,
    /// 沿着 y 轴翻转。
    pub flip_y: bool,
    /// 用于定位图形的锚点
    pub anchor: Vec2,
}
#[derive(Resource, Default)]
pub struct ExtractedSWFShapes {
    pub swf_sprites: EntityHashMap<ExtractedSWFShape>,
}

pub struct FlashRenderPlugin;

impl Plugin for FlashRenderPlugin {
    fn build(&self, app: &mut App) {}

    fn finish(&self, app: &mut App) {
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app
                .init_resource::<ExtractedSWFShapes>()
                .add_systems(ExtractSchedule, extract_swf_sprite);
        }
    }
}

fn extract_swf_sprite(
    mut extract_swf_sprites: ResMut<ExtractedSWFShapes>,
    swf_query: Extract<
        Query<(
            Entity,
            &SwfSprite,
            &Handle<SwfMovie>,
            &ViewVisibility,
            &GlobalTransform,
        )>,
    >,
) {
    extract_swf_sprites.swf_sprites.clear();
    for (entity, swf_sprite, swf_handle, view_visibility, global_transform) in swf_query.iter() {
        if !view_visibility.get() {
            continue;
        }
        extract_swf_sprites.swf_sprites.insert(
            entity,
            ExtractedSWFShape {
                global_transform: *global_transform,
                custom_size: swf_sprite.custom_size,
                swf_handle_id: swf_handle.id(),
                flip_x: swf_sprite.flip_x,
                flip_y: swf_sprite.flip_y,
                anchor: Vec2::default(),
            },
        );
    }
}
