use std::fs;
use std::io::{Cursor, Read};
use std::path::Path;

use rkyv::api::high::{HighDeserializer, HighSerializer, HighValidator, from_bytes, to_bytes};
use rkyv::bytecheck::CheckBytes;
use rkyv::rancor::Error as RkyvError;
use rkyv::ser::allocator::ArenaHandle;
use rkyv::util::AlignedVec;
use rkyv::{Archive, Deserialize, Serialize};
use zstd::stream::{decode_all, encode_all};

use crate::error::{DbtNovaError, Result};

type RkyvSerializer<'a> = HighSerializer<AlignedVec, ArenaHandle<'a>, RkyvError>;
type RkyvDeserializer = HighDeserializer<RkyvError>;

/// Persist a value to disk in rkyv format.
///
/// # Errors
/// Returns an error if serialization or writing fails.
pub fn save_rkyv<T>(value: &T, path: &Path) -> Result<()>
where
    T: for<'a> Serialize<RkyvSerializer<'a>>,
{
    let bytes =
        to_bytes::<RkyvError>(value).map_err(|e| DbtNovaError::ServerError(e.to_string()))?;
    fs::write(path, bytes)?;
    Ok(())
}

/// Persist a value to disk in compressed rkyv format.
///
/// # Errors
/// Returns an error if serialization, compression, or writing fails.
pub fn save_rkyv_zst<T>(value: &T, path: &Path) -> Result<()>
where
    T: for<'a> Serialize<RkyvSerializer<'a>>,
{
    let bytes =
        to_bytes::<RkyvError>(value).map_err(|e| DbtNovaError::ServerError(e.to_string()))?;
    let compressed = compress_bytes(bytes)?;
    fs::write(path, compressed)?;
    Ok(())
}

pub fn load_rkyv_file<T, F>(path: &Path, validate: F) -> Option<T>
where
    T: Archive,
    T::Archived:
        for<'a> CheckBytes<HighValidator<'a, RkyvError>> + Deserialize<T, RkyvDeserializer>,
    F: FnOnce(&T) -> bool,
{
    let bytes = fs::read(path).ok()?;
    decode_rkyv(&bytes, validate)
}

pub fn load_rkyv_file_zst<T, F>(path: &Path, max_decompressed_bytes: u64, validate: F) -> Option<T>
where
    T: Archive,
    T::Archived:
        for<'a> CheckBytes<HighValidator<'a, RkyvError>> + Deserialize<T, RkyvDeserializer>,
    F: FnOnce(&T) -> bool,
{
    let bytes = fs::read(path).ok()?;
    let decoded = decompress_bytes(bytes, max_decompressed_bytes).ok()?;
    decode_rkyv(&decoded, validate)
}

pub fn decode_rkyv<T, F>(bytes: &[u8], validate: F) -> Option<T>
where
    T: Archive,
    T::Archived:
        for<'a> CheckBytes<HighValidator<'a, RkyvError>> + Deserialize<T, RkyvDeserializer>,
    F: FnOnce(&T) -> bool,
{
    let archived = from_bytes::<T, RkyvError>(bytes).ok()?;
    if validate(&archived) {
        Some(archived)
    } else {
        None
    }
}

pub(crate) fn compress_bytes(bytes: impl AsRef<[u8]>) -> Result<Vec<u8>> {
    encode_all(Cursor::new(bytes.as_ref()), 0).map_err(|e| DbtNovaError::ServerError(e.to_string()))
}

pub(crate) fn decompress_bytes(bytes: impl AsRef<[u8]>, max_bytes: u64) -> Result<Vec<u8>> {
    if max_bytes == 0 {
        return decode_all(Cursor::new(bytes.as_ref()))
            .map_err(|e| DbtNovaError::ServerError(e.to_string()));
    }

    let mut decoder = zstd::stream::read::Decoder::new(Cursor::new(bytes.as_ref()))
        .map_err(|e| DbtNovaError::ServerError(e.to_string()))?;
    let mut out = Vec::new();
    let mut buf = [0u8; 8192];
    let mut total = 0u64;
    loop {
        let read = decoder
            .read(&mut buf)
            .map_err(|e| DbtNovaError::ServerError(e.to_string()))?;
        if read == 0 {
            break;
        }
        total = total.saturating_add(read as u64);
        if total > max_bytes {
            return Err(DbtNovaError::ServerError(format!(
                "Embeddings cache exceeded max decompressed bytes ({total} > {max_bytes})"
            )));
        }
        out.extend_from_slice(&buf[..read]);
    }
    Ok(out)
}
