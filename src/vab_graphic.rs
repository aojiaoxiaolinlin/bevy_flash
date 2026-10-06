//! Named baked vector graphics loaded with `ui.vab#export_name`.
use crate::vab_asset::VabAsset;

pub(crate) fn validate_nodes(
    nodes: &[vatf::baked::BakedNode],
    asset: &VabAsset,
) -> anyhow::Result<()> {
    use vatf::baked::BakedNode;
    for node in nodes {
        match node {
            BakedNode::Shape { id, ratio, .. } => {
                anyhow::ensure!(*ratio == 0, "UI graphic contains a morph ratio");
                let handles = asset
                    .shape_map
                    .get(id)
                    .ok_or_else(|| anyhow::anyhow!("UI graphic references missing Shape {id}"))?;
                for handle in handles {
                    anyhow::ensure!(
                        !matches!(
                            asset.render_meshes[*handle].material,
                            crate::vab_asset::MeshMaterial::Bitmap(_)
                        ),
                        "UI Shape {id} contains bitmap fill"
                    );
                }
            }
            BakedNode::Group { children, .. } => validate_nodes(children, asset)?,
            BakedNode::Mask { mask, children } => {
                validate_nodes(mask, asset)?;
                validate_nodes(children, asset)?;
            }
            BakedNode::Skin { .. } => anyhow::bail!("UI graphics cannot contain skin state"),
        }
    }
    Ok(())
}
use bevy::{asset::AssetPath, prelude::*};

/// A centered, immutable static or animated graphic. Meshes and materials are shared sub-assets.
#[derive(Asset, TypePath)]
pub struct VabGraphic {
    /// Original geometric bounds in Flash pixels, before centering.
    pub source_bounds: [f32; 4],
    /// Fixed union of all frame outputs including filter padding: x, y, width, height.
    pub visual_bounds: [f32; 4],
    pub frame_count: usize,
    pub frame_rate: f32,
    /// Baked renderer payload shared by static and animated UI.
    #[dependency]
    pub render_asset: Handle<VabAsset>,
}

impl VabGraphic {
    pub fn visual_size(&self) -> Vec2 {
        Vec2::new(self.visual_bounds[2], self.visual_bounds[3])
    }
    pub fn is_animated(&self) -> bool {
        self.frame_count > 1
    }

    pub fn size(&self) -> Vec2 {
        Vec2::new(
            self.source_bounds[2] - self.source_bounds[0],
            self.source_bounds[3] - self.source_bounds[1],
        )
    }
}

/// Public labels use the SWF ExportAssets name directly, without a prefix.
#[derive(Debug, Clone, Copy)]
pub enum VabAssetLabel<'a> {
    Graphic(&'a str),
    Button(&'a str),
}

impl VabAssetLabel<'_> {
    pub fn from_asset(&self, path: impl Into<AssetPath<'static>>) -> AssetPath<'static> {
        let (Self::Graphic(name) | Self::Button(name)) = self;
        path.into().with_label((*name).to_owned())
    }
}
