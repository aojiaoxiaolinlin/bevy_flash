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
    pub ramp_v: f32,
    pub blend: VabBlendMode,
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
#[extract_app(bevy::render::RenderApp)]
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
            .and_then(|commands| resolve(asset, &commands, &bitmaps, &gradients))
            .map(|ops| flatten_direct_fixed_blends(ops, false));
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
    let mut ops = Vec::new();
    while let Some(command) = commands.get(*index) {
        if stop.is_some_and(|stop| is_stop(command, stop)) {
            return Ok(ops);
        }
        *index += 1;
        ops.push(match command {
            VabCommand::RenderShape { handle, transform } => {
                let mesh = anyhow::Context::context(
                    asset.render_meshes.get(*handle),
                    "invalid shape handle",
                )?;
                let mut draw = Draw {
                    mesh: mesh.mesh.id(),
                    local_bounds: mesh.local_bounds,
                    texture: None,
                    transform: *transform,
                    uv: Mat4::IDENTITY,
                    kind: 0,
                    gradient: [0.0; 3],
                    ramp_v: 0.0,
                    blend: VabBlendMode::Normal,
                };
                match &mesh.material {
                    MeshMaterial::Color => {}
                    MeshMaterial::Bitmap(handle) => {
                        let material = anyhow::Context::context(
                            bitmaps.get(handle),
                            "bitmap material not loaded",
                        )?;
                        draw.texture = Some(material.texture.id());
                        draw.uv = material.texture_transform;
                        draw.kind = 1;
                    }
                    MeshMaterial::Gradient(handle) => {
                        let material = anyhow::Context::context(
                            gradients.get(handle),
                            "gradient material not loaded",
                        )?;
                        draw.texture = Some(material.texture.id());
                        draw.uv = material.texture_transform;
                        draw.kind = 2;
                        draw.gradient = [
                            material.gradient.focal_point,
                            material.gradient.shape as f32,
                            material.gradient.repeat as f32,
                        ];
                        draw.ramp_v = material.ramp_v;
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

fn draw_transformed_aabb(draw: &Draw) -> [f32; 4] {
    let r = draw.local_bounds;
    let m = draw.transform.matrix;
    let mut bounds = [
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    ];
    for [x, y] in [[r[0], r[1]], [r[0], r[3]], [r[2], r[1]], [r[2], r[3]]] {
        let x_out = m.a * x + m.c * y + m.tx;
        let y_out = m.b * x + m.d * y + m.ty;
        bounds[0] = bounds[0].min(x_out);
        bounds[1] = bounds[1].min(y_out);
        bounds[2] = bounds[2].max(x_out);
        bounds[3] = bounds[3].max(y_out);
    }
    bounds
}

fn direct_fixed_blend_target(
    blend: VabBlendMode,
    allow_lighten_approx: bool,
) -> Option<VabBlendMode> {
    match blend {
        VabBlendMode::Normal | VabBlendMode::Layer => Some(VabBlendMode::Normal),
        VabBlendMode::Add => Some(VabBlendMode::Add),
        VabBlendMode::Subtract => Some(VabBlendMode::Subtract),
        VabBlendMode::Screen => Some(VabBlendMode::Screen),
        VabBlendMode::Lighten if allow_lighten_approx => Some(VabBlendMode::Lighten),
        _ => None,
    }
}

fn can_direct_draw_fixed_blend(
    ops: &[Op],
    filters: &[vatf::animation::AnimFilter],
    blend: VabBlendMode,
    allow_lighten_approx: bool,
) -> Option<VabBlendMode> {
    if !filters.is_empty() || ops.is_empty() {
        return None;
    }
    let target_blend = direct_fixed_blend_target(blend, allow_lighten_approx)?;
    if !ops
        .iter()
        .all(|op| matches!(op, Op::Draw(draw) if draw.blend == VabBlendMode::Normal))
    {
        return None;
    }
    if ops.len() == 1 {
        return Some(target_blend);
    }
    if ops.len() > 16 {
        return None;
    }
    // Multiple draws can bypass isolation only when their AABBs are strictly separated
    // (with a 2px guard for MSAA/subpixel coverage), guaranteeing zero internal overlap.
    const DISJOINT_MARGIN: f32 = 2.0;
    let boxes: Vec<[f32; 4]> = ops
        .iter()
        .map(|op| match op {
            Op::Draw(draw) => draw_transformed_aabb(draw),
            _ => unreachable!(),
        })
        .collect();
    if boxes.iter().any(|b| !b.iter().all(|v| v.is_finite())) {
        return None;
    }
    for i in 0..boxes.len() {
        for j in (i + 1)..boxes.len() {
            let a = boxes[i];
            let b = boxes[j];
            let separated = a[2] + DISJOINT_MARGIN <= b[0]
                || b[2] + DISJOINT_MARGIN <= a[0]
                || a[3] + DISJOINT_MARGIN <= b[1]
                || b[3] + DISJOINT_MARGIN <= a[1];
            if !separated {
                return None;
            }
        }
    }
    Some(target_blend)
}

/// Eliminate intermediate render targets for unfiltered fixed-function blends
/// whose children cannot self-overlap (single mesh or strictly disjoint AABBs).
///
/// `allow_top_level_lighten` should be `true` for Camera2d scene instances (where
/// top-level Lighten already uses hardware `Max`) and `false` for offscreen/nested
/// targets (which run exact Flash straight-color Lighten in `complex_blend.wesl`).
pub(super) fn flatten_direct_fixed_blends(ops: Vec<Op>, allow_top_level_lighten: bool) -> Vec<Op> {
    let mut out = Vec::with_capacity(ops.len());
    for op in ops {
        match op {
            Op::Draw(draw) => out.push(Op::Draw(draw)),
            Op::Mask {
                mask,
                content,
                bounds,
            } => out.push(Op::Mask {
                mask: flatten_direct_fixed_blends(mask, false),
                content: flatten_direct_fixed_blends(content, false),
                bounds,
            }),
            Op::Layer {
                ops: children,
                bounds,
                filters,
                blend,
            } => {
                let children = flatten_direct_fixed_blends(children, false);
                if let Some(target_blend) =
                    can_direct_draw_fixed_blend(&children, &filters, blend, allow_top_level_lighten)
                {
                    for child in children {
                        if let Op::Draw(mut draw) = child {
                            draw.blend = target_blend;
                            out.push(Op::Draw(draw));
                        }
                    }
                } else {
                    out.push(Op::Layer {
                        ops: children,
                        bounds,
                        filters,
                        blend,
                    });
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use vatf::animation::{AnimBlurFilter, AnimFilter, AnimMatrix, AnimTransform};

    fn dummy_draw(x0: f32, y0: f32, x1: f32, y1: f32) -> Op {
        Op::Draw(Draw {
            mesh: AssetId::default(),
            local_bounds: [x0, y0, x1, y1],
            texture: None,
            transform: AnimTransform {
                matrix: AnimMatrix::IDENTITY,
                ..Default::default()
            },
            uv: Mat4::IDENTITY,
            kind: 0,
            gradient: [0.0; 3],
            ramp_v: 0.0,
            blend: VabBlendMode::Normal,
        })
    }

    #[test]
    fn flattens_single_and_disjoint_fixed_blends_but_keeps_overlapping_and_filtered() {
        // 1. Single-mesh unfiltered Add flattens to direct Draw with Add blend.
        let single_add = vec![Op::Layer {
            ops: vec![dummy_draw(0.0, 0.0, 10.0, 10.0)],
            bounds: [0.0, 0.0, 10.0, 10.0],
            filters: vec![],
            blend: VabBlendMode::Add,
        }];
        let flat = flatten_direct_fixed_blends(single_add, true);
        assert!(matches!(
            flat.as_slice(),
            [Op::Draw(d)] if d.blend == VabBlendMode::Add
        ));

        // 2. Disjoint two-mesh Add flattens; overlapping two-mesh Add stays isolated.
        let disjoint = vec![Op::Layer {
            ops: vec![
                dummy_draw(0.0, 0.0, 10.0, 10.0),
                dummy_draw(20.0, 0.0, 30.0, 10.0),
            ],
            bounds: [0.0, 0.0, 30.0, 10.0],
            filters: vec![],
            blend: VabBlendMode::Add,
        }];
        assert_eq!(flatten_direct_fixed_blends(disjoint, true).len(), 2);

        let overlapping = vec![Op::Layer {
            ops: vec![
                dummy_draw(0.0, 0.0, 10.0, 10.0),
                dummy_draw(5.0, 0.0, 15.0, 10.0),
            ],
            bounds: [0.0, 0.0, 15.0, 10.0],
            filters: vec![],
            blend: VabBlendMode::Add,
        }];
        assert!(matches!(
            flatten_direct_fixed_blends(overlapping, true).as_slice(),
            [Op::Layer { .. }]
        ));

        // 3. Filtered Add stays isolated.
        let filtered = vec![Op::Layer {
            ops: vec![dummy_draw(0.0, 0.0, 10.0, 10.0)],
            bounds: [0.0, 0.0, 10.0, 10.0],
            filters: vec![AnimFilter::BlurFilter(AnimBlurFilter {
                blur_x: 5 * 65536,
                blur_y: 5 * 65536,
                num_passes: 1,
            })],
            blend: VabBlendMode::Add,
        }];
        assert!(matches!(
            flatten_direct_fixed_blends(filtered, true).as_slice(),
            [Op::Layer { .. }]
        ));

        // 4. Lighten flattens only at top-level Camera2d (not offscreen/nested).
        let lighten = vec![Op::Layer {
            ops: vec![dummy_draw(0.0, 0.0, 10.0, 10.0)],
            bounds: [0.0, 0.0, 10.0, 10.0],
            filters: vec![],
            blend: VabBlendMode::Lighten,
        }];
        assert!(matches!(
            flatten_direct_fixed_blends(lighten.clone(), true).as_slice(),
            [Op::Draw(d)] if d.blend == VabBlendMode::Lighten
        ));
        assert!(matches!(
            flatten_direct_fixed_blends(lighten, false).as_slice(),
            [Op::Layer { .. }]
        ));
    }
}
