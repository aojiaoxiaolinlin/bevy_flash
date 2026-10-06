use super::{OffscreenViewTarget, RenderOffscreenTexture};
use crate::{
    material::{BitmapMaterial, GradientMaterial},
    sampling::VabSkin,
    vab_asset::{
        CommandList, MeshMaterial, VabAsset, VabAssetHandle, VabBlendMode, VabCommand,
        compute_command_bounds,
    },
    vab_player::VabPlayer,
};
use bevy::{
    asset::AssetId,
    prelude::*,
    render::{Extract, extract_component::ExtractComponent, sync_world::RenderEntity},
};

#[derive(Clone)]
pub struct Draw {
    pub mesh: AssetId<Mesh>,
    pub local_bounds: [f32; 4],
    pub texture: Option<AssetId<Image>>,
    pub transform: vatf::animation::AnimTransform,
    pub uv: Mat4,
    pub kind: u32,
    pub gradient: [f32; 3],
}

#[derive(Clone)]
pub enum Op {
    Draw(Draw),
    Mask {
        mask: Vec<Op>,
        content: Vec<Op>,
        bounds: [f32; 4],
    },
    Layer {
        ops: Vec<Op>,
        bounds: [f32; 4],
        filters: Vec<vatf::animation::AnimFilter>,
        blend: VabBlendMode,
    },
}

#[derive(Component, Clone, ExtractComponent, Default)]
pub struct ExtractedFrame {
    pub ops: Vec<Op>,
    pub error: Option<String>,
}

/// Extract only the selected baked frame, resolving material handles while the main assets exist.
#[allow(clippy::type_complexity)]
pub fn extract_frames(
    mut commands: Commands,
    query: Extract<
        Query<
            (
                RenderEntity,
                &VabAssetHandle,
                &VabPlayer,
                Option<&VabSkin>,
                &OffscreenViewTarget,
                Option<&super::RasterOnce>,
            ),
            With<RenderOffscreenTexture>,
        >,
    >,
    assets: Extract<Res<Assets<VabAsset>>>,
    bitmaps: Extract<Res<Assets<BitmapMaterial>>>,
    gradients: Extract<Res<Assets<GradientMaterial>>>,
) {
    for (entity, handle, player, skin, target, once) in &query {
        if once.is_some_and(|once| once.complete()) {
            commands
                .entity(entity)
                .remove::<(ExtractedFrame, super::gpu::PreparedFrame)>();
            continue;
        }
        let Some(asset) = assets.get(&handle.0) else {
            commands.entity(entity).remove::<ExtractedFrame>();
            continue;
        };
        let result = asset
            .sample(
                player.clip(),
                player.current_frame,
                skin.unwrap_or(&VabSkin::default()),
                target.scale.extend(1.0),
            )
            .and_then(|commands| resolve(asset, &commands, &bitmaps, &gradients));
        let frame = match result {
            Ok(ops) => ExtractedFrame { ops, error: None },
            Err(error) => ExtractedFrame {
                ops: vec![],
                error: Some(error.to_string()),
            },
        };
        commands.entity(entity).insert(frame);
    }
}

pub(super) fn resolve(
    asset: &VabAsset,
    commands: &CommandList,
    bitmaps: &Assets<BitmapMaterial>,
    gradients: &Assets<GradientMaterial>,
) -> anyhow::Result<Vec<Op>> {
    let mut index = 0;
    let ops = resolve_until(
        asset,
        &commands.commands,
        &mut index,
        None,
        bitmaps,
        gradients,
    )?;
    anyhow::ensure!(index == commands.commands.len(), "unexpected mask command");
    Ok(ops)
}

#[derive(Clone, Copy)]
enum Stop {
    Activate,
    Deactivate,
    Pop,
}

fn is_stop(command: &VabCommand, stop: Stop) -> bool {
    matches!(
        (command, stop),
        (VabCommand::ActivateMask, Stop::Activate)
            | (VabCommand::DeactivateMask, Stop::Deactivate)
            | (VabCommand::PopMask, Stop::Pop)
    )
}

fn resolve_until(
    asset: &VabAsset,
    commands: &[VabCommand],
    index: &mut usize,
    stop: Option<Stop>,
    bitmaps: &Assets<BitmapMaterial>,
    gradients: &Assets<GradientMaterial>,
) -> anyhow::Result<Vec<Op>> {
    use anyhow::Context;
    let mut ops = Vec::new();
    while let Some(command) = commands.get(*index) {
        if stop.is_some_and(|stop| is_stop(command, stop)) {
            return Ok(ops);
        }
        *index += 1;
        ops.push(match command {
            VabCommand::RenderShape { handle, transform } => {
                let mesh = asset
                    .render_meshes
                    .get(*handle)
                    .context("invalid shape handle")?;
                let mut draw = Draw {
                    mesh: mesh.mesh.id(),
                    local_bounds: mesh.local_bounds,
                    texture: None,
                    transform: *transform,
                    uv: Mat4::IDENTITY,
                    kind: 0,
                    gradient: [0.0; 3],
                };
                match &mesh.material {
                    MeshMaterial::Color => {}
                    MeshMaterial::Bitmap(handle) => {
                        let material = bitmaps.get(handle).context("bitmap material not loaded")?;
                        draw.texture = Some(material.texture.id());
                        draw.uv = material.texture_transform;
                        draw.kind = 1;
                    }
                    MeshMaterial::Gradient(handle) => {
                        let material = gradients
                            .get(handle)
                            .context("gradient material not loaded")?;
                        draw.texture = Some(material.texture.id());
                        draw.uv = material.texture_transform;
                        draw.kind = 2;
                        draw.gradient = [
                            material.gradient.focal_point,
                            material.gradient.shape as f32,
                            material.gradient.repeat as f32,
                        ];
                    }
                }
                Op::Draw(draw)
            }
            VabCommand::ApplyFilter {
                commands,
                filters,
                bounds,
            } => Op::Layer {
                ops: resolve(asset, commands, bitmaps, gradients)?,
                bounds: *bounds,
                filters: filters.clone(),
                blend: VabBlendMode::Normal,
            },
            VabCommand::Blend(commands, blend) => {
                // Filter and blend share the same isolation target.
                if let [
                    VabCommand::ApplyFilter {
                        commands,
                        filters,
                        bounds,
                    },
                ] = commands.commands.as_slice()
                {
                    Op::Layer {
                        ops: resolve(asset, commands, bitmaps, gradients)?,
                        bounds: *bounds,
                        filters: filters.clone(),
                        blend: *blend,
                    }
                } else {
                    Op::Layer {
                        ops: resolve(asset, commands, bitmaps, gradients)?,
                        bounds: compute_command_bounds(asset, commands),
                        filters: vec![],
                        blend: *blend,
                    }
                }
            }
            VabCommand::PushMask => {
                let mask = resolve_until(
                    asset,
                    commands,
                    index,
                    Some(Stop::Activate),
                    bitmaps,
                    gradients,
                )?;
                anyhow::ensure!(
                    matches!(commands.get(*index), Some(VabCommand::ActivateMask)),
                    "mask is missing ActivateMask"
                );
                *index += 1;
                let content = resolve_until(
                    asset,
                    commands,
                    index,
                    Some(Stop::Deactivate),
                    bitmaps,
                    gradients,
                )?;
                anyhow::ensure!(
                    matches!(commands.get(*index), Some(VabCommand::DeactivateMask)),
                    "mask is missing DeactivateMask"
                );
                *index += 1;
                // The command stream redraws the mask to decrement stencil. Alpha masking
                // uses the first copy as a texture, so parse and discard this clear copy.
                let _clear_mask =
                    resolve_until(asset, commands, index, Some(Stop::Pop), bitmaps, gradients)?;
                anyhow::ensure!(
                    matches!(commands.get(*index), Some(VabCommand::PopMask)),
                    "mask is missing PopMask"
                );
                *index += 1;
                let bounds = intersect_bounds(op_bounds(&mask), op_bounds(&content));
                Op::Mask {
                    mask,
                    content,
                    bounds,
                }
            }
            VabCommand::ActivateMask | VabCommand::DeactivateMask | VabCommand::PopMask => {
                anyhow::bail!("unexpected mask command")
            }
        });
    }
    anyhow::ensure!(stop.is_none(), "unterminated mask command sequence");
    Ok(ops)
}

pub(super) fn op_bounds(ops: &[Op]) -> [f32; 4] {
    let mut bounds = [
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    ];
    for op in ops {
        let rect = match op {
            Op::Draw(draw) => {
                let r = draw.local_bounds;
                let m = draw.transform.matrix;
                let mut transformed = [
                    f32::INFINITY,
                    f32::INFINITY,
                    f32::NEG_INFINITY,
                    f32::NEG_INFINITY,
                ];
                for [x, y] in [[r[0], r[1]], [r[0], r[3]], [r[2], r[1]], [r[2], r[3]]] {
                    let x_out = m.a * x + m.c * y + m.tx;
                    let y_out = m.b * x + m.d * y + m.ty;
                    transformed[0] = transformed[0].min(x_out);
                    transformed[1] = transformed[1].min(y_out);
                    transformed[2] = transformed[2].max(x_out);
                    transformed[3] = transformed[3].max(y_out);
                }
                [
                    transformed[0],
                    transformed[1],
                    transformed[2] - transformed[0],
                    transformed[3] - transformed[1],
                ]
            }
            Op::Mask { bounds, .. } | Op::Layer { bounds, .. } => *bounds,
        };
        bounds[0] = bounds[0].min(rect[0]);
        bounds[1] = bounds[1].min(rect[1]);
        bounds[2] = bounds[2].max(rect[0] + rect[2]);
        bounds[3] = bounds[3].max(rect[1] + rect[3]);
    }
    if !bounds[0].is_finite() {
        [0.0; 4]
    } else {
        [
            bounds[0],
            bounds[1],
            (bounds[2] - bounds[0]).max(0.0),
            (bounds[3] - bounds[1]).max(0.0),
        ]
    }
}

fn intersect_bounds(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let x = a[0].max(b[0]);
    let y = a[1].max(b[1]);
    let right = (a[0] + a[2]).min(b[0] + b[2]);
    let bottom = (a[1] + a[3]).min(b[1] + b[3]);
    [x, y, (right - x).max(0.0), (bottom - y).max(0.0)]
}
