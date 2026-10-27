//! Optional VAB compression and authenticated encryption.
//!
//! Compress before encryption. Keys belong to the host application, not this
//! library. A key shipped with a client can be recovered by that client.
use std::{
    borrow::Cow,
    io::{Read, Write},
};

use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use anyhow::{Context, Result, ensure};
use bevy::prelude::Resource;
use flate2::{Compression, bufread::ZlibDecoder, write::ZlibEncoder};

const SEALED_MAGIC: &[u8; 4] = b"VAE1";
const COMPRESSED_FLAG: u32 = 0x8000_0000;
const MAX_CHUNK: usize = 512 * 1024 * 1024;

/// Insert before adding `FlashPlayerPlugin`. The key is copied into the loader.
#[derive(Resource, Clone)]
pub struct VabProtectionKey([u8; 32]);

impl VabProtectionKey {
    pub fn new(key: [u8; 32]) -> Self {
        Self(key)
    }
}

fn header(data: &[u8]) -> Result<u32> {
    ensure!(
        data.len() >= 12 && &data[..4] == b"VATF",
        "Invalid VAB header"
    );
    let version = u32::from_le_bytes(data[4..8].try_into()?);
    ensure!(
        version == 1 || version == 2,
        "Unsupported VAB version {version}"
    );
    ensure!(
        u32::from_le_bytes(data[8..12].try_into()?) as usize == data.len(),
        "VAB length mismatch"
    );
    Ok(version)
}

fn chunks(data: &[u8], version: u32) -> Result<Vec<([u8; 4], Vec<u8>)>> {
    let mut offset = 12;
    let mut result: Vec<([u8; 4], Vec<u8>)> = Vec::new();
    let mut decoded_total = 0usize;
    while offset < data.len() {
        ensure!(data.len() - offset >= 8, "Truncated VAB chunk header");
        let name: [u8; 4] = data[offset..offset + 4].try_into()?;
        ensure!(
            !result.iter().any(|(n, _)| *n == name),
            "Duplicate VAB chunk"
        );
        let n = u32::from_le_bytes(data[offset + 4..offset + 8].try_into()?);
        let compressed = version == 2 && n & COMPRESSED_FLAG != 0;
        let len = if compressed { n & !COMPRESSED_FLAG } else { n } as usize;
        offset += 8;
        ensure!(
            len <= MAX_CHUNK && len <= data.len() - offset,
            "Invalid VAB chunk length"
        );
        let raw = &data[offset..offset + len];
        offset += len;
        let decoded = if compressed {
            ensure!(raw.len() >= 4, "Truncated compressed VAB chunk");
            let expected = u32::from_le_bytes(raw[..4].try_into()?) as usize;
            ensure!(expected <= MAX_CHUNK, "VAB chunk exceeds size limit");
            decoded_total = decoded_total
                .checked_add(expected)
                .context("VAB decoded size overflow")?;
            ensure!(
                decoded_total <= MAX_CHUNK,
                "VAB decoded data exceeds size limit"
            );
            let mut decoder = ZlibDecoder::new(&raw[4..]);
            let mut result = Vec::with_capacity(expected);
            (&mut decoder)
                .take(expected as u64 + 1)
                .read_to_end(&mut result)
                .context("Invalid compressed VAB chunk")?;
            ensure!(
                result.len() == expected && decoder.total_in() as usize == raw.len() - 4,
                "Compressed VAB chunk length mismatch"
            );
            result
        } else {
            decoded_total = decoded_total
                .checked_add(raw.len())
                .context("VAB decoded size overflow")?;
            ensure!(
                decoded_total <= MAX_CHUNK,
                "VAB decoded data exceeds size limit"
            );
            raw.to_vec()
        };
        result.push((name, decoded));
    }
    Ok(result)
}

/// Compress v1 chunks where beneficial. Existing v2 bytes are preserved.
pub fn compress(data: &[u8]) -> Result<Vec<u8>> {
    let version = header(data)?;
    let values = chunks(data, version)?;
    if version == 2 {
        return Ok(data.to_vec());
    }
    let mut out = b"VATF\x02\0\0\0\0\0\0\0".to_vec();
    let mut changed = false;
    for (name, raw) in values {
        out.extend_from_slice(&name);
        let mut packed = Vec::new();
        if &name != b"TEXT" && raw.len() >= 128 {
            let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
            encoder.write_all(&raw)?;
            packed = encoder.finish()?;
        }
        if !packed.is_empty() && packed.len() + 4 < raw.len() {
            out.extend_from_slice(
                &(u32::try_from(packed.len() + 4)? | COMPRESSED_FLAG).to_le_bytes(),
            );
            out.extend_from_slice(&u32::try_from(raw.len())?.to_le_bytes());
            out.extend_from_slice(&packed);
            changed = true;
        } else {
            out.extend_from_slice(&u32::try_from(raw.len())?.to_le_bytes());
            out.extend_from_slice(&raw);
        }
    }
    if !changed {
        return Ok(data.to_vec());
    }
    let len = u32::try_from(out.len())?;
    out[8..12].copy_from_slice(&len.to_le_bytes());
    Ok(out)
}

/// Compress and seal one VAB. Each file uses a fresh random 96-bit nonce.
pub fn protect(data: &[u8], key: &VabProtectionKey) -> Result<Vec<u8>> {
    let compressed = compress(data)?;
    let mut nonce = [0u8; 12];
    getrandom::fill(&mut nonce).map_err(|e| anyhow::anyhow!("Could not generate nonce: {e}"))?;
    let mut result = Vec::with_capacity(16 + compressed.len() + 16);
    result.extend_from_slice(SEALED_MAGIC);
    result.extend_from_slice(&nonce);
    let cipher = Aes256Gcm::new_from_slice(&key.0).expect("32-byte key");
    let encrypted = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: &compressed,
                aad: &result,
            },
        )
        .map_err(|_| anyhow::anyhow!("VAB encryption failed"))?;
    result.extend_from_slice(&encrypted);
    Ok(result)
}

/// Open a protected VAB, then expand v2 chunks into the v1 reader layout.
/// Plain v1 files pass through without a copy.
pub fn decode<'a>(data: &'a [u8], key: Option<&VabProtectionKey>) -> Result<Cow<'a, [u8]>> {
    let opened = if data.starts_with(SEALED_MAGIC) {
        ensure!(data.len() >= 32, "Truncated encrypted VAB");
        let key = key.context("Encrypted VAB requires a protection key")?;
        let cipher = Aes256Gcm::new_from_slice(&key.0).expect("32-byte key");
        Cow::Owned(
            cipher
                .decrypt(
                    Nonce::from_slice(&data[4..16]),
                    Payload {
                        msg: &data[16..],
                        aad: &data[..16],
                    },
                )
                .map_err(|_| anyhow::anyhow!("VAB authentication failed"))?,
        )
    } else {
        Cow::Borrowed(data)
    };
    let version = header(&opened)?;
    if version == 1 {
        return Ok(opened);
    }
    let values = chunks(&opened, version)?;
    let mut out = b"VATF\x01\0\0\0\0\0\0\0".to_vec();
    for (name, raw) in values {
        out.extend_from_slice(&name);
        out.extend_from_slice(&u32::try_from(raw.len())?.to_le_bytes());
        out.extend_from_slice(&raw);
    }
    let len = u32::try_from(out.len())?;
    out[8..12].copy_from_slice(&len.to_le_bytes());
    Ok(Cow::Owned(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<u8> {
        let mut file = b"VATF\x01\0\0\0\0\0\0\0".to_vec();
        for (name, data) in [(b"BAKD", vec![3u8; 8000]), (b"TEXT", vec![4u8; 128])] {
            file.extend_from_slice(name);
            file.extend_from_slice(&(data.len() as u32).to_le_bytes());
            file.extend_from_slice(&data);
        }
        let len = file.len() as u32;
        file[8..12].copy_from_slice(&len.to_le_bytes());
        file
    }

    #[test]
    fn round_trip_plain_compressed_and_encrypted() {
        let raw = fixture();
        let key = VabProtectionKey::new([7; 32]);
        assert!(matches!(decode(&raw, None).unwrap(), Cow::Borrowed(_)));
        let compressed = compress(&raw).unwrap();
        assert!(compressed.len() < raw.len());
        assert_eq!(&*decode(&compressed, None).unwrap(), raw);
        let protected = protect(&compressed, &key).unwrap();
        assert_eq!(&*decode(&protected, Some(&key)).unwrap(), raw);
        assert!(decode(&protected, None).is_err());
        assert!(decode(&protected, Some(&VabProtectionKey::new([9; 32]))).is_err());
        let mut damaged = protected.clone();
        *damaged.last_mut().unwrap() ^= 1;
        assert!(decode(&damaged, Some(&key)).is_err());
        damaged = protected.clone();
        damaged[4] ^= 1;
        assert!(decode(&damaged, Some(&key)).is_err());
        assert_ne!(protect(&raw, &key).unwrap(), protected);
    }

    #[test]
    fn rejects_truncated_and_malformed_data() {
        let key = VabProtectionKey::new([1; 32]);
        for bytes in [&b"VAE1"[..], &b"VATF"[..]] {
            assert!(decode(bytes, Some(&key)).is_err());
        }
        let mut raw = fixture();
        raw[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(protect(&raw, &key).is_err());
    }
}
