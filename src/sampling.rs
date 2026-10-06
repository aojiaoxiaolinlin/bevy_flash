//! Samples compiled frames. Ordinary sprite timeline evaluation belongs to vatf.
use std::collections::BTreeSet;

use anyhow::{Context, Result, bail, ensure};
use bevy::{math::Vec3, platform::collections::HashMap, prelude::Component};
use vatf::{
    animation::{AnimTransform, filter_dest_rect},
    baked::BakedNode,
};

use crate::vab_asset::{CommandList, VabAsset, VabBlendMode, compute_command_bounds};

#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct VabSkin {
    selections: HashMap<String, String>,
}

impl VabSkin {
    pub fn selection(&self, slot: &str) -> Option<&str> {
        self.selections.get(slot).map(String::as_str)
    }

    /// Validate every occurrence of this slot before changing the instance selection.
    pub fn set(&mut self, asset: &VabAsset, slot: &str, variant: &str) -> Result<()> {
        ensure!(!variant.is_empty(), "empty skin variant");
        let mut symbols = Vec::new();
        fn collect(nodes: &[BakedNode], slot: &str, symbols: &mut Vec<u16>) {
            for node in nodes {
                match node {
                    BakedNode::Skin {
                        slot: name, symbol, ..
                    } if name == slot => symbols.push(*symbol),
                    BakedNode::Group { children, .. } => collect(children, slot, symbols),
                    BakedNode::Mask { mask, children } => {
                        collect(mask, slot, symbols);
                        collect(children, slot, symbols);
                    }
                    _ => {}
                }
            }
        }
        for clip in &asset.baked.clips {
            for frame in &clip.frames {
                collect(frame, slot, &mut symbols);
            }
        }
        for skin in &asset.baked.skins {
            for variant in &skin.variants {
                collect(&variant.nodes, slot, &mut symbols);
            }
        }
        ensure!(!symbols.is_empty(), "unknown skin slot {slot}");
        for symbol in symbols {
            let skin = asset
                .baked
                .skins
                .iter()
                .find(|s| s.symbol == symbol)
                .context("missing skin symbol")?;
            ensure!(
                skin.variants
                    .iter()
                    .any(|candidate| candidate.name == variant),
                "skin {slot}: unknown variant {variant} for symbol {symbol}"
            );
        }
        self.selections.insert(slot.into(), variant.into());
        Ok(())
    }

    /// Atomic update: a failed slot leaves all selections unchanged.
    pub fn set_many<S, V>(
        &mut self,
        asset: &VabAsset,
        slots: impl IntoIterator<Item = (S, V)>,
    ) -> Result<()>
    where
        S: AsRef<str>,
        V: AsRef<str>,
    {
        let mut next = self.clone();
        for (slot, variant) in slots {
            next.set(asset, slot.as_ref(), variant.as_ref())?;
        }
        *self = next;
        Ok(())
    }
}

impl VabAsset {
    pub fn clip_index(&self, name: &str) -> Option<usize> {
        self.baked.clips.iter().position(|c| c.name == name)
    }

    pub fn skin_slots(&self) -> Vec<String> {
        fn collect(nodes: &[BakedNode], slots: &mut BTreeSet<String>) {
            for node in nodes {
                match node {
                    BakedNode::Skin { slot, .. } => {
                        slots.insert(slot.clone());
                    }
                    BakedNode::Group { children, .. } => collect(children, slots),
                    BakedNode::Mask { mask, children } => {
                        collect(mask, slots);
                        collect(children, slots);
                    }
                    BakedNode::Shape { .. } => {}
                }
            }
        }
        let mut slots = BTreeSet::new();
        for clip in &self.baked.clips {
            for frame in &clip.frames {
                collect(frame, &mut slots);
            }
        }
        for skin in &self.baked.skins {
            for variant in &skin.variants {
                collect(&variant.nodes, &mut slots);
            }
        }
        slots.into_iter().collect()
    }

    /// Names accepted by every symbol used through this slot, in source order.
    pub fn skin_variant_names(&self, slot: &str) -> Result<Vec<String>> {
        fn collect(nodes: &[BakedNode], slot: &str, symbols: &mut Vec<u16>) {
            for node in nodes {
                match node {
                    BakedNode::Skin {
                        slot: name, symbol, ..
                    } if name == slot && !symbols.contains(symbol) => symbols.push(*symbol),
                    BakedNode::Group { children, .. } => collect(children, slot, symbols),
                    BakedNode::Mask { mask, children } => {
                        collect(mask, slot, symbols);
                        collect(children, slot, symbols);
                    }
                    _ => {}
                }
            }
        }
        let mut symbols = Vec::new();
        for clip in &self.baked.clips {
            for frame in &clip.frames {
                collect(frame, slot, &mut symbols);
            }
        }
        for skin in &self.baked.skins {
            for variant in &skin.variants {
                collect(&variant.nodes, slot, &mut symbols);
            }
        }
        let first_symbol = *symbols
            .first()
            .with_context(|| format!("unknown skin slot {slot}"))?;
        let first = self
            .baked
            .skins
            .iter()
            .find(|skin| skin.symbol == first_symbol)
            .context("missing skin symbol")?;
        let names = first
            .variants
            .iter()
            .filter(|variant| {
                symbols.iter().all(|symbol| {
                    self.baked.skins.iter().any(|skin| {
                        skin.symbol == *symbol
                            && skin
                                .variants
                                .iter()
                                .any(|candidate| candidate.name == variant.name)
                    })
                })
            })
            .map(|variant| variant.name.clone())
            .collect::<Vec<_>>();
        ensure!(!names.is_empty(), "skin slot {slot} has no common variants");
        Ok(names)
    }

    pub fn sample(
        &self,
        clip: usize,
        frame: usize,
        skins: &VabSkin,
        scale: Vec3,
    ) -> Result<CommandList> {
        ensure!(
            scale.is_finite() && scale.x > 0.0 && scale.y > 0.0,
            "invalid output scale"
        );
        let nodes = self
            .baked
            .clips
            .get(clip)
            .and_then(|c| c.frames.get(frame))
            .context("clip/frame out of range")?;
        // Both geometry and filter extents use output pixels, so nested bounds agree.
        let mut root = AnimTransform::default();
        root.matrix.a = scale.x;
        root.matrix.d = scale.y;
        self.sample_nodes(nodes, skins, scale, root, 0)
    }

    fn sample_nodes(
        &self,
        nodes: &[BakedNode],
        skins: &VabSkin,
        scale: Vec3,
        parent: AnimTransform,
        depth: usize,
    ) -> Result<CommandList> {
        ensure!(depth < 128, "recursive/deep baked groups or skins");
        let mut commands = CommandList::default();
        for node in nodes {
            match node {
                BakedNode::Shape {
                    id,
                    ratio,
                    transform,
                } => {
                    let handles = self
                        .shape_map
                        .get(id)
                        .or_else(|| self.morph_map.get(&(*id, *ratio)))
                        .with_context(|| format!("unresolved shape {id} ratio {ratio}"))?;
                    let transform = AnimTransform {
                        matrix: parent.matrix * transform.matrix,
                        color_transform: parent.color_transform * transform.color_transform,
                    };
                    for &handle in handles {
                        commands.render_shape(handle, transform);
                    }
                }
                BakedNode::Skin {
                    slot,
                    symbol,
                    transform,
                } => {
                    let skin = self
                        .baked
                        .skins
                        .iter()
                        .find(|s| s.symbol == *symbol)
                        .context("missing skin symbol")?;
                    let variant = match skins.selection(slot) {
                        Some(name) => skin
                            .variants
                            .iter()
                            .find(|variant| variant.name == name)
                            .with_context(|| format!("skin {slot}: invalid variant {name}"))?,
                        None => skin.variants.first().context("skin has no variants")?,
                    };
                    let parent = AnimTransform {
                        matrix: parent.matrix * transform.matrix,
                        color_transform: parent.color_transform * transform.color_transform,
                    };
                    commands.commands.extend(
                        self.sample_nodes(&variant.nodes, skins, scale, parent, depth + 1)?
                            .commands,
                    );
                }
                BakedNode::Group {
                    children,
                    filters,
                    blend_mode,
                } => {
                    let mut inner = self.sample_nodes(children, skins, scale, parent, depth + 1)?;
                    let mut filters: Vec<_> =
                        filters.iter().filter(|f| !f.impotent()).cloned().collect();
                    for filter in &mut filters {
                        filter.scale(scale.x, scale.y);
                    }
                    if !filters.is_empty() && !inner.is_empty() {
                        let [x, y, w, h] = compute_command_bounds(self, &inner);
                        let (x, y, w, h) = filter_dest_rect(x, y, w, h, &filters);
                        ensure!(
                            [x, y, w, h].iter().all(|v| v.is_finite()),
                            "non-finite filter bounds"
                        );
                        let mut filtered = CommandList::default();
                        filtered.apply_filter(inner, filters, [x, y, w, h]);
                        inner = filtered;
                    }
                    ensure!(*blend_mode <= 14, "invalid blend mode");
                    let blend = VabBlendMode::from(*blend_mode);
                    if blend == VabBlendMode::Normal {
                        commands.commands.extend(inner.commands);
                    } else {
                        commands.blend(inner, blend);
                    }
                }
                BakedNode::Mask { mask, children } => {
                    let mask = self.sample_nodes(mask, skins, scale, parent, depth + 1)?;
                    commands.push_mask();
                    commands.commands.extend(mask.commands.iter().cloned());
                    commands.activate_mask();
                    commands.commands.extend(
                        self.sample_nodes(children, skins, scale, parent, depth + 1)?
                            .commands,
                    );
                    commands.deactivate_mask();
                    commands.commands.extend(mask.commands);
                    commands.pop_mask();
                }
            }
        }
        Ok(commands)
    }

    pub(crate) fn validate_baked_references(&self) -> Result<()> {
        fn validate(
            asset: &VabAsset,
            nodes: &[BakedNode],
            visiting: &mut Vec<u16>,
            depth: usize,
        ) -> Result<()> {
            ensure!(depth < 128, "baked tree too deep");
            for node in nodes {
                match node {
                    BakedNode::Shape {
                        id,
                        ratio,
                        transform,
                    } => {
                        ensure!(
                            asset.shape_map.contains_key(id)
                                || asset.morph_map.contains_key(&(*id, *ratio)),
                            "unresolved shape {id} ratio {ratio}"
                        );
                        let m = transform.matrix;
                        let c = transform.color_transform;
                        ensure!(
                            [
                                m.a,
                                m.b,
                                m.c,
                                m.d,
                                m.tx,
                                m.ty,
                                c.r_multiply,
                                c.g_multiply,
                                c.b_multiply,
                                c.a_multiply,
                                c.r_add,
                                c.g_add,
                                c.b_add,
                                c.a_add
                            ]
                            .iter()
                            .all(|v| v.is_finite()),
                            "non-finite transform"
                        );
                    }
                    BakedNode::Skin { symbol, slot, .. } => {
                        ensure!(
                            !slot.is_empty() && !visiting.contains(symbol),
                            "recursive/invalid skin {symbol}"
                        );
                        let skin = asset
                            .baked
                            .skins
                            .iter()
                            .find(|s| s.symbol == *symbol)
                            .context("missing skin")?;
                        visiting.push(*symbol);
                        for variant in &skin.variants {
                            validate(asset, &variant.nodes, visiting, depth + 1)?;
                        }
                        visiting.pop();
                    }
                    BakedNode::Group {
                        children,
                        blend_mode,
                        ..
                    } => {
                        if *blend_mode > 14 {
                            bail!("unknown blend mode");
                        }
                        validate(asset, children, visiting, depth + 1)?;
                    }
                    BakedNode::Mask { mask, children } => {
                        validate(asset, mask, visiting, depth + 1)?;
                        validate(asset, children, visiting, depth + 1)?;
                    }
                }
            }
            Ok(())
        }
        for clip in &self.baked.clips {
            for frame in &clip.frames {
                validate(self, frame, &mut Vec::new(), 0)?;
            }
        }
        Ok(())
    }
}
