use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::time::Instant;

use rkyv::api::high::{HighDeserializer, HighSerializer, HighValidator, from_bytes, to_bytes};
use rkyv::bytecheck::CheckBytes;
use rkyv::rancor::Error as RkyvError;
use rkyv::ser::allocator::ArenaHandle;
use rkyv::util::AlignedVec;
use rkyv::{Archive, Deserialize, Serialize};
use zstd::stream::{decode_all, encode_all};

use crate::error::{DbtNovaError, Result};
use crate::utils::unique_suffix;
use tracing::info;

type RkyvSerializer<'a> = HighSerializer<AlignedVec, ArenaHandle<'a>, RkyvError>;
type RkyvDeserializer = HighDeserializer<RkyvError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheLoadFailure {
    Missing {
        path: PathBuf,
    },
    TooLarge {
        path: PathBuf,
        actual_bytes: u64,
        max_bytes: u64,
    },
    ReadFailed {
        path: PathBuf,
        error: String,
    },
    DecompressFailed {
        path: PathBuf,
        error: String,
    },
    DecodeFailed {
        path: PathBuf,
        error: String,
    },
}

impl CacheLoadFailure {
    #[must_use]
    pub fn path(&self) -> &Path {
        match self {
            Self::Missing { path }
            | Self::TooLarge { path, .. }
            | Self::ReadFailed { path, .. }
            | Self::DecompressFailed { path, .. }
            | Self::DecodeFailed { path, .. } => path,
        }
    }

    #[must_use]
    pub fn summary(&self) -> String {
        match self {
            Self::Missing { path } => format!("cache file is missing: {}", path.display()),
            Self::TooLarge {
                path,
                actual_bytes,
                max_bytes,
            } => format!(
                "cache file exceeds size limit ({} > {}) at {}",
                actual_bytes,
                max_bytes,
                path.display()
            ),
            Self::ReadFailed { path, error } => {
                format!("failed to read cache {}: {error}", path.display())
            }
            Self::DecompressFailed { path, error } => {
                format!("failed to decompress cache {}: {error}", path.display())
            }
            Self::DecodeFailed { path, error } => {
                format!("failed to decode cache {}: {error}", path.display())
            }
        }
    }
}

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
    write_bytes_atomic(path, bytes.as_slice())
}

/// Persist a value to disk in compressed rkyv format.
///
/// # Errors
/// Returns an error if serialization, compression, or writing fails.
pub fn save_rkyv_zst<T>(value: &T, path: &Path) -> Result<()>
where
    T: for<'a> Serialize<RkyvSerializer<'a>>,
{
    let save_started = Instant::now();
    info!(cache_path = %path.display(), "starting compressed rkyv cache save");

    let serialize_started = Instant::now();
    let bytes =
        to_bytes::<RkyvError>(value).map_err(|e| DbtNovaError::ServerError(e.to_string()))?;
    let serialize_ms = serialize_started.elapsed().as_millis();
    let serialized_bytes = bytes.len();

    let compress_started = Instant::now();
    let compressed = compress_bytes(bytes.as_slice())?;
    let compress_ms = compress_started.elapsed().as_millis();
    let compressed_bytes = compressed.len();

    let write_started = Instant::now();
    write_bytes_atomic(path, compressed.as_slice())?;
    let write_ms = write_started.elapsed().as_millis();

    info!(
        cache_path = %path.display(),
        serialized_bytes,
        compressed_bytes,
        serialize_ms,
        compress_ms,
        write_ms,
        total_ms = save_started.elapsed().as_millis(),
        "finished compressed rkyv cache save"
    );

    Ok(())
}

/// Load an rkyv cache from disk.
///
/// # Errors
/// Returns a typed cache failure when the file is missing, unreadable, or invalid.
pub fn load_rkyv_file<T>(path: &Path) -> std::result::Result<T, CacheLoadFailure>
where
    T: Archive,
    T::Archived:
        for<'a> CheckBytes<HighValidator<'a, RkyvError>> + Deserialize<T, RkyvDeserializer>,
{
    let bytes = fs::read(path).map_err(|error| map_read_failure(path, &error))?;
    decode_rkyv(path, &bytes)
}

/// Load a compressed rkyv cache from disk.
///
/// # Errors
/// Returns a typed cache failure when the file is missing, unreadable, too large, or invalid.
pub fn load_rkyv_file_zst<T>(
    path: &Path,
    max_decompressed_bytes: u64,
) -> std::result::Result<T, CacheLoadFailure>
where
    T: Archive,
    T::Archived:
        for<'a> CheckBytes<HighValidator<'a, RkyvError>> + Deserialize<T, RkyvDeserializer>,
{
    let bytes = fs::read(path).map_err(|error| map_read_failure(path, &error))?;
    let decoded = decompress_bytes(bytes, max_decompressed_bytes).map_err(|error| {
        if error
            .to_string()
            .starts_with("Embeddings cache exceeded max decompressed bytes")
        {
            let actual_bytes = match fs::metadata(path) {
                Ok(metadata) => metadata.len(),
                Err(_) => max_decompressed_bytes.saturating_add(1),
            };
            CacheLoadFailure::TooLarge {
                path: path.to_path_buf(),
                actual_bytes,
                max_bytes: max_decompressed_bytes,
            }
        } else {
            CacheLoadFailure::DecompressFailed {
                path: path.to_path_buf(),
                error: error.to_string(),
            }
        }
    })?;
    decode_rkyv(path, &decoded)
}

/// Decode rkyv bytes that have already been read into memory.
///
/// # Errors
/// Returns a typed cache failure when the payload cannot be deserialized safely.
pub fn decode_rkyv<T>(path: &Path, bytes: &[u8]) -> std::result::Result<T, CacheLoadFailure>
where
    T: Archive,
    T::Archived:
        for<'a> CheckBytes<HighValidator<'a, RkyvError>> + Deserialize<T, RkyvDeserializer>,
{
    from_bytes::<T, RkyvError>(bytes).map_err(|error| CacheLoadFailure::DecodeFailed {
        path: path.to_path_buf(),
        error: error.to_string(),
    })
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

fn map_read_failure(path: &Path, error: &std::io::Error) -> CacheLoadFailure {
    if error.kind() == std::io::ErrorKind::NotFound {
        CacheLoadFailure::Missing {
            path: path.to_path_buf(),
        }
    } else {
        CacheLoadFailure::ReadFailed {
            path: path.to_path_buf(),
            error: error.to_string(),
        }
    }
}

fn write_bytes_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("cache.rkyv");
    let tmp = path.with_file_name(format!("{file_name}.{}.tmp", unique_suffix()));
    fs::write(&tmp, bytes)?;
    fs::rename(tmp, path)?;
    Ok(())
}
