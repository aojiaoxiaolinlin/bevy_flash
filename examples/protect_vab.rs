//! Create a compressed, encrypted `.vab` using a 32-byte key file.
//! Usage: cargo run --features protected_assets --example protect_vab -- input.vab output.vab key.bin
use anyhow::{Context, Result, ensure};
use bevy_flash::protected_vab::{VabProtectionKey, decode, protect};
use std::{fs::OpenOptions, io::Write, path::PathBuf};

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let input = PathBuf::from(args.next().context("Missing input path")?);
    let output = PathBuf::from(args.next().context("Missing output path")?);
    let key_path = PathBuf::from(args.next().context("Missing key file")?);
    ensure!(args.next().is_none(), "Expected input, output and key file");
    let key: [u8; 32] = std::fs::read(key_path)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("Key must contain exactly 32 bytes"))?;
    let key = VabProtectionKey::new(key);
    let original = std::fs::read(input)?;
    let encoded = protect(&original, &key)?;
    ensure!(
        decode(&encoded, Some(&key))? == decode(&original, None)?,
        "VAB verification failed"
    );
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?;
    file.write_all(&encoded)?;
    file.sync_all()?;
    Ok(())
}
