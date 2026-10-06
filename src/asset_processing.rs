//! Optional SWF preprocessing. Runtime output is loaded by the ordinary VAB loader.
use bevy::{
    asset::{
        AssetApp, AsyncWriteExt,
        io::{AssetReaderError, Writer},
        processor::{Process, ProcessContext, ProcessError},
    },
    prelude::*,
    tasks::AsyncComputeTaskPool,
};
use std::path::{Path, PathBuf};

use crate::vab_asset::VabLoader;
pub use vatf::{COMPILER_REVISION, SwfCompileMode, SwfCompileSettings};

/// A revisioned cache root: compiler updates invalidate cached results without
/// changing the VAB layout version or editing user-authored source metadata.
/// Use the same path when generating and shipping processed assets.
pub fn vab_processed_asset_path(root: impl AsRef<Path>) -> PathBuf {
    root.as_ref()
        .join(format!("vab-compiler-{COMPILER_REVISION}"))
}

/// Add after AssetPlugin and FlashPlayerPlugin. Compilation only runs when
/// AssetPlugin uses Processed mode and its asset processor is enabled.
pub struct VabAssetProcessorPlugin;

impl Plugin for VabAssetProcessorPlugin {
    fn build(&self, app: &mut App) {
        app.register_asset_processor(SwfToVabProcessor)
            .set_default_asset_processor::<SwfToVabProcessor>("swf");
    }
}

/// Bevy stores VAB bytes under the original `.swf` path and selects VabLoader
/// through the generated output metadata, including named UI sub-assets.
#[derive(TypePath)]
pub struct SwfToVabProcessor;

impl Process for SwfToVabProcessor {
    type Settings = SwfCompileSettings;
    type OutputLoader = VabLoader;

    async fn process(
        &self,
        context: &mut ProcessContext<'_>,
        settings: &Self::Settings,
        writer: &mut Writer,
    ) -> Result<(), ProcessError> {
        let path = context.path().clone();
        let mut source = Vec::new();
        context
            .asset_reader()
            .read_to_end(&mut source)
            .await
            .map_err(|error| ProcessError::AssetReaderError {
                path: path.clone(),
                err: AssetReaderError::Io(error.into()),
            })?;
        let settings = settings.clone();
        // CPU compilation must not occupy Bevy's I/O pool.
        let compiled = AsyncComputeTaskPool::get()
            .spawn(async move { vatf::compile_swf(&source, &settings) })
            .await
            .map_err(|error| ProcessError::AssetTransformError(error.into()))?;
        writer
            .write_all(&compiled.bytes)
            .await
            .map_err(|error| ProcessError::AssetSaveError(error.into()))?;
        debug!(asset = %path, meshes_before = compiled.pruning.before.meshes, meshes_after = compiled.pruning.after.meshes, "SWF compiled to VAB");
        Ok(())
    }
}
