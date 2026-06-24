mod dbfs;
mod file;
mod gcs;
mod http;
mod s3;

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use blake3;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::sync::atomic::{AtomicU64, Ordering};
use tracing::warn;

use crate::config::DbtNovaConfig;
use crate::error::{DbtNovaError, Result};
use crate::utils::unique_suffix;

use dbfs::resolve_dbfs;
use file::resolve_file;
use gcs::resolve_gcs;
use http::resolve_http;
use s3::resolve_s3;

#[derive(Debug, Clone)]
struct ManifestLocator {
    raw: String,
    scheme: String,
    rest: String,
}

impl ManifestLocator {
    fn parse(raw: &str) -> Result<Self> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(DbtNovaError::InvalidParams(
                "manifest path/uri cannot be empty".to_string(),
            ));
        }
        let lower = trimmed.to_lowercase();
        let is_legacy_dbfs = lower.starts_with("dbfs:/") && !lower.starts_with("dbfs://");
        let normalized = if is_legacy_dbfs {
            let rest = &trimmed[6..];
            format!("dbfs://{rest}")
        } else {
            trimmed.to_string()
        };

        if let Some((scheme, rest)) = split_scheme(&normalized) {
            return Ok(Self {
                raw: trimmed.to_string(),
                scheme,
                rest,
            });
        }

        Ok(Self {
            raw: trimmed.to_string(),
            scheme: "file".to_string(),
            rest: trimmed.to_string(),
        })
    }
}

struct FetchDeadline {
    started_at: Instant,
    timeout: Duration,
}

impl FetchDeadline {
    fn new(timeout_secs: u64) -> Option<Self> {
        if timeout_secs == 0 {
            return None;
        }
        Some(Self {
            started_at: Instant::now(),
            timeout: Duration::from_secs(timeout_secs),
        })
    }

    fn expired(&self) -> bool {
        self.started_at.elapsed() >= self.timeout
    }
}

fn resolve_provider(
    locator: &ManifestLocator,
    config: &DbtNovaConfig,
) -> Result<ManifestResolution> {
    match locator.scheme.as_str() {
        "file" => resolve_file(locator, config),
        "http" | "https" => resolve_http(locator, config),
        "dbfs" => resolve_dbfs(locator, config),
        "s3" => resolve_s3(locator, config),
        "gs" => resolve_gcs(locator, config),
        _ => Err(DbtNovaError::ServerError(format!(
            "Unsupported manifest URI scheme: {scheme} (supported: file,http,https,dbfs,s3,gs)",
            scheme = locator.scheme
        ))),
    }
}

#[derive(Debug)]
/// Resolved manifest metadata and on-disk cache location.
pub struct ManifestResolution {
    pub local_path: PathBuf,
    pub source_uri: String,
    pub cached: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct CacheMeta {
    source_uri: String,
    fetched_at_ms: u128,
    etag: Option<String>,
    last_modified: Option<String>,
    remote_modified_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub(crate) struct ManifestSignature {
    pub path: String,
    pub len: u64,
    pub modified_ms: u128,
    pub content_hash: String,
    pub prune_fingerprint: String,
    pub search_index_fingerprint: String,
    pub source_uri: String,
}

static CACHE_HITS: AtomicU64 = AtomicU64::new(0);
static CACHE_MISSES: AtomicU64 = AtomicU64::new(0);

pub(crate) fn manifest_cache_stats() -> (u64, u64) {
    (
        CACHE_HITS.load(Ordering::Relaxed),
        CACHE_MISSES.load(Ordering::Relaxed),
    )
}

fn record_cache_hit() {
    CACHE_HITS.fetch_add(1, Ordering::Relaxed);
}

fn record_cache_miss() {
    CACHE_MISSES.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn manifest_signature(path: &Path, source_uri: &str) -> Result<ManifestSignature> {
    let meta = fs::metadata(path)?;
    let modified = meta
        .modified()
        .or_else(|_| meta.created())
        .unwrap_or(UNIX_EPOCH);
    let modified_ms = modified
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let content_hash = hash_file(path)?;
    Ok(ManifestSignature {
        path: path.to_string_lossy().to_string(),
        len: meta.len(),
        modified_ms,
        content_hash,
        prune_fingerprint: String::new(),
        search_index_fingerprint: String::new(),
        source_uri: source_uri.to_string(),
    })
}

/// Resolve and cache the manifest based on configuration and URI.
///
/// # Errors
/// Returns an error if the URI is invalid or the manifest cannot be loaded.
pub fn resolve_manifest(config: &DbtNovaConfig) -> Result<ManifestResolution> {
    let uri = if config.manifest_uri.trim().is_empty() {
        config.manifest_path.trim().to_string()
    } else {
        config.manifest_uri.trim().to_string()
    };
    let locator = ManifestLocator::parse(&uri)?;
    resolve_provider(&locator, config)
}

const REMOTE_FETCH_MAX_ATTEMPTS: usize = 3;

fn split_scheme(uri: &str) -> Option<(String, String)> {
    let pos = uri.find("://")?;
    let scheme = uri[..pos].to_lowercase();
    let rest = uri[(pos + 3)..].to_string();
    Some((scheme, rest))
}

fn http_client(config: &DbtNovaConfig) -> Result<Client> {
    let mut builder = Client::builder();
    if config.manifest_http_connect_timeout_secs > 0 {
        builder = builder.connect_timeout(Duration::from_secs(
            config.manifest_http_connect_timeout_secs,
        ));
    }
    if config.manifest_http_timeout_secs > 0 {
        builder = builder.timeout(Duration::from_secs(config.manifest_http_timeout_secs));
    }
    builder
        .build()
        .map_err(|e| DbtNovaError::ServerError(format!("HTTP client init failed: {e}")))
}

fn deadline_expired(deadline: Option<&FetchDeadline>) -> Option<u64> {
    deadline
        .filter(|d| d.expired())
        .map(|d| d.timeout.as_secs())
}

#[allow(clippy::too_many_lines)]
fn fetch_http_manifest(url: &str, config: &DbtNovaConfig) -> Result<ManifestResolution> {
    let (cache_path, meta_path) = cache_paths(config, url)?;
    let mut meta = read_cache_meta(&meta_path).unwrap_or_default();
    let max_bytes = config.manifest_max_bytes;
    let deadline = FetchDeadline::new(config.manifest_fetch_timeout_secs);

    if cache_path.exists() && is_cache_fresh(&meta, config.manifest_refresh_secs) {
        record_cache_hit();
        return Ok(ManifestResolution {
            local_path: cache_path,
            source_uri: url.to_string(),
            cached: true,
        });
    }

    let client = http_client(config)?;

    let mut response = None;
    let mut last_err = None;
    for attempt in 1..=REMOTE_FETCH_MAX_ATTEMPTS {
        if let Some(timeout_secs) = deadline_expired(deadline.as_ref()) {
            if cache_path.exists() {
                record_cache_hit();
                warn!(
                    timeout_secs,
                    "Manifest fetch timed out; falling back to cached copy"
                );
                return Ok(ManifestResolution {
                    local_path: cache_path,
                    source_uri: url.to_string(),
                    cached: true,
                });
            }
            return Err(DbtNovaError::ServerError(format!(
                "Manifest fetch timed out after {timeout_secs}s"
            )));
        }
        let mut request = client.get(url);
        if let Some(etag) = meta.etag.as_ref() {
            request = request.header(reqwest::header::IF_NONE_MATCH, etag);
        }
        if let Some(last_modified) = meta.last_modified.as_ref() {
            request = request.header(reqwest::header::IF_MODIFIED_SINCE, last_modified);
        }
        match request.send() {
            Ok(resp) => {
                response = Some(resp);
                break;
            }
            Err(err) => {
                last_err = Some(err);
                if attempt < REMOTE_FETCH_MAX_ATTEMPTS {
                    std::thread::sleep(retry_backoff(attempt));
                }
            }
        }
    }
    let Some(response) = response else {
        if cache_path.exists() {
            record_cache_hit();
            warn!("Manifest fetch failed; falling back to cached copy");
            return Ok(ManifestResolution {
                local_path: cache_path,
                source_uri: url.to_string(),
                cached: true,
            });
        }
        record_cache_miss();
        let err = last_err.map_or_else(|| "unknown error".to_string(), |e| e.to_string());
        return Err(DbtNovaError::ServerError(format!(
            "Manifest fetch failed: {err}"
        )));
    };

    if response.status() == reqwest::StatusCode::NOT_MODIFIED && cache_path.exists() {
        record_cache_hit();
        meta.fetched_at_ms = now_ms();
        write_cache_meta(&meta_path, &meta)?;
        return Ok(ManifestResolution {
            local_path: cache_path,
            source_uri: url.to_string(),
            cached: true,
        });
    }

    if !response.status().is_success() {
        if cache_path.exists() {
            record_cache_hit();
            warn!(
                status = %response.status(),
                "manifest fetch failed; falling back to cached copy"
            );
            return Ok(ManifestResolution {
                local_path: cache_path,
                source_uri: url.to_string(),
                cached: true,
            });
        }
        record_cache_miss();
        let status = response.status();
        return Err(DbtNovaError::ServerError(format!(
            "Manifest fetch failed (HTTP {status})"
        )));
    }

    if max_bytes > 0
        && let Some(len) = response.content_length()
        && len > max_bytes
    {
        if cache_path.exists() {
            record_cache_hit();
            warn!(
                size = len,
                max_bytes = max_bytes,
                "manifest fetch exceeded size limit; falling back to cached copy"
            );
            return Ok(ManifestResolution {
                local_path: cache_path,
                source_uri: url.to_string(),
                cached: true,
            });
        }
        record_cache_miss();
        return Err(DbtNovaError::ServerError(format!(
            "Manifest fetch exceeded size limit ({len} > {max_bytes})",
        )));
    }

    let etag = response
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let last_modified = response
        .headers()
        .get(reqwest::header::LAST_MODIFIED)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);

    match write_limited_reader_atomic(&cache_path, response, max_bytes) {
        Ok(_) => {}
        Err(LimitedWriteError::LimitExceeded { observed, max }) => {
            if cache_path.exists() {
                record_cache_hit();
                warn!(
                    size = observed,
                    max_bytes = max,
                    "manifest fetch exceeded size limit; falling back to cached copy"
                );
                return Ok(ManifestResolution {
                    local_path: cache_path,
                    source_uri: url.to_string(),
                    cached: true,
                });
            }
            record_cache_miss();
            return Err(DbtNovaError::ServerError(format!(
                "Manifest fetch exceeded size limit (> {max} bytes)",
            )));
        }
        Err(LimitedWriteError::Io(error)) => {
            if cache_path.exists() {
                record_cache_hit();
                warn!(
                    error = %error,
                    "manifest fetch failed while writing response; falling back to cached copy"
                );
                return Ok(ManifestResolution {
                    local_path: cache_path,
                    source_uri: url.to_string(),
                    cached: true,
                });
            }
            record_cache_miss();
            return Err(DbtNovaError::ServerError(format!(
                "Manifest fetch failed: {error}"
            )));
        }
    }

    meta.source_uri = url.to_string();
    meta.fetched_at_ms = now_ms();
    meta.etag = etag;
    meta.last_modified = last_modified;
    write_cache_meta(&meta_path, &meta)?;

    // A fetch with a network response and successful write counts as a cache miss.
    record_cache_miss();
    Ok(ManifestResolution {
        local_path: cache_path,
        source_uri: url.to_string(),
        cached: false,
    })
}

#[allow(clippy::too_many_lines)]
fn fetch_dbfs_manifest(
    path: &str,
    uri: &str,
    config: &DbtNovaConfig,
) -> Result<ManifestResolution> {
    let host = std::env::var("DATABRICKS_HOST")
        .map_err(|_| DbtNovaError::ServerError("DATABRICKS_HOST not set".to_string()))?;
    let token = std::env::var("DATABRICKS_ACCESS_TOKEN")
        .map_err(|_| DbtNovaError::ServerError("DATABRICKS_ACCESS_TOKEN not set".to_string()))?;

    let (cache_path, meta_path) = cache_paths(config, uri)?;
    let mut meta = read_cache_meta(&meta_path).unwrap_or_default();
    let max_bytes = config.manifest_max_bytes;
    let deadline = FetchDeadline::new(config.manifest_fetch_timeout_secs);

    let client = http_client(config)?;

    let host = host.trim_end_matches('/');
    let path = path.trim_start_matches('/');
    let status_url = format!("{host}/api/2.0/dbfs/get-status");
    if let Some(timeout_secs) = deadline_expired(deadline.as_ref()) {
        if cache_path.exists() {
            record_cache_hit();
            warn!(
                timeout_secs,
                "DBFS status timed out; falling back to cached copy"
            );
            return Ok(ManifestResolution {
                local_path: cache_path,
                source_uri: uri.to_string(),
                cached: true,
            });
        }
        return Err(DbtNovaError::ServerError(format!(
            "DBFS status timed out after {timeout_secs}s"
        )));
    }

    let status_resp = match client
        .post(status_url)
        .bearer_auth(&token)
        .json(&serde_json::json!({ "path": format!("/{path}") }))
        .send()
    {
        Ok(resp) => resp,
        Err(err) => {
            if cache_path.exists() {
                warn!(
                    error = %err,
                    "DBFS status failed; falling back to cached copy"
                );
                return Ok(ManifestResolution {
                    local_path: cache_path,
                    source_uri: uri.to_string(),
                    cached: true,
                });
            }
            return Err(DbtNovaError::ServerError(format!(
                "DBFS status failed: {err}"
            )));
        }
    };

    if status_resp.status().is_success() {
        let status: JsonValue = status_resp
            .json()
            .map_err(|e| DbtNovaError::ServerError(format!("DBFS status parse failed: {e}")))?;
        let remote_modified = status.get("modification_time").and_then(JsonValue::as_u64);
        if let Some(remote_modified) = remote_modified {
            meta.remote_modified_ms = Some(remote_modified);
            if cache_path.exists()
                && is_cache_fresh(&meta, config.manifest_refresh_secs)
                && meta.remote_modified_ms == Some(remote_modified)
            {
                record_cache_hit();
                return Ok(ManifestResolution {
                    local_path: cache_path,
                    source_uri: uri.to_string(),
                    cached: true,
                });
            }
        }
    }

    if cache_path.exists() && is_cache_fresh(&meta, config.manifest_refresh_secs) {
        record_cache_hit();
        return Ok(ManifestResolution {
            local_path: cache_path,
            source_uri: uri.to_string(),
            cached: true,
        });
    }

    record_cache_miss();
    let read_url = format!("{host}/api/2.0/dbfs/read");
    let mut offset = 0u64;
    let mut buffer: Vec<u8> = Vec::new();
    loop {
        if let Some(timeout_secs) = deadline_expired(deadline.as_ref()) {
            if cache_path.exists() {
                record_cache_hit();
                warn!(
                    timeout_secs,
                    "DBFS fetch timed out; falling back to cached copy"
                );
                return Ok(ManifestResolution {
                    local_path: cache_path,
                    source_uri: uri.to_string(),
                    cached: true,
                });
            }
            return Err(DbtNovaError::ServerError(format!(
                "DBFS fetch timed out after {timeout_secs}s"
            )));
        }
        let mut resp = None;
        let mut last_err = None;
        for attempt in 1..=REMOTE_FETCH_MAX_ATTEMPTS {
            if let Some(timeout_secs) = deadline_expired(deadline.as_ref()) {
                if cache_path.exists() {
                    record_cache_hit();
                    warn!(
                        timeout_secs,
                        "DBFS read timed out; falling back to cached copy"
                    );
                    return Ok(ManifestResolution {
                        local_path: cache_path,
                        source_uri: uri.to_string(),
                        cached: true,
                    });
                }
                return Err(DbtNovaError::ServerError(format!(
                    "DBFS read timed out after {timeout_secs}s"
                )));
            }
            let request = client
                .get(&read_url)
                .bearer_auth(&token)
                .query(&[
                    ("path", format!("/{path}")),
                    ("offset", offset.to_string()),
                    ("length", (1024 * 1024u64).to_string()),
                ])
                .send();
            match request {
                Ok(response) => {
                    resp = Some(response);
                    break;
                }
                Err(err) => {
                    last_err = Some(err);
                    if attempt < REMOTE_FETCH_MAX_ATTEMPTS {
                        std::thread::sleep(retry_backoff(attempt));
                    }
                }
            }
        }
        let Some(resp) = resp else {
            if cache_path.exists() {
                record_cache_hit();
                warn!(
                    error = %last_err.map_or_else(String::new, |e| e.to_string()),
                    "DBFS read failed; falling back to cached copy"
                );
                return Ok(ManifestResolution {
                    local_path: cache_path,
                    source_uri: uri.to_string(),
                    cached: true,
                });
            }
            let err = last_err.map_or_else(|| "unknown error".to_string(), |e| e.to_string());
            return Err(DbtNovaError::ServerError(format!(
                "DBFS read failed: {err}"
            )));
        };
        if !resp.status().is_success() {
            if cache_path.exists() {
                warn!(
                    status = %resp.status(),
                    "DBFS read failed; falling back to cached copy"
                );
                return Ok(ManifestResolution {
                    local_path: cache_path,
                    source_uri: uri.to_string(),
                    cached: true,
                });
            }
            let status = resp.status();
            return Err(DbtNovaError::ServerError(format!(
                "DBFS read failed (HTTP {status})"
            )));
        }
        let body: JsonValue = resp
            .json()
            .map_err(|e| DbtNovaError::ServerError(format!("DBFS read parse failed: {e}")))?;
        let bytes_str = dbfs_read_data_field(&body, offset);
        let chunk = BASE64
            .decode(bytes_str.as_bytes())
            .map_err(|e| DbtNovaError::ServerError(format!("DBFS decode failed: {e}")))?;
        let bytes_read = dbfs_read_bytes_read_field(&body, offset);
        buffer.extend_from_slice(&chunk);
        if max_bytes > 0 && buffer.len() as u64 > max_bytes {
            if cache_path.exists() {
                record_cache_hit();
                warn!(
                    size = buffer.len(),
                    max_bytes = max_bytes,
                    "DBFS manifest exceeded size limit; falling back to cached copy"
                );
                return Ok(ManifestResolution {
                    local_path: cache_path,
                    source_uri: uri.to_string(),
                    cached: true,
                });
            }
            return Err(DbtNovaError::ServerError(format!(
                "DBFS manifest exceeded size limit (> {max_bytes} bytes)",
            )));
        }
        if bytes_read == 0 || chunk.is_empty() {
            break;
        }
        offset = offset.saturating_add(bytes_read);
    }

    write_atomic(&cache_path, &buffer)?;
    meta.source_uri = uri.to_string();
    meta.fetched_at_ms = now_ms();
    write_cache_meta(&meta_path, &meta)?;

    Ok(ManifestResolution {
        local_path: cache_path,
        source_uri: uri.to_string(),
        cached: false,
    })
}

fn dbfs_read_data_field(body: &JsonValue, offset: u64) -> &str {
    if let Some(data) = body.get("data").and_then(JsonValue::as_str) {
        data
    } else {
        warn!(
            offset = offset,
            "DBFS read response missing string 'data' field; treating chunk as empty"
        );
        ""
    }
}

fn dbfs_read_bytes_read_field(body: &JsonValue, offset: u64) -> u64 {
    if let Some(bytes_read) = body.get("bytes_read").and_then(JsonValue::as_u64) {
        bytes_read
    } else {
        warn!(
            offset = offset,
            "DBFS read response missing numeric 'bytes_read' field; treating as zero"
        );
        0
    }
}

fn fetch_s3_manifest(rest: &str, uri: &str, config: &DbtNovaConfig) -> Result<ManifestResolution> {
    let mode = env_mode("DBT_NOVA_S3_MODE");
    if mode == "sdk" {
        #[cfg(feature = "s3")]
        {
            return fetch_s3_manifest_sdk(rest, uri, config);
        }
        #[cfg(not(feature = "s3"))]
        {
            return Err(DbtNovaError::ServerError(
                "S3 SDK support not enabled; build with --features s3 or set DBT_NOVA_S3_MODE=https"
                    .to_string(),
            ));
        }
    }

    let url = s3_to_https(rest)?;
    fetch_http_manifest(&url, config).map(|mut res| {
        res.source_uri = uri.to_string();
        res
    })
}

fn fetch_gcs_manifest(rest: &str, uri: &str, config: &DbtNovaConfig) -> Result<ManifestResolution> {
    let mode = env_mode("DBT_NOVA_GCS_MODE");
    if mode == "sdk" {
        #[cfg(feature = "gcs")]
        {
            return fetch_gcs_manifest_sdk(rest, uri, config);
        }
        #[cfg(not(feature = "gcs"))]
        {
            return Err(DbtNovaError::ServerError(
                "GCS SDK support not enabled; build with --features gcs or set DBT_NOVA_GCS_MODE=https"
                    .to_string(),
            ));
        }
    }

    let url = gcs_to_https(rest)?;
    fetch_http_manifest(&url, config).map(|mut res| {
        res.source_uri = uri.to_string();
        res
    })
}

fn cache_paths(config: &DbtNovaConfig, uri: &str) -> Result<(PathBuf, PathBuf)> {
    let cache_dir = config.manifest_cache_dir()?;
    fs::create_dir_all(&cache_dir)?;
    let hash = blake3::hash(uri.as_bytes()).to_hex().to_string();
    let cache_path = cache_dir.join(format!("{hash}.json"));
    let meta_path = cache_dir.join(format!("{hash}.meta.json"));
    Ok((cache_path, meta_path))
}

fn read_cache_meta(path: &Path) -> Result<CacheMeta> {
    let file = fs::File::open(path)?;
    serde_json::from_reader(file)
        .map_err(|e| DbtNovaError::ServerError(format!("Invalid manifest cache metadata: {e}")))
}

fn write_cache_meta(path: &Path, meta: &CacheMeta) -> Result<()> {
    let data = serde_json::to_vec(meta).map_err(|e| {
        DbtNovaError::ServerError(format!("Failed to encode manifest cache metadata: {e}"))
    })?;
    write_atomic(path, &data)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("manifest.cache");
    let tmp = path.with_file_name(format!("{file_name}.{}.tmp", unique_suffix()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(tmp, path)?;
    Ok(())
}

#[derive(Debug)]
enum LimitedWriteError {
    Io(std::io::Error),
    LimitExceeded { observed: u64, max: u64 },
}

fn write_limited_reader_atomic<R: Read>(
    path: &Path,
    mut reader: R,
    max_bytes: u64,
) -> std::result::Result<u64, LimitedWriteError> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("manifest.cache");
    let tmp = path.with_file_name(format!("{file_name}.{}.tmp", unique_suffix()));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .map_err(LimitedWriteError::Io)?;
        let mut written = 0u64;
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let read = reader.read(&mut buf).map_err(LimitedWriteError::Io)?;
            if read == 0 {
                break;
            }
            written = written.saturating_add(read as u64);
            if max_bytes > 0 && written > max_bytes {
                return Err(LimitedWriteError::LimitExceeded {
                    observed: written,
                    max: max_bytes,
                });
            }
            file.write_all(&buf[..read])
                .map_err(LimitedWriteError::Io)?;
        }
        file.sync_all().map_err(LimitedWriteError::Io)?;
        fs::rename(&tmp, path).map_err(LimitedWriteError::Io)?;
        Ok(written)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

fn hash_file(path: &Path) -> Result<String> {
    let file = fs::File::open(path)?;
    let mut reader = std::io::BufReader::new(file);
    let mut hasher = blake3::Hasher::new();
    let mut buf = [0u8; 8192];
    loop {
        let read = reader.read(&mut buf)?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn is_cache_fresh(meta: &CacheMeta, refresh_secs: u64) -> bool {
    if refresh_secs == 0 {
        return true;
    }
    let age_ms = now_ms().saturating_sub(meta.fetched_at_ms);
    age_ms < u128::from(refresh_secs).saturating_mul(1000)
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0))
        .as_millis()
}

fn retry_backoff(attempt: usize) -> Duration {
    let base = 200u64;
    let shift = u32::try_from(attempt.saturating_sub(1)).unwrap_or(u32::MAX);
    let factor = 1u64.checked_shl(shift).unwrap_or(u64::MAX);
    let delay = base.saturating_mul(factor).min(2_000);
    Duration::from_millis(delay)
}

fn env_mode(name: &str) -> String {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_lowercase())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "https".to_string())
}

fn s3_to_https(rest: &str) -> Result<String> {
    let (bucket, key) = split_bucket_key(rest)?;
    let endpoint =
        std::env::var("DBT_NOVA_S3_ENDPOINT").unwrap_or_else(|_| "s3.amazonaws.com".into());
    let endpoint = endpoint.trim_end_matches('/');
    Ok(format!("https://{endpoint}/{bucket}/{key}"))
}

fn gcs_to_https(rest: &str) -> Result<String> {
    let (bucket, key) = split_bucket_key(rest)?;
    let endpoint =
        std::env::var("DBT_NOVA_GCS_ENDPOINT").unwrap_or_else(|_| "storage.googleapis.com".into());
    let endpoint = endpoint.trim_end_matches('/');
    Ok(format!("https://{endpoint}/{bucket}/{key}"))
}

fn split_bucket_key(rest: &str) -> Result<(String, String)> {
    let mut parts = rest.splitn(2, '/');
    let bucket = parts
        .next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| DbtNovaError::ServerError("Invalid manifest URI bucket".to_string()))?;
    let key = parts
        .next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| DbtNovaError::ServerError("Invalid manifest URI key".to_string()))?;
    Ok((bucket.to_string(), key.to_string()))
}

#[cfg(any(feature = "s3", feature = "gcs"))]
fn fetch_sdk_manifest_with_cache<F>(
    provider: &'static str,
    uri: &str,
    config: &DbtNovaConfig,
    fetch_once: F,
) -> Result<ManifestResolution>
where
    F: Fn() -> Result<Vec<u8>>,
{
    let (cache_path, meta_path) = cache_paths(config, uri)?;
    let mut meta = read_cache_meta(&meta_path).unwrap_or_default();
    let deadline = FetchDeadline::new(config.manifest_fetch_timeout_secs);

    if cache_path.exists() && is_cache_fresh(&meta, config.manifest_refresh_secs) {
        record_cache_hit();
        return Ok(ManifestResolution {
            local_path: cache_path,
            source_uri: uri.to_string(),
            cached: true,
        });
    }

    record_cache_miss();
    let mut bytes = None;
    let mut last_err = None;
    for attempt in 1..=REMOTE_FETCH_MAX_ATTEMPTS {
        if let Some(timeout_secs) = deadline_expired(deadline.as_ref()) {
            if cache_path.exists() {
                record_cache_hit();
                warn!(
                    provider = provider,
                    timeout_secs, "{provider} download timed out; falling back to cached copy"
                );
                return Ok(ManifestResolution {
                    local_path: cache_path,
                    source_uri: uri.to_string(),
                    cached: true,
                });
            }
            return Err(DbtNovaError::ServerError(format!(
                "{provider} download timed out after {timeout_secs}s"
            )));
        }

        match fetch_once() {
            Ok(data) => {
                bytes = Some(data);
                break;
            }
            Err(err) => {
                last_err = Some(err);
                if attempt < REMOTE_FETCH_MAX_ATTEMPTS {
                    std::thread::sleep(retry_backoff(attempt));
                }
            }
        }
    }

    let Some(bytes) = bytes else {
        if cache_path.exists() {
            record_cache_hit();
            if let Some(err) = &last_err {
                warn!(
                    provider = provider,
                    error = %err,
                    "{provider} download failed; falling back to cached copy"
                );
            } else {
                warn!(
                    provider = provider,
                    "{provider} download failed; falling back to cached copy"
                );
            }
            return Ok(ManifestResolution {
                local_path: cache_path,
                source_uri: uri.to_string(),
                cached: true,
            });
        }
        return Err(last_err.unwrap_or_else(|| {
            DbtNovaError::ServerError(format!("{provider} download failed: unknown error"))
        }));
    };

    if config.manifest_max_bytes > 0 && bytes.len() as u64 > config.manifest_max_bytes {
        if cache_path.exists() {
            record_cache_hit();
            warn!(
                provider = provider,
                size = bytes.len(),
                max_bytes = config.manifest_max_bytes,
                "{provider} manifest exceeded size limit; falling back to cached copy"
            );
            return Ok(ManifestResolution {
                local_path: cache_path,
                source_uri: uri.to_string(),
                cached: true,
            });
        }
        return Err(DbtNovaError::ServerError(format!(
            "{provider} manifest exceeded size limit (> {} bytes)",
            config.manifest_max_bytes
        )));
    }

    write_atomic(&cache_path, &bytes)?;
    meta.source_uri = uri.to_string();
    meta.fetched_at_ms = now_ms();
    write_cache_meta(&meta_path, &meta)?;

    Ok(ManifestResolution {
        local_path: cache_path,
        source_uri: uri.to_string(),
        cached: false,
    })
}

#[cfg(any(feature = "s3", feature = "gcs"))]
fn run_async_fetch_blocking<F, Fut, T>(factory: F) -> Result<T>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<T>> + Send + 'static,
    T: Send + 'static,
{
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        return match handle.runtime_flavor() {
            tokio::runtime::RuntimeFlavor::CurrentThread => {
                let (tx, rx) = std::sync::mpsc::sync_channel(1);
                std::thread::spawn(move || {
                    let result = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|e| {
                            DbtNovaError::ServerError(format!("Failed to init runtime: {e}"))
                        })
                        .and_then(|rt| rt.block_on(factory()));
                    let _ = tx.send(result);
                });
                rx.recv().map_err(|e| {
                    DbtNovaError::ServerError(format!(
                        "Failed to receive async fetch result from worker thread: {e}"
                    ))
                })?
            }
            tokio::runtime::RuntimeFlavor::MultiThread | _ => {
                tokio::task::block_in_place(|| handle.block_on(factory()))
            }
        };
    }

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| DbtNovaError::ServerError(format!("Failed to init runtime: {e}")))?;
    rt.block_on(factory())
}

#[cfg(feature = "s3")]
fn fetch_s3_manifest_sdk(
    rest: &str,
    uri: &str,
    config: &DbtNovaConfig,
) -> Result<ManifestResolution> {
    let (bucket, key) = split_bucket_key(rest)?;
    let timeout_secs = config.manifest_http_timeout_secs;
    fetch_sdk_manifest_with_cache("S3", uri, config, || {
        let bucket = bucket.clone();
        let key = key.clone();
        run_async_fetch_blocking(move || async move {
            let fetch = async {
                let shared = aws_config::defaults(aws_config::BehaviorVersion::latest())
                    .load()
                    .await;
                let client = aws_sdk_s3::Client::new(&shared);
                let response = client
                    .get_object()
                    .bucket(bucket.clone())
                    .key(key.clone())
                    .send()
                    .await
                    .map_err(|e| DbtNovaError::ServerError(format!("S3 get_object failed: {e}")))?;
                let data = response
                    .body
                    .collect()
                    .await
                    .map_err(|e| DbtNovaError::ServerError(format!("S3 read failed: {e}")))?
                    .into_bytes();
                Ok::<_, DbtNovaError>(data.to_vec())
            };
            if timeout_secs > 0 {
                tokio::time::timeout(Duration::from_secs(timeout_secs), fetch)
                    .await
                    .map_err(|_| DbtNovaError::ServerError("S3 download timed out".to_string()))?
            } else {
                fetch.await
            }
        })
    })
}

#[cfg(feature = "gcs")]
fn fetch_gcs_manifest_sdk(
    rest: &str,
    uri: &str,
    config: &DbtNovaConfig,
) -> Result<ManifestResolution> {
    let (bucket, object) = split_bucket_key(rest)?;
    let timeout_secs = config.manifest_http_timeout_secs;
    fetch_sdk_manifest_with_cache("GCS", uri, config, || {
        let bucket = bucket.clone();
        let object = object.clone();
        run_async_fetch_blocking(move || async move {
            let fetch = async {
                let gcs_config = google_cloud_storage::client::ClientConfig::default()
                    .with_auth()
                    .await
                    .map_err(|e| DbtNovaError::ServerError(format!("GCS auth failed: {e}")))?;
                let client = google_cloud_storage::client::Client::new(gcs_config);
                let req = google_cloud_storage::http::objects::get::GetObjectRequest {
                    bucket: bucket.clone(),
                    object: object.clone(),
                    ..Default::default()
                };
                client
                    .download_object(
                        &req,
                        &google_cloud_storage::http::objects::download::Range::default(),
                    )
                    .await
                    .map_err(|e| DbtNovaError::ServerError(format!("GCS download failed: {e}")))
            };
            if timeout_secs > 0 {
                tokio::time::timeout(Duration::from_secs(timeout_secs), fetch)
                    .await
                    .map_err(|_| DbtNovaError::ServerError("GCS download timed out".to_string()))?
            } else {
                fetch.await
            }
        })
    })
}

#[cfg(test)]
mod tests;
