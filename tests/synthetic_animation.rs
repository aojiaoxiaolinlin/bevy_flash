//! Structural verification of the command-list builder.
//!
//! These tests never touch a renderer or the GPU: they build a `VabAsset` around
//! a hand-written baked clip and assert exactly what `build_frame_commands`
//! emits — filter isolation, blend wrapping, and how bounds cover nested
//! sub-lists.
//!
//! Transform accumulation and sub-timeline phasing are deliberately **not**
//! tested here. Both happen at bake time now (see `vatf`'s `baked.rs`), so the
//! renderer has no transform stack and no per-instance timeline to drive.

use bevy::asset::Handle;
use bevy::math::Vec3;
use bevy_flash_remake::vab_asset::{
    CommandList, MeshMaterial, RenderMeshGroup, VabAsset, VabBlendMode, VabCommand,
};
use vatf::animation::{AnimBlurFilter, AnimFilter, AnimTransform};
use vatf::baked::{BakedClip, BakedMovie, BakedNode};

// ===========================================================================
// Builders
// ===========================================================================

/// A 4px, single-pass blur (radius in pixels is `Fixed16`, i.e. pixels * 65536).
fn blur_filter() -> AnimFilter {
    AnimFilter::BlurFilter(AnimBlurFilter {
        blur_x: (4.0 * 65536.0) as i32,
        blur_y: (4.0 * 65536.0) as i32,
        num_passes: 1,
    })
}

/// A leaf shape node. Baked transforms are world-space, so identity means
/// "wherever the mesh's own local bounds put it".
fn shape(id: u16) -> BakedNode {
    BakedNode::Shape {
        id,
        ratio: 0,
        transform: AnimTransform::default(),
    }
}

/// A `Group` node. `blend_mode` uses the raw `swf::BlendMode` discriminants,
/// where anything that is not `Normal` is a real blend mode; `1` reads as
/// `Normal` and is what the baker emits for "no blend".
fn group(children: Vec<BakedNode>, filters: Vec<AnimFilter>, blend_mode: u8) -> BakedNode {
    BakedNode::Group {
        children,
        filters,
        blend_mode,
    }
}

fn render_mesh(local_bounds: [f32; 4]) -> RenderMeshGroup {
    RenderMeshGroup {
        mesh: Handle::default(),
        material: MeshMaterial::Color,
        local_bounds,
    }
}

/// A `VabAsset` whose single clip is one frame containing `nodes`.
fn asset_with_frame(render_meshes: Vec<RenderMeshGroup>, nodes: Vec<BakedNode>) -> VabAsset {
    VabAsset {
        baked: BakedMovie {
            frame_rate: 30.0,
            skins: Vec::new(),
            clips: vec![BakedClip {
                name: "default".into(),
                start_frame: 0,
                events: Vec::new(),
                frames: vec![nodes],
            }],
        },
        shape_map: Default::default(),
        morph_map: Default::default(),
        render_meshes,
    }
}

fn render_shapes(commands: &CommandList) -> Vec<usize> {
    commands
        .commands
        .iter()
        .filter_map(|command| match command {
            VabCommand::RenderShape { handle, .. } => Some(*handle),
            _ => None,
        })
        .collect()
}

const TOLERANCE: f32 = 1e-4;

fn assert_close(actual: f32, expected: f32, context: &str) {
    assert!(
        (actual - expected).abs() <= TOLERANCE,
        "{context}: expected {expected}, got {actual}",
    );
}

// ===========================================================================
// Filter / blend isolation
// ===========================================================================

#[test]
fn outer_filter_includes_inner_filter_output() {
    let mut asset = asset_with_frame(
        vec![render_mesh([0.0, 0.0, 10.0, 10.0])],
        vec![group(
            vec![group(vec![shape(1)], vec![blur_filter()], 1)],
            vec![blur_filter()],
            1,
        )],
    );
    asset.shape_map.insert(1, vec![0]);

    let commands = asset.build_frame_commands(Vec3::ONE, 0);
    match &commands.commands[0] {
        VabCommand::ApplyFilter { bounds, .. } => assert_eq!(*bounds, [-8.0, -8.0, 26.0, 26.0]),
        other => panic!("expected nested filter, got {other:?}"),
    }
}

#[test]
fn filter_isolation_emits_apply_filter_and_scales_with_view() {
    let mut asset = asset_with_frame(
        vec![render_mesh([0.0, 0.0, 10.0, 10.0])],
        vec![group(vec![shape(1)], vec![blur_filter()], 1)],
    );
    asset.shape_map.insert(1, vec![0]);

    let commands = asset.build_frame_commands(Vec3::splat(1.0), 0);
    assert_eq!(commands.commands.len(), 1);

    match &commands.commands[0] {
        VabCommand::ApplyFilter {
            commands,
            filters,
            bounds,
        } => {
            assert_eq!(filters.len(), 1, "filter must survive isolation");
            assert_eq!(
                render_shapes(commands),
                vec![0],
                "sub-commands must not be dropped"
            );
            // 10x10 bounds expanded by a 4px blur, rounded out to whole pixels.
            assert_close(bounds[0], -4.0, "offset x");
            assert_close(bounds[1], -4.0, "offset y");
            assert_close(bounds[2], 18.0, "width");
            assert_close(bounds[3], 18.0, "height");
        }
        other => panic!("expected ApplyFilter, got {other:?}"),
    }

    // At 2x view scale `sample` bakes the factor into the root transform, so the
    // geometry itself becomes 20x20 and the blur radius goes 4px -> 7px
    // (`(blur - ONE) * factor + ONE`). The offscreen texture must cover both:
    // 20 + 2*7 = 34.
    let scaled = asset.build_frame_commands(Vec3::splat(2.0), 0);
    match &scaled.commands[0] {
        VabCommand::ApplyFilter { bounds, .. } => {
            assert_close(bounds[0], -7.0, "scaled offset x");
            assert_close(bounds[2], 34.0, "scaled width");
            assert_close(bounds[3], 34.0, "scaled height");
        }
        other => panic!("expected ApplyFilter, got {other:?}"),
    }
}

#[test]
fn blend_is_preserved_when_filters_are_present() {
    let mut asset = asset_with_frame(
        vec![render_mesh([0.0, 0.0, 10.0, 10.0])],
        // Multiply (3) with a filter -> the group must be both filtered and blended.
        vec![group(vec![shape(1)], vec![blur_filter()], 3)],
    );
    asset.shape_map.insert(1, vec![0]);

    let commands = asset.build_frame_commands(Vec3::splat(1.0), 0);
    assert_eq!(commands.commands.len(), 1);

    match &commands.commands[0] {
        VabCommand::Blend(inner, mode) => {
            assert_eq!(*mode, VabBlendMode::Multiply);
            assert!(
                matches!(inner.commands.first(), Some(VabCommand::ApplyFilter { .. })),
                "filter must still be applied inside the blend",
            );
        }
        other => panic!("expected Blend wrapping ApplyFilter, got {other:?}"),
    }
}

#[test]
fn blend_without_filters_emits_blend_command() {
    let mut asset = asset_with_frame(
        vec![render_mesh([0.0, 0.0, 10.0, 10.0])],
        // Add (8), no filters.
        vec![group(vec![shape(1)], vec![], 8)],
    );
    asset.shape_map.insert(1, vec![0]);

    let commands = asset.build_frame_commands(Vec3::splat(1.0), 0);
    match &commands.commands[0] {
        VabCommand::Blend(inner, mode) => {
            assert_eq!(*mode, VabBlendMode::Add);
            assert_eq!(render_shapes(inner), vec![0]);
        }
        other => panic!("expected Blend, got {other:?}"),
    }
}

#[test]
fn filter_bounds_cover_nested_blend_commands() {
    let mut asset = asset_with_frame(
        vec![
            render_mesh([0.0, 0.0, 10.0, 10.0]),
            render_mesh([100.0, 0.0, 110.0, 10.0]),
        ],
        // A filtered group wrapping a blended group drawing two shapes far apart.
        vec![group(
            vec![group(vec![shape(1), shape(2)], vec![], 8)],
            vec![blur_filter()],
            1,
        )],
    );
    asset.shape_map.insert(1, vec![0]);
    asset.shape_map.insert(2, vec![1]);

    let commands = asset.build_frame_commands(Vec3::splat(1.0), 0);
    match &commands.commands[0] {
        VabCommand::ApplyFilter { bounds, .. } => {
            // x spans 0..110, expanded by the 4px blur on both sides.
            assert_close(bounds[0], -4.0, "offset x");
            assert_close(bounds[2], 118.0, "width covers the nested blend");
            assert_close(bounds[3], 18.0, "height");
        }
        other => panic!("expected ApplyFilter, got {other:?}"),
    }
}
