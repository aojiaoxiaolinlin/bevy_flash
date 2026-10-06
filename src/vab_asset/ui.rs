//! Build named UI sub-assets from compiler output, sharing the main asset's meshes.
use super::{VabAsset, compute_command_bounds};
use anyhow::{Context, Result};
use bevy::{asset::LoadContext, math::Vec3, platform::collections::HashMap};

pub(super) fn load_ui_assets(
    asset: &VabAsset,
    graphics: Vec<vatf::graphics::Graphic>,
    buttons: Vec<vatf::graphics::Button>,
    load_context: &mut LoadContext<'_>,
) -> Result<()> {
    let mut ui_graphics = HashMap::new();
    for graphic in graphics {
        let (name, graphic) = build_graphic(asset, graphic, load_context)?;
        ui_graphics.insert(name, graphic);
    }
    align_button_bounds(&mut ui_graphics, &buttons)?;
    let graphic_handles: HashMap<_, _> = ui_graphics
        .into_iter()
        .map(|(name, graphic)| {
            let handle = load_context.add_labeled_asset(name.clone(), graphic);
            (name, handle)
        })
        .collect();
    for button in buttons {
        let state = |name: &str| {
            graphic_handles
                .get(name)
                .cloned()
                .with_context(|| format!("missing button state handle {name}"))
        };
        load_context.add_labeled_asset(
            button.name,
            crate::vab_button::VabButton {
                up: state(&button.up)?,
                over: state(&button.over)?,
                down: state(&button.down)?,
                hit_test: button.hit_test.as_deref().map(state).transpose()?,
            },
        );
    }
    Ok(())
}

fn build_graphic(
    asset: &VabAsset,
    graphic: vatf::graphics::Graphic,
    load_context: &mut LoadContext<'_>,
) -> Result<(String, crate::vab_graphic::VabGraphic)> {
    for nodes in &graphic.frames {
        crate::vab_graphic::validate_nodes(nodes, asset)
            .with_context(|| format!("UI export {:?}", graphic.name))?;
    }
    let render_asset = VabAsset {
        baked: vatf::baked::BakedMovie {
            frame_rate: graphic.frame_rate,
            skins: vec![],
            clips: vec![vatf::baked::BakedClip {
                name: "default".into(),
                start_frame: 0,
                events: vec![],
                frames: graphic.frames,
            }],
        },
        shape_map: asset.shape_map.clone(),
        morph_map: HashMap::default(),
        render_meshes: asset.render_meshes.clone(),
    };
    render_asset.validate_baked_references()?;
    let mut visual = [
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    ];
    let frame_count = render_asset.baked.clips[0].frames.len();
    for frame in 0..frame_count {
        let sample =
            render_asset.sample(0, frame, &crate::sampling::VabSkin::default(), Vec3::ONE)?;
        let b = compute_command_bounds(&render_asset, &sample);
        if b[2] > 0.0 && b[3] > 0.0 {
            visual[0] = visual[0].min(b[0]);
            visual[1] = visual[1].min(b[1]);
            visual[2] = visual[2].max(b[0] + b[2]);
            visual[3] = visual[3].max(b[1] + b[3]);
        }
    }
    anyhow::ensure!(
        visual.iter().all(|v| v.is_finite()),
        "empty UI visual bounds"
    );
    let render_asset =
        load_context.add_labeled_asset(format!("__vab/graphic/{}", graphic.name), render_asset);
    Ok((
        graphic.name,
        crate::vab_graphic::VabGraphic {
            source_bounds: graphic.source_bounds,
            visual_bounds: [
                visual[0],
                visual[1],
                visual[2] - visual[0],
                visual[3] - visual[1],
            ],
            frame_count,
            frame_rate: graphic.frame_rate,
            render_asset,
        },
    ))
}

fn align_button_bounds(
    ui_graphics: &mut HashMap<String, crate::vab_graphic::VabGraphic>,
    buttons: &[vatf::graphics::Button],
) -> Result<()> {
    for button in buttons {
        let states = [&button.up, &button.over, &button.down];
        let mut union = [
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        ];
        let registration = ui_graphics
            .get(&button.up)
            .context("missing button up graphic")?
            .source_bounds;
        for name in states {
            let graphic = ui_graphics
                .get(name)
                .with_context(|| format!("button {} missing state {name}", button.name))?;
            anyhow::ensure!(
                graphic.source_bounds == registration,
                "button {} states have inconsistent registration",
                button.name
            );
            let b = graphic.visual_bounds;
            union[0] = union[0].min(b[0]);
            union[1] = union[1].min(b[1]);
            union[2] = union[2].max(b[0] + b[2]);
            union[3] = union[3].max(b[1] + b[3]);
        }
        for name in states {
            ui_graphics
                .get_mut(name)
                .context("missing button state graphic")?
                .visual_bounds = [union[0], union[1], union[2] - union[0], union[3] - union[1]];
        }
    }
    Ok(())
}
