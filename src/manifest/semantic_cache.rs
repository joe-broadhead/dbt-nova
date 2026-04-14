use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};

use tracing::warn;

use crate::config::SearchConfig;

static WARNED_LEGACY_CACHE_PATHS: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

const MANIFEST_CACHE_DIR: &str = "manifests";
const MISSING_MANIFEST_HASH: &str = "_missing_manifest_hash";
const DEFAULT_SPARSE_MODEL_NAME: &str = "Qdrant/Splade_PP_en_v1";
const DEFAULT_EMBEDDINGS_CACHE_DIRNAME: &str = ".dbt-nova/.fastembed_cache";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticCacheComponent {
    Dense,
    Sparse,
}

#[derive(Debug, Clone)]
pub struct SemanticCachePaths {
    pub cache_root: PathBuf,
    pub manifest_dir: PathBuf,
    pub compressed_path: PathBuf,
    pub raw_path: PathBuf,
    pub legacy_compressed_path: PathBuf,
    pub legacy_raw_path: PathBuf,
    pub manifest_hash: String,
    pub model_slug: String,
}

impl SemanticCacheComponent {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Dense => "vector",
            Self::Sparse => "sparse",
        }
    }

    #[must_use]
    pub fn file_prefix(self) -> &'static str {
        match self {
            Self::Dense => "dense",
            Self::Sparse => "sparse",
        }
    }

    #[must_use]
    pub fn legacy_file_zst(self) -> &'static str {
        match self {
            Self::Dense => "embeddings.rkyv.zst",
            Self::Sparse => "sparse_embeddings.rkyv.zst",
        }
    }

    #[must_use]
    pub fn legacy_file_raw(self) -> &'static str {
        match self {
            Self::Dense => "embeddings.rkyv",
            Self::Sparse => "sparse_embeddings.rkyv",
        }
    }
}

impl SemanticCachePaths {
    #[must_use]
    pub fn present(&self) -> bool {
        self.compressed_path.is_file() || self.raw_path.is_file()
    }

    #[must_use]
    pub fn legacy_present(&self) -> bool {
        self.legacy_compressed_path.is_file() || self.legacy_raw_path.is_file()
    }

    #[must_use]
    pub fn preferred_path(&self) -> PathBuf {
        if self.compressed_path.is_file() {
            self.compressed_path.clone()
        } else {
            self.raw_path.clone()
        }
    }
}

pub fn embeddings_cache_dir(config: &SearchConfig) -> PathBuf {
    let raw = config.embedding_cache_dir.trim();
    let path = if raw.is_empty() {
        if let Ok(exe_path) = std::env::current_exe()
            && let Some(parent) = exe_path.parent()
        {
            let bundled = parent.join("models");
            if bundled.is_dir() {
                return bundled;
            }
        }
        if let Ok(home) = std::env::var("HOME") {
            let home_path = PathBuf::from(&home);
            let bundled = home_path.join(".local").join("bin").join("models");
            if bundled.is_dir() {
                return bundled;
            }

            home_path.join(DEFAULT_EMBEDDINGS_CACHE_DIRNAME)
        } else {
            PathBuf::from(DEFAULT_EMBEDDINGS_CACHE_DIRNAME)
        }
    } else {
        PathBuf::from(raw)
    };
    if let Err(err) = fs::create_dir_all(&path) {
        warn!(error = %err, cache_dir = %path.display(), "failed to create embeddings cache dir");
    }
    path
}

#[must_use]
pub fn default_sparse_model_name() -> &'static str {
    DEFAULT_SPARSE_MODEL_NAME
}

#[must_use]
pub fn model_slug(model_name: &str) -> String {
    let trimmed = model_name.trim();
    if trimmed.is_empty() {
        return "default".to_string();
    }

    let mut slug = String::with_capacity(trimmed.len());
    for ch in trimmed.chars() {
        match ch {
            '/' | '\\' => slug.push_str("--"),
            'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '_' | '-' => slug.push(ch),
            _ => slug.push('-'),
        }
    }
    slug
}

#[must_use]
pub fn cache_paths(
    config: &SearchConfig,
    component: SemanticCacheComponent,
    model_name: &str,
    manifest_hash: &str,
) -> SemanticCachePaths {
    let cache_root = embeddings_cache_dir(config);
    let manifest_hash = manifest_hash_segment(manifest_hash);
    let model_slug = model_slug(model_name);
    let file_stem = format!("{}__{model_slug}", component.file_prefix());
    let manifest_dir = cache_root.join(MANIFEST_CACHE_DIR).join(&manifest_hash);

    SemanticCachePaths {
        cache_root: cache_root.clone(),
        manifest_dir: manifest_dir.clone(),
        compressed_path: manifest_dir.join(format!("{file_stem}.rkyv.zst")),
        raw_path: manifest_dir.join(format!("{file_stem}.rkyv")),
        legacy_compressed_path: cache_root.join(component.legacy_file_zst()),
        legacy_raw_path: cache_root.join(component.legacy_file_raw()),
        manifest_hash,
        model_slug,
    }
}

pub fn warn_if_legacy_cache_present(paths: &SemanticCachePaths, component: SemanticCacheComponent) {
    if !paths.legacy_present() {
        return;
    }

    let key = format!("{}:{}", component.name(), paths.cache_root.display());
    let should_warn = {
        let mut warned = WARNED_LEGACY_CACHE_PATHS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        warned.insert(key)
    };
    if should_warn {
        warn!(
            component = component.name(),
            legacy_compressed_path = %paths.legacy_compressed_path.display(),
            legacy_raw_path = %paths.legacy_raw_path.display(),
            "legacy singleton semantic cache files are ignored; use manifest-scoped caches instead"
        );
    }
}

fn manifest_hash_segment(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        MISSING_MANIFEST_HASH.to_string()
    } else {
        trimmed.to_string()
    }
}
