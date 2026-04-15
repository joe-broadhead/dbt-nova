use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use blake3;
use flate2::read::GzDecoder;
use tar::Archive;
use tracing::{info, warn};

use crate::config::{ArtifactFetchPolicy, DbtNovaConfig, SearchConfig};
use crate::error::{DbtNovaError, Result};
use crate::manifest::prebuilt_assets::PrebuiltAssetsMetadata;
use crate::manifest::semantic_cache::{self, SemanticCacheComponent, default_sparse_model_name};
use crate::manifest::source::resolve_manifest;
#[cfg(feature = "embeddings")]
use crate::manifest::vector_search::{
    RequiredLocalModelLayout as RuntimeRequiredModelLayout, required_embedding_model_layout,
    required_reranker_model_layout, required_sparse_model_layout,
};
use crate::utils::{sanitize_uri, unique_suffix};

/// Result of prebuilt artifact materialization.
#[derive(Debug, Clone)]
pub struct FileArtifactMaterialization {
    pub metadata: PrebuiltAssetsMetadata,
    pub storage_materialized: bool,
    pub models_materialized: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ModelsCachePresence {
    Missing,
    Valid,
    Invalid(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RequiredModelLayout {
    component: &'static str,
    model_code: String,
    model_file: String,
    additional_files: Vec<&'static str>,
}

/// Materialize prebuilt artifacts from configured URIs.
///
/// # Errors
///
/// Returns an error when URIs are invalid, metadata validation fails, or archive
/// extraction cannot be completed safely.
pub fn materialize_file_artifacts(
    config: &DbtNovaConfig,
    expected_manifest_hash: &str,
) -> Result<Option<FileArtifactMaterialization>> {
    if !config.remote_artifact_mode_enabled() {
        return Ok(None);
    }

    let storage_target = config.storage_root_dir()?;
    let models_target = semantic_cache::embeddings_cache_dir(&config.search);
    info!(
        expected_manifest_hash,
        fetch_policy = artifact_fetch_policy_label(config.artifact_fetch_policy),
        storage_target = %storage_target.display(),
        models_target = %models_target.display(),
        storage_uri = %sanitize_uri(&config.storage_artifact_uri),
        metadata_uri = %sanitize_uri(&config.metadata_artifact_uri),
        models_uri = %sanitize_uri(&config.models_artifact_uri),
        "evaluating file artifact materialization"
    );

    let metadata_path = resolve_artifact_uri_to_local(
        config,
        "DBT_NOVA_METADATA_ARTIFACT_URI",
        &config.metadata_artifact_uri,
    )?;
    ensure_regular_file("DBT_NOVA_METADATA_ARTIFACT_URI", &metadata_path)?;
    let metadata_raw = read_small_text_file(&metadata_path, config.manifest_max_bytes)?;
    let metadata = PrebuiltAssetsMetadata::from_json_str(&metadata_raw)?;
    validate_metadata_against_runtime(&metadata, config, expected_manifest_hash)?;

    if !config.models_artifact_uri.trim().is_empty() && !metadata.has_models_artifact() {
        return Err(DbtNovaError::InvalidParams(
            "DBT_NOVA_MODELS_ARTIFACT_URI is set but metadata contract has no artifact_name_models"
                .to_string(),
        ));
    }

    let storage_present = storage_instance_present(config)?;
    let should_materialize_storage = should_materialize(
        "storage artifacts",
        config.artifact_fetch_policy,
        storage_present,
    )?;
    ensure_materialization_allowed(config, should_materialize_storage, "storage artifacts")?;
    let storage_materialized = if should_materialize_storage {
        let storage_archive_path = resolve_artifact_uri_to_local(
            config,
            "DBT_NOVA_STORAGE_ARTIFACT_URI",
            &config.storage_artifact_uri,
        )?;
        ensure_regular_file("DBT_NOVA_STORAGE_ARTIFACT_URI", &storage_archive_path)?;
        extract_archive_atomically(&storage_archive_path, &storage_target)?;
        true
    } else {
        info!(
            target = %storage_target.display(),
            "skipping storage artifact materialization"
        );
        false
    };

    let mut models_materialized = false;
    if !config.models_artifact_uri.trim().is_empty() {
        let models_presence =
            inspect_models_cache_presence(config, &models_target, expected_manifest_hash)?;
        let should_materialize_models = should_materialize_models(
            config.artifact_fetch_policy,
            &models_target,
            &models_presence,
        )?;
        ensure_materialization_allowed(config, should_materialize_models, "models artifacts")?;
        if should_materialize_models {
            let models_archive_path = resolve_artifact_uri_to_local(
                config,
                "DBT_NOVA_MODELS_ARTIFACT_URI",
                &config.models_artifact_uri,
            )?;
            ensure_regular_file("DBT_NOVA_MODELS_ARTIFACT_URI", &models_archive_path)?;
            extract_archive_atomically_with_validation(
                &models_archive_path,
                &models_target,
                |cache_dir| validate_models_cache_layout(config, cache_dir, expected_manifest_hash),
            )?;
            models_materialized = true;
        } else {
            info!(
                target = %models_target.display(),
                "skipping models artifact materialization"
            );
        }
    }

    info!(
        storage_uri = %sanitize_uri(&config.storage_artifact_uri),
        metadata_uri = %sanitize_uri(&config.metadata_artifact_uri),
        models_uri = %sanitize_uri(&config.models_artifact_uri),
        storage_materialized,
        models_materialized,
        "file artifact materialization evaluated"
    );

    Ok(Some(FileArtifactMaterialization {
        metadata,
        storage_materialized,
        models_materialized,
    }))
}

#[derive(Debug, Clone)]
struct ArtifactLocator {
    raw: String,
    scheme: String,
}

pub(crate) fn resolve_artifact_uri_to_local(
    config: &DbtNovaConfig,
    name: &str,
    uri: &str,
) -> Result<PathBuf> {
    let locator = parse_artifact_locator(name, uri)?;
    let sanitized_uri = sanitize_uri(&locator.raw);
    let policy = artifact_fetch_policy_label(config.artifact_fetch_policy);
    if locator.scheme == "file" {
        info!(
            artifact = name,
            scheme = %locator.scheme,
            fetch_policy = policy,
            uri = %sanitized_uri,
            "artifact resolver using local file URI"
        );
        return resolve_file_artifact_uri(name, &locator.raw);
    }

    resolve_remote_artifact_uri(config, name, &locator, &sanitized_uri, policy)
}

fn resolve_remote_artifact_uri(
    config: &DbtNovaConfig,
    name: &str,
    locator: &ArtifactLocator,
    sanitized_uri: &str,
    policy: &'static str,
) -> Result<PathBuf> {
    let cache_dir = config.artifacts_cache_dir()?;
    fs::create_dir_all(&cache_dir)?;
    let (cache_path, meta_path) = artifact_cache_paths(&cache_dir, &locator.raw);

    if let Some(path) = maybe_reuse_cached_artifact(
        config,
        name,
        locator,
        sanitized_uri,
        policy,
        &cache_path,
        &meta_path,
    )? {
        return Ok(path);
    }

    fetch_remote_artifact(
        config,
        name,
        locator,
        sanitized_uri,
        policy,
        cache_dir.as_path(),
    )
}

fn maybe_reuse_cached_artifact(
    config: &DbtNovaConfig,
    name: &str,
    locator: &ArtifactLocator,
    sanitized_uri: &str,
    policy: &'static str,
    cache_path: &Path,
    meta_path: &Path,
) -> Result<Option<PathBuf>> {
    match config.artifact_fetch_policy {
        ArtifactFetchPolicy::Never => {
            if cache_path.exists() {
                info!(
                    artifact = name,
                    scheme = %locator.scheme,
                    fetch_policy = policy,
                    cache_hit = true,
                    fetched = false,
                    uri = %sanitized_uri,
                    cache_path = %cache_path.display(),
                    "artifact resolver using cached artifact (policy=never)"
                );
                return Ok(Some(cache_path.to_path_buf()));
            }
            warn!(
                artifact = name,
                scheme = %locator.scheme,
                fetch_policy = policy,
                cache_hit = false,
                fetched = false,
                uri = %sanitized_uri,
                "artifact resolver missing cached artifact (policy=never)"
            );
            Err(DbtNovaError::ServerError(format!(
                "artifact fetch policy is 'never' but no cached copy exists for {name}: {sanitized_uri}"
            )))
        }
        ArtifactFetchPolicy::IfMissing => {
            if cache_path.exists() {
                info!(
                    artifact = name,
                    scheme = %locator.scheme,
                    fetch_policy = policy,
                    cache_hit = true,
                    fetched = false,
                    uri = %sanitized_uri,
                    cache_path = %cache_path.display(),
                    "artifact resolver using cached artifact (policy=if_missing)"
                );
                return Ok(Some(cache_path.to_path_buf()));
            }
            Ok(None)
        }
        ArtifactFetchPolicy::Always => {
            info!(
                artifact = name,
                scheme = %locator.scheme,
                fetch_policy = policy,
                cache_hit = cache_path.exists(),
                fetched = true,
                uri = %sanitized_uri,
                "artifact resolver forcing refetch (policy=always)"
            );
            let _ = fs::remove_file(cache_path);
            let _ = fs::remove_file(meta_path);
            Ok(None)
        }
    }
}

fn fetch_remote_artifact(
    config: &DbtNovaConfig,
    name: &str,
    locator: &ArtifactLocator,
    sanitized_uri: &str,
    policy: &'static str,
    cache_dir: &Path,
) -> Result<PathBuf> {
    let fetch_config = build_remote_artifact_fetch_config(config, &locator.raw, cache_dir);

    info!(
        artifact = name,
        scheme = %locator.scheme,
        fetch_policy = policy,
        cache_hit = false,
        fetched = true,
        allow_http = config.artifact_allow_http,
        timeout_secs = config.artifact_timeout_secs,
        uri = %sanitized_uri,
        "artifact resolver fetching remote artifact"
    );
    let resolution = resolve_manifest(&fetch_config)?;
    info!(
        artifact = name,
        scheme = %locator.scheme,
        fetch_policy = policy,
        fetched = !resolution.cached,
        cached_copy_used = resolution.cached,
        uri = %sanitized_uri,
        local_path = %resolution.local_path.display(),
        "artifact resolver completed remote artifact resolution"
    );
    Ok(resolution.local_path)
}

fn build_remote_artifact_fetch_config(
    config: &DbtNovaConfig,
    uri: &str,
    cache_dir: &Path,
) -> DbtNovaConfig {
    let mut fetch_config = config.clone();
    fetch_config.manifest_path = String::new();
    fetch_config.manifest_uri = uri.to_string();
    fetch_config.manifest_cache_dir = cache_dir.to_string_lossy().to_string();
    // Artifacts can exceed manifest limits; metadata size is enforced after fetch.
    fetch_config.manifest_max_bytes = 0;
    fetch_config.manifest_refresh_secs = 0;
    fetch_config.manifest_allow_http = config.artifact_allow_http;
    fetch_config.manifest_fetch_timeout_secs = config.artifact_timeout_secs;
    fetch_config.manifest_http_timeout_secs = config.artifact_timeout_secs;
    fetch_config.manifest_http_connect_timeout_secs = config.artifact_timeout_secs;
    fetch_config
}

fn parse_artifact_locator(name: &str, uri: &str) -> Result<ArtifactLocator> {
    let trimmed = uri.trim();
    if trimmed.is_empty() {
        return Err(DbtNovaError::InvalidParams(format!(
            "{name} cannot be empty"
        )));
    }

    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("dbfs:/") && !lower.starts_with("dbfs://") {
        return Ok(ArtifactLocator {
            raw: trimmed.to_string(),
            scheme: "dbfs".to_string(),
        });
    }

    if let Some((scheme, _)) = lower.split_once("://") {
        return Ok(ArtifactLocator {
            raw: trimmed.to_string(),
            scheme: scheme.to_string(),
        });
    }

    Ok(ArtifactLocator {
        raw: trimmed.to_string(),
        scheme: "file".to_string(),
    })
}

fn artifact_cache_paths(cache_dir: &Path, uri: &str) -> (PathBuf, PathBuf) {
    let hash = blake3::hash(uri.as_bytes()).to_hex().to_string();
    let cache_path = cache_dir.join(format!("{hash}.json"));
    let meta_path = cache_dir.join(format!("{hash}.meta.json"));
    (cache_path, meta_path)
}

fn artifact_fetch_policy_label(policy: ArtifactFetchPolicy) -> &'static str {
    match policy {
        ArtifactFetchPolicy::IfMissing => "if_missing",
        ArtifactFetchPolicy::Always => "always",
        ArtifactFetchPolicy::Never => "never",
    }
}

fn ensure_materialization_allowed(
    config: &DbtNovaConfig,
    should_materialize: bool,
    label: &str,
) -> Result<()> {
    if config.storage_read_only && should_materialize {
        return Err(DbtNovaError::ServerError(format!(
            "Storage is read-only; cannot materialize {label} on first-run hydration. Supported flows: either unset DBT_NOVA_STORAGE_READ_ONLY and use DBT_NOVA_ARTIFACT_FETCH_POLICY=if_missing|always to hydrate locally, or keep DBT_NOVA_STORAGE_READ_ONLY=true with DBT_NOVA_ARTIFACT_FETCH_POLICY=never after assets are already materialized locally."
        )));
    }
    Ok(())
}

fn should_materialize(label: &str, policy: ArtifactFetchPolicy, is_present: bool) -> Result<bool> {
    match policy {
        ArtifactFetchPolicy::Always => Ok(true),
        ArtifactFetchPolicy::IfMissing => Ok(!is_present),
        ArtifactFetchPolicy::Never => {
            if is_present {
                Ok(false)
            } else {
                Err(DbtNovaError::ServerError(format!(
                    "artifact fetch policy is 'never' but no {label} are present locally"
                )))
            }
        }
    }
}

fn validate_metadata_against_runtime(
    metadata: &PrebuiltAssetsMetadata,
    config: &DbtNovaConfig,
    expected_manifest_hash: &str,
) -> Result<()> {
    let instance_id = config.storage_instance_id.trim();
    info!(
        contract_version = %metadata.contract_version,
        metadata_storage_instance_id = %metadata.storage_instance_id,
        runtime_storage_instance_id = %instance_id,
        metadata_manifest_hash = %metadata.manifest_hash,
        runtime_manifest_hash = %expected_manifest_hash,
        has_models_artifact = metadata.has_models_artifact(),
        "validating prebuilt artifact metadata contract"
    );
    if metadata.storage_instance_id.trim() != instance_id {
        warn!(
            metadata_storage_instance_id = %metadata.storage_instance_id,
            runtime_storage_instance_id = %instance_id,
            "prebuilt artifact metadata contract validation failed: storage_instance_id mismatch"
        );
        return Err(DbtNovaError::ServerError(format!(
            "storage_instance_id mismatch: metadata='{}' runtime='{}'",
            metadata.storage_instance_id, instance_id
        )));
    }
    if metadata.manifest_hash.trim() != expected_manifest_hash.trim() {
        warn!(
            metadata_manifest_hash = %metadata.manifest_hash,
            runtime_manifest_hash = %expected_manifest_hash,
            "prebuilt artifact metadata contract validation failed: manifest hash mismatch"
        );
        return Err(DbtNovaError::ServerError(format!(
            "manifest hash mismatch: metadata='{}' runtime='{}'",
            metadata.manifest_hash, expected_manifest_hash
        )));
    }
    info!(
        storage_instance_id = %metadata.storage_instance_id,
        manifest_hash = %metadata.manifest_hash,
        "prebuilt artifact metadata contract validated"
    );
    Ok(())
}

fn storage_instance_present(config: &DbtNovaConfig) -> Result<bool> {
    let versions_root = config.storage_instance_root_dir()?.join("versions");
    if !versions_root.is_dir() {
        return Ok(false);
    }
    for entry in fs::read_dir(&versions_root)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            return Ok(true);
        }
    }
    Ok(false)
}

fn directory_has_files(path: &Path) -> Result<bool> {
    if !path.is_dir() {
        return Ok(false);
    }
    let mut stack = vec![path.to_path_buf()];
    while let Some(next) = stack.pop() {
        for entry in fs::read_dir(&next)? {
            let entry = entry?;
            let ty = entry.file_type()?;
            if ty.is_file() {
                return Ok(true);
            }
            if ty.is_dir() {
                stack.push(entry.path());
            }
        }
    }
    Ok(false)
}

fn inspect_models_cache_presence(
    config: &DbtNovaConfig,
    cache_dir: &Path,
    expected_manifest_hash: &str,
) -> Result<ModelsCachePresence> {
    if !cache_dir.is_dir() || !directory_has_files(cache_dir)? {
        return Ok(ModelsCachePresence::Missing);
    }

    match validate_models_cache_layout(config, cache_dir, expected_manifest_hash) {
        Ok(()) => Ok(ModelsCachePresence::Valid),
        Err(err) => Ok(ModelsCachePresence::Invalid(err.to_string())),
    }
}

fn should_materialize_models(
    policy: ArtifactFetchPolicy,
    models_target: &Path,
    presence: &ModelsCachePresence,
) -> Result<bool> {
    match policy {
        ArtifactFetchPolicy::Always => Ok(true),
        ArtifactFetchPolicy::IfMissing => match presence {
            ModelsCachePresence::Missing => Ok(true),
            ModelsCachePresence::Valid => Ok(false),
            ModelsCachePresence::Invalid(reason) => {
                warn!(
                    target = %models_target.display(),
                    reason,
                    "local models cache is incomplete or invalid for the requested manifest; re-materializing models artifact"
                );
                Ok(true)
            }
        },
        ArtifactFetchPolicy::Never => match presence {
            ModelsCachePresence::Valid => Ok(false),
            ModelsCachePresence::Missing => Err(DbtNovaError::ServerError(
                "artifact fetch policy is 'never' but no valid models artifacts are present locally"
                    .to_string(),
            )),
            ModelsCachePresence::Invalid(reason) => Err(DbtNovaError::ServerError(format!(
                "artifact fetch policy is 'never' but local models artifacts are incomplete or invalid for the requested manifest: {reason}"
            ))),
        },
    }
}

pub(crate) fn read_small_text_file(path: &Path, max_bytes: u64) -> Result<String> {
    let mut file = File::open(path)?;
    let metadata = file.metadata()?;
    if max_bytes > 0 && metadata.len() > max_bytes {
        return Err(DbtNovaError::ServerError(format!(
            "artifact metadata exceeds configured size limit ({} > {})",
            metadata.len(),
            max_bytes
        )));
    }

    let mut raw = String::new();
    file.read_to_string(&mut raw)?;
    Ok(raw)
}

fn resolve_file_artifact_uri(name: &str, uri: &str) -> Result<PathBuf> {
    let trimmed = uri.trim();
    if trimmed.is_empty() {
        return Err(DbtNovaError::InvalidParams(format!(
            "{name} cannot be empty"
        )));
    }
    let lower = trimmed.to_ascii_lowercase();

    if let Some(rest) = lower.strip_prefix("file://") {
        let original_rest = &trimmed[(trimmed.len() - rest.len())..];
        if rest.starts_with("localhost/") {
            let host_path = &original_rest["localhost/".len()..];
            let normalized = format!("/{}", host_path.trim_start_matches('/'));
            return Ok(PathBuf::from(normalized));
        }
        if original_rest.starts_with('/') {
            return Ok(PathBuf::from(original_rest));
        }
        return Err(DbtNovaError::InvalidParams(format!(
            "{name} must use an absolute file:// URI"
        )));
    }

    if lower.contains("://") {
        return Err(DbtNovaError::ServerError(format!(
            "{name} uses a non-file URI scheme; file resolver currently supports only file:// or plain local paths"
        )));
    }

    Ok(PathBuf::from(trimmed))
}

pub(crate) fn ensure_regular_file(name: &str, path: &Path) -> Result<()> {
    if !path.exists() {
        return Err(DbtNovaError::ServerError(format!(
            "{name} does not exist: {}",
            path.display()
        )));
    }
    if !path.is_file() {
        return Err(DbtNovaError::ServerError(format!(
            "{name} must point to a file: {}",
            path.display()
        )));
    }
    Ok(())
}

fn extract_archive_atomically(archive_path: &Path, target_dir: &Path) -> Result<()> {
    extract_archive_atomically_with_validation(archive_path, target_dir, |_| Ok(()))
}

fn extract_archive_atomically_with_validation<F>(
    archive_path: &Path,
    target_dir: &Path,
    validate: F,
) -> Result<()>
where
    F: FnOnce(&Path) -> Result<()>,
{
    let parent = target_dir.parent().ok_or_else(|| {
        DbtNovaError::ServerError(format!(
            "cannot materialize archive; target has no parent: {}",
            target_dir.display()
        ))
    })?;
    fs::create_dir_all(parent)?;

    let stage_root = parent.join(format!(".nova-artifacts-stage-{}", unique_suffix()));
    fs::create_dir_all(&stage_root)?;

    let extracted_root = stage_root.join("extract-root");
    fs::create_dir_all(&extracted_root)?;

    extract_tar_gz(archive_path, &extracted_root)?;
    let payload_root = single_directory_child(&extracted_root)?;
    validate(&payload_root)?;
    swap_directory_atomically(&payload_root, target_dir)?;

    let _ = fs::remove_dir_all(&stage_root);
    Ok(())
}

fn extract_tar_gz(archive_path: &Path, output_root: &Path) -> Result<()> {
    let file = File::open(archive_path)?;
    let decoder = GzDecoder::new(file);
    let mut archive = Archive::new(decoder);

    for entry_result in archive.entries()? {
        let mut entry = entry_result?;
        let entry_path = entry.path()?.into_owned();
        validate_archive_entry_path(&entry_path)?;
        let entry_type = entry.header().entry_type();
        if !(entry_type.is_dir() || entry_type.is_file()) {
            return Err(DbtNovaError::ServerError(format!(
                "archive contains unsupported entry type for path: {}",
                entry_path.display()
            )));
        }

        let out_path = output_root.join(&entry_path);
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)?;
        }
        entry.unpack(&out_path)?;
    }
    Ok(())
}

fn validate_archive_entry_path(path: &Path) -> Result<()> {
    if path.is_absolute() {
        return Err(DbtNovaError::ServerError(format!(
            "archive contains absolute path entry: {}",
            path.display()
        )));
    }
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(DbtNovaError::ServerError(format!(
            "archive contains parent traversal entry: {}",
            path.display()
        )));
    }
    Ok(())
}

fn single_directory_child(root: &Path) -> Result<PathBuf> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        entries.push(entry);
    }
    if entries.len() != 1 {
        return Err(DbtNovaError::ServerError(format!(
            "archive must contain exactly one top-level directory entry; found {}",
            entries.len()
        )));
    }
    let entry = entries.remove(0);
    if !entry.file_type()?.is_dir() {
        return Err(DbtNovaError::ServerError(
            "archive top-level entry must be a directory".to_string(),
        ));
    }
    Ok(entry.path())
}

fn validate_models_cache_layout(
    config: &DbtNovaConfig,
    cache_dir: &Path,
    expected_manifest_hash: &str,
) -> Result<()> {
    reject_legacy_semantic_cache_files(cache_dir)?;

    let required_models = required_model_layouts(config);
    if required_models.is_empty()
        && !config.search.enable_vector_search
        && !config.search.enable_sparse_search
    {
        return Ok(());
    }

    for required_model in &required_models {
        validate_required_model_repo_layout(cache_dir, required_model)?;
    }

    validate_manifest_scoped_semantic_caches(config, cache_dir, expected_manifest_hash)?;
    Ok(())
}

fn required_model_layouts(config: &DbtNovaConfig) -> Vec<RequiredModelLayout> {
    let mut required = Vec::new();
    if config.search.enable_vector_search {
        required.push(resolve_embedding_model_layout(
            &config.search.embedding_model,
        ));
    }
    if config.search.enable_sparse_search {
        required.push(resolve_sparse_model_layout());
    }
    if config.search.enable_reranker {
        required.push(resolve_reranker_model_layout(&config.search.reranker_model));
    }
    required
}

#[cfg(feature = "embeddings")]
fn required_model_layout(layout: RuntimeRequiredModelLayout) -> RequiredModelLayout {
    RequiredModelLayout {
        component: layout.component,
        model_code: layout.model_code,
        model_file: layout.model_file,
        additional_files: layout.additional_files,
    }
}

#[cfg(feature = "embeddings")]
fn resolve_embedding_model_layout(value: &str) -> RequiredModelLayout {
    required_model_layout(required_embedding_model_layout(value))
}

#[cfg(not(feature = "embeddings"))]
fn resolve_embedding_model_layout(value: &str) -> RequiredModelLayout {
    let normalized = value.trim().to_lowercase();
    match normalized.as_str() {
        "multilingual-e5-large" | "intfloat/multilingual-e5-large" | "e5-large" => {
            RequiredModelLayout {
                component: "vector",
                model_code: "intfloat/multilingual-e5-large".to_string(),
                model_file: "onnx/model.onnx".to_string(),
                additional_files: vec!["model.onnx_data"],
            }
        }
        "multilingual-e5-small" | "intfloat/multilingual-e5-small" | "e5-small" => {
            RequiredModelLayout {
                component: "vector",
                model_code: "intfloat/multilingual-e5-small".to_string(),
                model_file: "onnx/model.onnx".to_string(),
                additional_files: Vec::new(),
            }
        }
        "bge-small-en-v1.5" | "baai/bge-small-en-v1.5" | "bge-small" => RequiredModelLayout {
            component: "vector",
            model_code: "BAAI/bge-small-en-v1.5".to_string(),
            model_file: "onnx/model.onnx".to_string(),
            additional_files: Vec::new(),
        },
        "bge-base-en-v1.5" | "baai/bge-base-en-v1.5" | "bge-base" => RequiredModelLayout {
            component: "vector",
            model_code: "Xenova/bge-base-en-v1.5".to_string(),
            model_file: "onnx/model.onnx".to_string(),
            additional_files: Vec::new(),
        },
        "bge-large-en-v1.5" | "baai/bge-large-en-v1.5" | "bge-large" => RequiredModelLayout {
            component: "vector",
            model_code: "Xenova/bge-large-en-v1.5".to_string(),
            model_file: "onnx/model.onnx".to_string(),
            additional_files: Vec::new(),
        },
        "all-minilm-l6-v2" | "allminilm-l6-v2" | "minilm-l6" | "minilm" => RequiredModelLayout {
            component: "vector",
            model_code: "Qdrant/all-MiniLM-L6-v2-onnx".to_string(),
            model_file: "model.onnx".to_string(),
            additional_files: Vec::new(),
        },
        "all-minilm-l12-v2" | "allminilm-l12-v2" | "minilm-l12" => RequiredModelLayout {
            component: "vector",
            model_code: "Xenova/all-MiniLM-L12-v2".to_string(),
            model_file: "onnx/model.onnx".to_string(),
            additional_files: Vec::new(),
        },
        "nomic-embed-text-v1" | "nomic-ai/nomic-embed-text-v1" | "nomic-v1" => {
            RequiredModelLayout {
                component: "vector",
                model_code: "nomic-ai/nomic-embed-text-v1".to_string(),
                model_file: "onnx/model.onnx".to_string(),
                additional_files: Vec::new(),
            }
        }
        "nomic-embed-text-v1.5" | "nomic-ai/nomic-embed-text-v1.5" | "nomic-v1.5" | "nomic" => {
            RequiredModelLayout {
                component: "vector",
                model_code: "nomic-ai/nomic-embed-text-v1.5".to_string(),
                model_file: "onnx/model.onnx".to_string(),
                additional_files: Vec::new(),
            }
        }
        "mxbai-embed-large-v1" | "mixedbread-ai/mxbai-embed-large-v1" | "mxbai-large" => {
            RequiredModelLayout {
                component: "vector",
                model_code: "mixedbread-ai/mxbai-embed-large-v1".to_string(),
                model_file: "onnx/model.onnx".to_string(),
                additional_files: Vec::new(),
            }
        }
        "gte-base-en-v1.5" | "alibaba-nlp/gte-base-en-v1.5" | "gte-base" => RequiredModelLayout {
            component: "vector",
            model_code: "Alibaba-NLP/gte-base-en-v1.5".to_string(),
            model_file: "onnx/model.onnx".to_string(),
            additional_files: Vec::new(),
        },
        "gte-large-en-v1.5" | "alibaba-nlp/gte-large-en-v1.5" | "gte-large" => {
            RequiredModelLayout {
                component: "vector",
                model_code: "Alibaba-NLP/gte-large-en-v1.5".to_string(),
                model_file: "onnx/model.onnx".to_string(),
                additional_files: Vec::new(),
            }
        }
        _ => RequiredModelLayout {
            component: "vector",
            model_code: SearchConfig::default().embedding_model,
            model_file: "onnx/model.onnx".to_string(),
            additional_files: Vec::new(),
        },
    }
}

#[cfg(feature = "embeddings")]
fn resolve_sparse_model_layout() -> RequiredModelLayout {
    required_model_layout(required_sparse_model_layout())
}

#[cfg(not(feature = "embeddings"))]
fn resolve_sparse_model_layout() -> RequiredModelLayout {
    RequiredModelLayout {
        component: "sparse",
        model_code: default_sparse_model_name().to_string(),
        model_file: "model.onnx".to_string(),
        additional_files: Vec::new(),
    }
}

#[cfg(feature = "embeddings")]
fn resolve_reranker_model_layout(value: &str) -> RequiredModelLayout {
    required_model_layout(required_reranker_model_layout(value))
}

#[cfg(not(feature = "embeddings"))]
fn resolve_reranker_model_layout(value: &str) -> RequiredModelLayout {
    let normalized = value.trim().to_lowercase();
    match normalized.as_str() {
        "jina-reranker-v1-turbo-en"
        | "jinaai/jina-reranker-v1-turbo-en"
        | "jina-v1-turbo-en"
        | "jina-turbo"
        | "turbo" => RequiredModelLayout {
            component: "reranker",
            model_code: "jinaai/jina-reranker-v1-turbo-en".to_string(),
            model_file: "onnx/model.onnx".to_string(),
            additional_files: Vec::new(),
        },
        "bge-reranker-base" | "baai/bge-reranker-base" | "bge-base" | "bge" => {
            RequiredModelLayout {
                component: "reranker",
                model_code: "BAAI/bge-reranker-base".to_string(),
                model_file: "onnx/model.onnx".to_string(),
                additional_files: Vec::new(),
            }
        }
        _ => RequiredModelLayout {
            component: "reranker",
            model_code: "jinaai/jina-reranker-v2-base-multilingual".to_string(),
            model_file: "onnx/model.onnx".to_string(),
            additional_files: Vec::new(),
        },
    }
}

fn configured_embedding_model_name(search: &SearchConfig) -> String {
    let trimmed = search.embedding_model.trim();
    if trimmed.is_empty() {
        SearchConfig::default().embedding_model
    } else {
        trimmed.to_string()
    }
}

fn reject_legacy_semantic_cache_files(cache_dir: &Path) -> Result<()> {
    for component in [
        SemanticCacheComponent::Dense,
        SemanticCacheComponent::Sparse,
    ] {
        let legacy_zst = cache_dir.join(component.legacy_file_zst());
        if legacy_zst.is_file() {
            return Err(DbtNovaError::ServerError(format!(
                "models archive contains legacy singleton semantic cache file {}; publish manifest-scoped caches under manifests/<manifest_hash>/ instead",
                legacy_zst.display()
            )));
        }
        let legacy_raw = cache_dir.join(component.legacy_file_raw());
        if legacy_raw.is_file() {
            return Err(DbtNovaError::ServerError(format!(
                "models archive contains legacy singleton semantic cache file {}; publish manifest-scoped caches under manifests/<manifest_hash>/ instead",
                legacy_raw.display()
            )));
        }
    }
    Ok(())
}

fn validate_manifest_scoped_semantic_caches(
    config: &DbtNovaConfig,
    cache_dir: &Path,
    expected_manifest_hash: &str,
) -> Result<()> {
    let require_dense = config.search.enable_vector_search;
    let require_sparse = config.search.enable_sparse_search;
    if !require_dense && !require_sparse {
        return Ok(());
    }

    let manifests_dir = cache_dir.join("manifests");
    if !manifests_dir.is_dir() {
        return Err(DbtNovaError::ServerError(format!(
            "models archive is missing manifests directory for semantic caches in {}",
            cache_dir.display()
        )));
    }

    let scoped_cache_dir = manifests_dir.join(expected_manifest_hash);
    if !scoped_cache_dir.is_dir() {
        return Err(DbtNovaError::ServerError(format!(
            "models archive is missing manifest-scoped semantic caches for manifest hash {expected_manifest_hash}",
        )));
    }

    if require_dense {
        ensure_semantic_cache_file(
            &scoped_cache_dir,
            SemanticCacheComponent::Dense,
            &configured_embedding_model_name(&config.search),
        )?;
    }
    if require_sparse {
        ensure_semantic_cache_file(
            &scoped_cache_dir,
            SemanticCacheComponent::Sparse,
            default_sparse_model_name(),
        )?;
    }
    Ok(())
}

fn ensure_semantic_cache_file(
    manifest_dir: &Path,
    component: SemanticCacheComponent,
    model_name: &str,
) -> Result<()> {
    let file_stem = format!(
        "{}__{}",
        component.file_prefix(),
        semantic_cache::model_slug(model_name)
    );
    let compressed_path = manifest_dir.join(format!("{file_stem}.rkyv.zst"));
    let raw_path = manifest_dir.join(format!("{file_stem}.rkyv"));
    if compressed_path.is_file() || raw_path.is_file() {
        return Ok(());
    }

    Err(DbtNovaError::ServerError(format!(
        "models archive is missing {} semantic cache file for model '{}' in {}; expected {} or {}",
        component.name(),
        model_name,
        manifest_dir.display(),
        compressed_path.display(),
        raw_path.display()
    )))
}

fn validate_required_model_repo_layout(
    cache_dir: &Path,
    required_model: &RequiredModelLayout,
) -> Result<()> {
    let repo_dir = cache_dir.join(model_repo_dir_name(&required_model.model_code));
    validate_model_repo_layout(&repo_dir)?;
    let snapshot_dir = resolve_snapshot_dir(&repo_dir)?;

    for relative_path in required_model_files(required_model) {
        let path = snapshot_dir.join(relative_path);
        if !path.is_file() {
            return Err(DbtNovaError::ServerError(format!(
                "models archive is missing required {} model file {} for {}",
                required_model.component,
                path.display(),
                required_model.model_code
            )));
        }
    }

    Ok(())
}

fn model_repo_dir_name(model_code: &str) -> String {
    format!("models--{}", model_code.replace('/', "--"))
}

fn resolve_snapshot_dir(repo_dir: &Path) -> Result<PathBuf> {
    let snapshots_dir = repo_dir.join("snapshots");
    let refs_main = repo_dir.join("refs").join("main");
    if refs_main.is_file() {
        let revision = fs::read_to_string(&refs_main)?;
        let revision = revision.trim();
        if !revision.is_empty() {
            let snapshot_path = snapshots_dir.join(revision);
            if snapshot_path.is_dir() {
                return Ok(snapshot_path);
            }
        }
    }

    let main_snapshot = snapshots_dir.join("main");
    if main_snapshot.is_dir() {
        return Ok(main_snapshot);
    }

    let mut snapshot_dirs = fs::read_dir(&snapshots_dir)?
        .filter_map(std::result::Result::ok)
        .filter_map(|entry| entry.file_type().ok().map(|file_type| (entry, file_type)))
        .filter(|(_, file_type)| file_type.is_dir())
        .map(|(entry, _)| entry.path())
        .collect::<Vec<_>>();
    if snapshot_dirs.len() == 1 {
        return Ok(snapshot_dirs.remove(0));
    }

    Err(DbtNovaError::ServerError(format!(
        "unable to resolve model snapshot directory in {}",
        repo_dir.display()
    )))
}

fn required_model_files(required_model: &RequiredModelLayout) -> Vec<&str> {
    let mut required = vec![
        required_model.model_file.as_str(),
        "tokenizer.json",
        "config.json",
        "special_tokens_map.json",
        "tokenizer_config.json",
    ];
    required.extend(required_model.additional_files.iter().copied());
    required
}

fn validate_model_repo_layout(repo_dir: &Path) -> Result<()> {
    let refs_dir = repo_dir.join("refs");
    if !refs_dir.is_dir() {
        return Err(DbtNovaError::ServerError(format!(
            "models archive is missing refs directory for {}",
            repo_dir.display()
        )));
    }

    let snapshots_dir = repo_dir.join("snapshots");
    if !snapshots_dir.is_dir() {
        return Err(DbtNovaError::ServerError(format!(
            "models archive is missing snapshots directory for {}",
            repo_dir.display()
        )));
    }

    let mut saw_ref = false;
    for entry in fs::read_dir(&refs_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        saw_ref = true;

        let ref_name = entry.file_name().to_string_lossy().to_string();
        let commit_hash = fs::read_to_string(entry.path())?;
        let commit_hash = commit_hash.trim();
        if commit_hash.is_empty() {
            return Err(DbtNovaError::ServerError(format!(
                "models archive ref {ref_name} is empty for {}",
                repo_dir.display()
            )));
        }

        let snapshot_path = snapshots_dir.join(commit_hash);
        if !snapshot_path.is_dir() {
            return Err(DbtNovaError::ServerError(format!(
                "models archive ref {ref_name} points to missing snapshot '{commit_hash}' for {}",
                repo_dir.display()
            )));
        }
        if !directory_has_files(&snapshot_path)? {
            return Err(DbtNovaError::ServerError(format!(
                "models archive snapshot '{commit_hash}' contains no files for {}",
                repo_dir.display()
            )));
        }
    }

    if !saw_ref {
        return Err(DbtNovaError::ServerError(format!(
            "models archive contains no refs for {}",
            repo_dir.display()
        )));
    }

    Ok(())
}

fn swap_directory_atomically(source_dir: &Path, target_dir: &Path) -> Result<()> {
    let mut backup_dir = None;
    if target_dir.exists() {
        let file_name = target_dir
            .file_name()
            .unwrap_or_else(|| OsStr::new("target"))
            .to_string_lossy()
            .to_string();
        let backup = target_dir.with_file_name(format!("{file_name}.backup-{}", unique_suffix()));
        info!(
            source = %source_dir.display(),
            target = %target_dir.display(),
            backup = %backup.display(),
            "swapping directory atomically via backup rotation"
        );
        fs::rename(target_dir, &backup)?;
        backup_dir = Some(backup);
    } else {
        info!(
            source = %source_dir.display(),
            target = %target_dir.display(),
            "materializing directory atomically into empty target"
        );
    }

    if let Err(err) = fs::rename(source_dir, target_dir) {
        if let Some(backup) = backup_dir.as_ref() {
            let _ = fs::rename(backup, target_dir);
        }
        return Err(DbtNovaError::ServerError(format!(
            "failed to materialize archive into {}: {err}",
            target_dir.display()
        )));
    }

    if let Some(backup) = backup_dir {
        info!(
            backup = %backup.display(),
            "removing backup directory after successful swap"
        );
        let _ = fs::remove_dir_all(backup);
    }
    Ok(())
}
