use dbt_nova::config::{ArtifactFetchPolicy, DbtNovaConfig};
use dbt_nova::manifest::prebuilt_assets_resolver::materialize_file_artifacts;
use flate2::Compression;
use flate2::write::GzEncoder;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use tar::Builder;
use tempfile::TempDir;

static ENV_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();

fn lock_env() -> std::sync::MutexGuard<'static, ()> {
    ENV_MUTEX
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

struct EnvVarRestore {
    key: &'static str,
    previous: Option<String>,
}

impl EnvVarRestore {
    fn remove(key: &'static str) -> Self {
        let previous = std::env::var(key).ok();
        // SAFETY: tests serialize environment mutation with `ENV_MUTEX`.
        unsafe { std::env::remove_var(key) };
        Self { key, previous }
    }
}

impl Drop for EnvVarRestore {
    fn drop(&mut self) {
        if let Some(value) = self.previous.take() {
            // SAFETY: tests serialize environment mutation with `ENV_MUTEX`.
            unsafe { std::env::set_var(self.key, value) };
        } else {
            // SAFETY: tests serialize environment mutation with `ENV_MUTEX`.
            unsafe { std::env::remove_var(self.key) };
        }
    }
}

fn create_archive_from_dir(source_dir: &Path, archive_path: &Path) {
    let file = File::create(archive_path).expect("create archive file");
    let encoder = GzEncoder::new(file, Compression::default());
    let mut tar = Builder::new(encoder);
    let root_name = source_dir
        .file_name()
        .expect("source root name")
        .to_string_lossy()
        .to_string();
    tar.append_dir_all(root_name, source_dir)
        .expect("append dir to archive");
    let encoder = tar.into_inner().expect("finish tar stream");
    encoder.finish().expect("finish gzip stream");
}

fn write_file(path: &Path, content: &[u8]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent dirs");
    }
    let mut file = File::create(path).expect("create file");
    file.write_all(content).expect("write file");
}

fn to_file_uri(path: &Path) -> String {
    format!("file://{}", path.display())
}

fn write_cached_artifact(cache_dir: &Path, uri: &str, source: &Path) {
    fs::create_dir_all(cache_dir).expect("create artifacts cache dir");
    let hash = blake3::hash(uri.as_bytes()).to_hex().to_string();
    let cache_path = cache_dir.join(format!("{hash}.json"));
    fs::copy(source, cache_path).expect("copy cached artifact");
}

fn artifact_stage_dirs(root: &Path) -> Vec<PathBuf> {
    fs::read_dir(root)
        .expect("read stage parent")
        .filter_map(|entry| {
            let entry = entry.expect("stage dir entry");
            let name = entry.file_name();
            name.to_string_lossy()
                .starts_with(".nova-artifacts-stage-")
                .then(|| entry.path())
        })
        .collect()
}

fn setup_config(workspace: &TempDir) -> DbtNovaConfig {
    let manifest_path = workspace.path().join("nova_manifest.json");
    write_file(&manifest_path, br#"{"metadata":{"dbt_version":"1.8.0"}}"#);

    let mut config = DbtNovaConfig {
        manifest_path: manifest_path.to_string_lossy().to_string(),
        storage_dir: workspace
            .path()
            .join(".dbt-nova")
            .to_string_lossy()
            .to_string(),
        storage_instance_id: "analytics-prod".to_string(),
        ..DbtNovaConfig::default()
    };
    config.search.enable_vector_search = true;
    config.search.enable_sparse_search = true;
    config.search.enable_reranker = true;
    config
}

fn write_standard_metadata(workspace: &TempDir, artifact_name_models: &str) -> PathBuf {
    let metadata_path = workspace.path().join("nova-build-metadata.json");
    write_file(
        &metadata_path,
        format!(
            r#"{{
  "contract_version":"v1",
  "manifest_hash":"manifest-hash",
  "manifest_version":"v12",
  "entity_count":42,
  "storage_instance_id":"analytics-prod",
  "dbt_nova_version":"0.0.2",
  "build_timestamp":"2026-03-02T00:00:00Z",
  "artifact_name_storage":"storage-asset",
  "artifact_name_models":"{artifact_name_models}"
}}"#
        )
        .as_bytes(),
    );
    metadata_path
}

fn create_storage_source(workspace: &TempDir) -> PathBuf {
    let storage_source = workspace.path().join("storage-source");
    let version_dir = storage_source
        .join("instances")
        .join("analytics-prod")
        .join("versions")
        .join("abc123");
    write_file(&version_dir.join("entities.bin"), b"entities");
    write_file(&version_dir.join("entities.idx"), b"{}");
    write_file(&version_dir.join("entities.checksum.json"), b"{}");
    write_file(&version_dir.join("manifest.signature.json"), b"{}");
    storage_source
}

#[test]
fn materialize_file_artifacts_rejects_oversized_compressed_archive() {
    let workspace = TempDir::new().expect("tempdir");
    let mut config = setup_config(&workspace);

    let storage_source = create_storage_source(&workspace);
    let storage_archive = workspace.path().join("storage.tar.gz");
    create_archive_from_dir(&storage_source, &storage_archive);
    let metadata_path = write_standard_metadata(&workspace, "");

    config.storage_artifact_uri = to_file_uri(&storage_archive);
    config.metadata_artifact_uri = to_file_uri(&metadata_path);
    config.artifact_fetch_policy = ArtifactFetchPolicy::Always;
    config.artifact_max_bytes = 16;

    let err = materialize_file_artifacts(&config, "manifest-hash")
        .expect_err("oversized compressed archive should be rejected");
    assert!(err.to_string().contains("artifact size limit"));
}

#[test]
fn materialize_file_artifacts_rejects_archive_entry_count_over_limit() {
    let workspace = TempDir::new().expect("tempdir");
    let mut config = setup_config(&workspace);

    let storage_source = create_storage_source(&workspace);
    let storage_archive = workspace.path().join("storage.tar.gz");
    create_archive_from_dir(&storage_source, &storage_archive);
    let metadata_path = write_standard_metadata(&workspace, "");

    config.storage_artifact_uri = to_file_uri(&storage_archive);
    config.metadata_artifact_uri = to_file_uri(&metadata_path);
    config.artifact_fetch_policy = ArtifactFetchPolicy::Always;
    config.artifact_archive_max_entries = 1;

    let err = materialize_file_artifacts(&config, "manifest-hash")
        .expect_err("archive with too many entries should be rejected");
    assert!(err.to_string().contains("too many entries"));
    assert!(artifact_stage_dirs(workspace.path()).is_empty());
}

#[test]
fn materialize_file_artifacts_rejects_decompressed_archive_over_limit() {
    let workspace = TempDir::new().expect("tempdir");
    let mut config = setup_config(&workspace);

    let storage_source = create_storage_source(&workspace);
    let storage_archive = workspace.path().join("storage.tar.gz");
    create_archive_from_dir(&storage_source, &storage_archive);
    let metadata_path = write_standard_metadata(&workspace, "");

    config.storage_artifact_uri = to_file_uri(&storage_archive);
    config.metadata_artifact_uri = to_file_uri(&metadata_path);
    config.artifact_fetch_policy = ArtifactFetchPolicy::Always;
    config.artifact_archive_max_uncompressed_bytes = 5;

    let err = materialize_file_artifacts(&config, "manifest-hash")
        .expect_err("oversized decompressed archive should be rejected");
    assert!(err.to_string().contains("decompressed size limit"));
    assert!(artifact_stage_dirs(workspace.path()).is_empty());
}

fn write_manifest_scoped_semantic_caches(root: &Path, manifest_hash: &str) {
    write_file(
        &root
            .join("manifests")
            .join(manifest_hash)
            .join("dense__intfloat--multilingual-e5-base.rkyv.zst"),
        b"dense-cache",
    );
    write_file(
        &root
            .join("manifests")
            .join(manifest_hash)
            .join("sparse__Qdrant--Splade_PP_en_v1.rkyv.zst"),
        b"sparse-cache",
    );
}

fn write_hf_model_repo(
    root: &Path,
    model_code: &str,
    commit_hash: &str,
    model_file: &str,
    additional_files: &[&str],
) {
    let repo_root = root.join(format!("models--{}", model_code.replace('/', "--")));
    write_file(&repo_root.join("refs").join("main"), commit_hash.as_bytes());

    let snapshot_root = repo_root.join("snapshots").join(commit_hash);
    write_file(&snapshot_root.join(model_file), b"model");
    write_file(&snapshot_root.join("tokenizer.json"), b"{}");
    write_file(&snapshot_root.join("config.json"), b"{}");
    write_file(&snapshot_root.join("special_tokens_map.json"), b"{}");
    write_file(
        &snapshot_root.join("tokenizer_config.json"),
        br#"{"model_max_length":512,"pad_token":"[PAD]"}"#,
    );
    for additional_file in additional_files {
        write_file(&snapshot_root.join(additional_file), b"extra");
    }
}

fn write_complete_models_cache(root: &Path, manifest_hash: &str) {
    let commit_hash = "abc123";
    write_hf_model_repo(
        root,
        "intfloat/multilingual-e5-base",
        commit_hash,
        "onnx/model.onnx",
        &[],
    );
    write_hf_model_repo(
        root,
        "Qdrant/Splade_PP_en_v1",
        commit_hash,
        "model.onnx",
        &[],
    );
    write_hf_model_repo(
        root,
        "jinaai/jina-reranker-v2-base-multilingual",
        commit_hash,
        "onnx/model.onnx",
        &[],
    );
    write_manifest_scoped_semantic_caches(root, manifest_hash);
}

fn create_models_source(workspace: &TempDir, manifest_hash: &str) -> PathBuf {
    let models_source = workspace.path().join("models-source");
    write_complete_models_cache(&models_source, manifest_hash);
    models_source
}

fn create_invalid_models_source_missing_refs(workspace: &TempDir) -> PathBuf {
    let models_source = workspace.path().join("models-source-invalid");
    let commit_hash = "abc123";
    write_hf_model_repo(
        &models_source,
        "Qdrant/Splade_PP_en_v1",
        commit_hash,
        "model.onnx",
        &[],
    );
    write_hf_model_repo(
        &models_source,
        "jinaai/jina-reranker-v2-base-multilingual",
        commit_hash,
        "onnx/model.onnx",
        &[],
    );
    let model_path = models_source
        .join("models--intfloat--multilingual-e5-base")
        .join("snapshots")
        .join(commit_hash)
        .join("onnx")
        .join("model.onnx");
    write_file(&model_path, b"model");
    models_source
}

fn create_models_source_with_legacy_semantic_caches(
    workspace: &TempDir,
    manifest_hash: &str,
) -> PathBuf {
    let models_source = create_models_source(workspace, manifest_hash);
    write_file(&models_source.join("embeddings.rkyv.zst"), b"legacy-dense");
    models_source
}

#[test]
fn materialize_file_artifacts_happy_path() {
    let workspace = TempDir::new().expect("tempdir");
    let mut config = setup_config(&workspace);
    config.search.embedding_cache_dir = workspace
        .path()
        .join("embedding-cache")
        .to_string_lossy()
        .to_string();

    let storage_source = create_storage_source(&workspace);
    let storage_archive = workspace.path().join("storage.tar.gz");
    create_archive_from_dir(&storage_source, &storage_archive);

    let models_source = create_models_source(&workspace, "manifest-hash");
    let models_archive = workspace.path().join("models.tar.gz");
    create_archive_from_dir(&models_source, &models_archive);

    let metadata_path = workspace.path().join("nova-build-metadata.json");
    write_file(
        &metadata_path,
        br#"{
  "contract_version":"v1",
  "manifest_hash":"manifest-hash",
  "manifest_version":"v12",
  "entity_count":42,
  "storage_instance_id":"analytics-prod",
  "dbt_nova_version":"0.0.2",
  "build_timestamp":"2026-03-02T00:00:00Z",
  "artifact_name_storage":"storage-asset",
  "artifact_name_models":"models-asset"
}"#,
    );

    config.storage_artifact_uri = to_file_uri(&storage_archive);
    config.metadata_artifact_uri = to_file_uri(&metadata_path);
    config.models_artifact_uri = to_file_uri(&models_archive);
    config.artifact_fetch_policy = ArtifactFetchPolicy::IfMissing;

    let outcome = materialize_file_artifacts(&config, "manifest-hash")
        .expect("materialization should succeed")
        .expect("artifact mode enabled");

    assert!(outcome.storage_materialized);
    assert!(outcome.models_materialized);
    assert_eq!(outcome.metadata.storage_instance_id, "analytics-prod");

    let storage_target = PathBuf::from(&config.storage_dir);
    assert!(
        storage_target
            .join("instances")
            .join("analytics-prod")
            .join("versions")
            .join("abc123")
            .join("entities.bin")
            .exists()
    );
    assert!(
        PathBuf::from(&config.search.embedding_cache_dir)
            .join("models--intfloat--multilingual-e5-base")
            .join("refs")
            .join("main")
            .exists()
    );
    assert!(
        PathBuf::from(&config.search.embedding_cache_dir)
            .join("models--intfloat--multilingual-e5-base")
            .join("snapshots")
            .join("abc123")
            .join("onnx")
            .join("model.onnx")
            .exists()
    );
    assert!(
        PathBuf::from(&config.search.embedding_cache_dir)
            .join("models--Qdrant--Splade_PP_en_v1")
            .join("snapshots")
            .join("abc123")
            .join("model.onnx")
            .exists()
    );
    assert!(
        PathBuf::from(&config.search.embedding_cache_dir)
            .join("models--jinaai--jina-reranker-v2-base-multilingual")
            .join("snapshots")
            .join("abc123")
            .join("onnx")
            .join("model.onnx")
            .exists()
    );
    assert!(
        PathBuf::from(&config.search.embedding_cache_dir)
            .join("models--intfloat--multilingual-e5-base")
            .join("snapshots")
            .join("abc123")
            .join("tokenizer_config.json")
            .exists()
    );
    assert!(
        PathBuf::from(&config.search.embedding_cache_dir)
            .join("manifests")
            .join("manifest-hash")
            .join("dense__intfloat--multilingual-e5-base.rkyv.zst")
            .exists()
    );
    assert!(
        PathBuf::from(&config.search.embedding_cache_dir)
            .join("manifests")
            .join("manifest-hash")
            .join("sparse__Qdrant--Splade_PP_en_v1.rkyv.zst")
            .exists()
    );
}

#[test]
fn materialize_file_artifacts_rejects_models_archive_missing_refs() {
    let workspace = TempDir::new().expect("tempdir");
    let mut config = setup_config(&workspace);
    config.search.embedding_cache_dir = workspace
        .path()
        .join("embedding-cache")
        .to_string_lossy()
        .to_string();

    let storage_source = create_storage_source(&workspace);
    let storage_archive = workspace.path().join("storage.tar.gz");
    create_archive_from_dir(&storage_source, &storage_archive);

    let models_source = create_invalid_models_source_missing_refs(&workspace);
    let models_archive = workspace.path().join("models-invalid.tar.gz");
    create_archive_from_dir(&models_source, &models_archive);

    let metadata_path = workspace.path().join("nova-build-metadata.json");
    write_file(
        &metadata_path,
        br#"{
  "contract_version":"v1",
  "manifest_hash":"manifest-hash",
  "manifest_version":"v12",
  "entity_count":42,
  "storage_instance_id":"analytics-prod",
  "dbt_nova_version":"0.0.2",
  "build_timestamp":"2026-03-02T00:00:00Z",
  "artifact_name_storage":"storage-asset",
  "artifact_name_models":"models-asset"
}"#,
    );

    config.storage_artifact_uri = to_file_uri(&storage_archive);
    config.metadata_artifact_uri = to_file_uri(&metadata_path);
    config.models_artifact_uri = to_file_uri(&models_archive);
    config.artifact_fetch_policy = ArtifactFetchPolicy::IfMissing;

    let err = materialize_file_artifacts(&config, "manifest-hash")
        .expect_err("invalid models archive should be rejected");
    assert!(err.to_string().contains("missing refs directory"));
}

#[test]
fn materialize_file_artifacts_prefers_ref_target_snapshot_over_stale_main_snapshot() {
    let workspace = TempDir::new().expect("tempdir");
    let mut config = setup_config(&workspace);
    config.search.embedding_cache_dir = workspace
        .path()
        .join("embedding-cache")
        .to_string_lossy()
        .to_string();

    let storage_source = create_storage_source(&workspace);
    let storage_archive = workspace.path().join("storage.tar.gz");
    create_archive_from_dir(&storage_source, &storage_archive);

    let models_source = create_models_source(&workspace, "manifest-hash");
    write_file(
        &models_source
            .join("models--intfloat--multilingual-e5-base")
            .join("snapshots")
            .join("main")
            .join("README.txt"),
        b"stale-main-snapshot",
    );
    let models_archive = workspace.path().join("models-ref-target.tar.gz");
    create_archive_from_dir(&models_source, &models_archive);

    let metadata_path = workspace.path().join("nova-build-metadata.json");
    write_file(
        &metadata_path,
        br#"{
  "contract_version":"v1",
  "manifest_hash":"manifest-hash",
  "manifest_version":"v12",
  "entity_count":42,
  "storage_instance_id":"analytics-prod",
  "dbt_nova_version":"0.0.2",
  "build_timestamp":"2026-03-02T00:00:00Z",
  "artifact_name_storage":"storage-asset",
  "artifact_name_models":"models-asset"
}"#,
    );

    config.storage_artifact_uri = to_file_uri(&storage_archive);
    config.metadata_artifact_uri = to_file_uri(&metadata_path);
    config.models_artifact_uri = to_file_uri(&models_archive);
    config.artifact_fetch_policy = ArtifactFetchPolicy::IfMissing;

    let outcome = materialize_file_artifacts(&config, "manifest-hash")
        .expect("models archive should validate using refs/main target")
        .expect("artifact mode enabled");
    assert!(outcome.models_materialized);
    assert!(
        PathBuf::from(&config.search.embedding_cache_dir)
            .join("models--intfloat--multilingual-e5-base")
            .join("snapshots")
            .join("abc123")
            .join("tokenizer_config.json")
            .exists()
    );
}

#[test]
fn materialize_file_artifacts_rejects_models_archive_missing_manifest_scoped_caches() {
    let workspace = TempDir::new().expect("tempdir");
    let mut config = setup_config(&workspace);
    config.search.embedding_cache_dir = workspace
        .path()
        .join("embedding-cache")
        .to_string_lossy()
        .to_string();

    let storage_source = create_storage_source(&workspace);
    let storage_archive = workspace.path().join("storage.tar.gz");
    create_archive_from_dir(&storage_source, &storage_archive);

    let models_source = workspace.path().join("models-source-no-semantic");
    write_hf_model_repo(
        &models_source,
        "intfloat/multilingual-e5-base",
        "abc123",
        "onnx/model.onnx",
        &[],
    );
    write_hf_model_repo(
        &models_source,
        "Qdrant/Splade_PP_en_v1",
        "abc123",
        "model.onnx",
        &[],
    );
    write_hf_model_repo(
        &models_source,
        "jinaai/jina-reranker-v2-base-multilingual",
        "abc123",
        "onnx/model.onnx",
        &[],
    );
    let models_archive = workspace.path().join("models-missing-semantic.tar.gz");
    create_archive_from_dir(&models_source, &models_archive);

    let metadata_path = workspace.path().join("nova-build-metadata.json");
    write_file(
        &metadata_path,
        br#"{
  "contract_version":"v1",
  "manifest_hash":"manifest-hash",
  "manifest_version":"v12",
  "entity_count":42,
  "storage_instance_id":"analytics-prod",
  "dbt_nova_version":"0.0.2",
  "build_timestamp":"2026-03-02T00:00:00Z",
  "artifact_name_storage":"storage-asset",
  "artifact_name_models":"models-asset"
}"#,
    );

    config.storage_artifact_uri = to_file_uri(&storage_archive);
    config.metadata_artifact_uri = to_file_uri(&metadata_path);
    config.models_artifact_uri = to_file_uri(&models_archive);
    config.artifact_fetch_policy = ArtifactFetchPolicy::IfMissing;

    let err = materialize_file_artifacts(&config, "manifest-hash")
        .expect_err("missing manifest-scoped semantic caches should be rejected");
    assert!(err.to_string().contains("missing manifests directory"));
}

#[test]
fn materialize_file_artifacts_rejects_legacy_singleton_semantic_cache_files() {
    let workspace = TempDir::new().expect("tempdir");
    let mut config = setup_config(&workspace);
    config.search.embedding_cache_dir = workspace
        .path()
        .join("embedding-cache")
        .to_string_lossy()
        .to_string();

    let storage_source = create_storage_source(&workspace);
    let storage_archive = workspace.path().join("storage.tar.gz");
    create_archive_from_dir(&storage_source, &storage_archive);

    let models_source =
        create_models_source_with_legacy_semantic_caches(&workspace, "manifest-hash");
    let models_archive = workspace.path().join("models-legacy-semantic.tar.gz");
    create_archive_from_dir(&models_source, &models_archive);

    let metadata_path = workspace.path().join("nova-build-metadata.json");
    write_file(
        &metadata_path,
        br#"{
  "contract_version":"v1",
  "manifest_hash":"manifest-hash",
  "manifest_version":"v12",
  "entity_count":42,
  "storage_instance_id":"analytics-prod",
  "dbt_nova_version":"0.0.2",
  "build_timestamp":"2026-03-02T00:00:00Z",
  "artifact_name_storage":"storage-asset",
  "artifact_name_models":"models-asset"
}"#,
    );

    config.storage_artifact_uri = to_file_uri(&storage_archive);
    config.metadata_artifact_uri = to_file_uri(&metadata_path);
    config.models_artifact_uri = to_file_uri(&models_archive);
    config.artifact_fetch_policy = ArtifactFetchPolicy::IfMissing;

    let err = materialize_file_artifacts(&config, "manifest-hash")
        .expect_err("legacy singleton semantic cache files should be rejected");
    assert!(
        err.to_string()
            .contains("legacy singleton semantic cache file")
    );
}

#[test]
fn materialize_file_artifacts_rejects_manifest_hash_mismatch() {
    let workspace = TempDir::new().expect("tempdir");
    let mut config = setup_config(&workspace);

    let storage_source = create_storage_source(&workspace);
    let storage_archive = workspace.path().join("storage.tar.gz");
    create_archive_from_dir(&storage_source, &storage_archive);

    let metadata_path = workspace.path().join("nova-build-metadata.json");
    write_file(
        &metadata_path,
        br#"{
  "contract_version":"v1",
  "manifest_hash":"other-hash",
  "manifest_version":"v12",
  "entity_count":42,
  "storage_instance_id":"analytics-prod",
  "dbt_nova_version":"0.0.2",
  "build_timestamp":"2026-03-02T00:00:00Z",
  "artifact_name_storage":"storage-asset",
  "artifact_name_models":""
}"#,
    );

    config.storage_artifact_uri = to_file_uri(&storage_archive);
    config.metadata_artifact_uri = to_file_uri(&metadata_path);

    let err = materialize_file_artifacts(&config, "manifest-hash")
        .expect_err("should fail on hash mismatch");
    assert!(err.to_string().contains("manifest hash mismatch"));
}

#[test]
fn materialize_file_artifacts_policy_never_requires_local_storage() {
    let workspace = TempDir::new().expect("tempdir");
    let mut config = setup_config(&workspace);

    let storage_source = create_storage_source(&workspace);
    let storage_archive = workspace.path().join("storage.tar.gz");
    create_archive_from_dir(&storage_source, &storage_archive);

    let metadata_path = workspace.path().join("nova-build-metadata.json");
    write_file(
        &metadata_path,
        br#"{
  "contract_version":"v1",
  "manifest_hash":"manifest-hash",
  "manifest_version":"v12",
  "entity_count":42,
  "storage_instance_id":"analytics-prod",
  "dbt_nova_version":"0.0.2",
  "build_timestamp":"2026-03-02T00:00:00Z",
  "artifact_name_storage":"storage-asset",
  "artifact_name_models":""
}"#,
    );

    config.storage_artifact_uri = to_file_uri(&storage_archive);
    config.metadata_artifact_uri = to_file_uri(&metadata_path);
    config.artifact_fetch_policy = ArtifactFetchPolicy::Never;

    let err = materialize_file_artifacts(&config, "manifest-hash")
        .expect_err("policy never should fail when local storage missing");
    assert!(err.to_string().contains("policy is 'never'"));
}

#[test]
fn materialize_file_artifacts_if_missing_skips_existing_storage() {
    let workspace = TempDir::new().expect("tempdir");
    let mut config = setup_config(&workspace);

    let existing_version_dir = PathBuf::from(&config.storage_dir)
        .join("instances")
        .join("analytics-prod")
        .join("versions")
        .join("existing");
    write_file(&existing_version_dir.join("entities.bin"), b"entities");

    let storage_source = create_storage_source(&workspace);
    let storage_archive = workspace.path().join("storage.tar.gz");
    create_archive_from_dir(&storage_source, &storage_archive);

    let metadata_path = workspace.path().join("nova-build-metadata.json");
    write_file(
        &metadata_path,
        br#"{
  "contract_version":"v1",
  "manifest_hash":"manifest-hash",
  "manifest_version":"v12",
  "entity_count":42,
  "storage_instance_id":"analytics-prod",
  "dbt_nova_version":"0.0.2",
  "build_timestamp":"2026-03-02T00:00:00Z",
  "artifact_name_storage":"storage-asset",
  "artifact_name_models":""
}"#,
    );

    config.storage_artifact_uri = to_file_uri(&storage_archive);
    config.metadata_artifact_uri = to_file_uri(&metadata_path);
    config.artifact_fetch_policy = ArtifactFetchPolicy::IfMissing;

    let outcome = materialize_file_artifacts(&config, "manifest-hash")
        .expect("materialization should succeed")
        .expect("artifact mode enabled");
    assert!(!outcome.storage_materialized);
    assert!(!outcome.models_materialized);
}

#[test]
fn materialize_file_artifacts_if_missing_skips_remote_storage_fetch_when_local_present() {
    let workspace = TempDir::new().expect("tempdir");
    let mut config = setup_config(&workspace);

    let existing_version_dir = PathBuf::from(&config.storage_dir)
        .join("instances")
        .join("analytics-prod")
        .join("versions")
        .join("existing");
    write_file(&existing_version_dir.join("entities.bin"), b"entities");

    let metadata_path = workspace.path().join("nova-build-metadata.json");
    write_file(
        &metadata_path,
        br#"{
  "contract_version":"v1",
  "manifest_hash":"manifest-hash",
  "manifest_version":"v12",
  "entity_count":42,
  "storage_instance_id":"analytics-prod",
  "dbt_nova_version":"0.0.2",
  "build_timestamp":"2026-03-02T00:00:00Z",
  "artifact_name_storage":"storage-asset",
  "artifact_name_models":""
}"#,
    );

    config.storage_artifact_uri = "s3://bucket/storage.tar.gz".to_string();
    config.metadata_artifact_uri = to_file_uri(&metadata_path);
    config.artifact_fetch_policy = ArtifactFetchPolicy::IfMissing;

    let outcome = materialize_file_artifacts(&config, "manifest-hash")
        .expect("existing local storage should avoid remote storage fetch")
        .expect("artifact mode enabled");
    assert!(!outcome.storage_materialized);
}

#[test]
fn materialize_file_artifacts_if_missing_skips_remote_models_fetch_when_local_cache_is_complete() {
    let workspace = TempDir::new().expect("tempdir");
    let mut config = setup_config(&workspace);
    config.search.embedding_cache_dir = workspace
        .path()
        .join("embedding-cache")
        .to_string_lossy()
        .to_string();

    let existing_version_dir = PathBuf::from(&config.storage_dir)
        .join("instances")
        .join("analytics-prod")
        .join("versions")
        .join("existing");
    write_file(&existing_version_dir.join("entities.bin"), b"entities");

    write_complete_models_cache(
        Path::new(&config.search.embedding_cache_dir),
        "manifest-hash",
    );

    let metadata_path = workspace.path().join("nova-build-metadata.json");
    write_file(
        &metadata_path,
        br#"{
  "contract_version":"v1",
  "manifest_hash":"manifest-hash",
  "manifest_version":"v12",
  "entity_count":42,
  "storage_instance_id":"analytics-prod",
  "dbt_nova_version":"0.0.2",
  "build_timestamp":"2026-03-02T00:00:00Z",
  "artifact_name_storage":"storage-asset",
  "artifact_name_models":"models-asset"
}"#,
    );

    config.storage_artifact_uri = "s3://bucket/storage.tar.gz".to_string();
    config.metadata_artifact_uri = to_file_uri(&metadata_path);
    config.models_artifact_uri = "gs://bucket/models.tar.gz".to_string();
    config.artifact_fetch_policy = ArtifactFetchPolicy::IfMissing;

    let outcome = materialize_file_artifacts(&config, "manifest-hash")
        .expect("existing local models should avoid remote models fetch")
        .expect("artifact mode enabled");
    assert!(!outcome.storage_materialized);
    assert!(!outcome.models_materialized);
}

#[test]
fn materialize_file_artifacts_if_missing_fetches_remote_models_when_local_cache_is_incomplete() {
    let workspace = TempDir::new().expect("tempdir");
    let mut config = setup_config(&workspace);
    config.search.embedding_cache_dir = workspace
        .path()
        .join("embedding-cache")
        .to_string_lossy()
        .to_string();

    let existing_version_dir = PathBuf::from(&config.storage_dir)
        .join("instances")
        .join("analytics-prod")
        .join("versions")
        .join("existing");
    write_file(&existing_version_dir.join("entities.bin"), b"entities");

    let existing_model_ref = PathBuf::from(&config.search.embedding_cache_dir)
        .join("models--intfloat--multilingual-e5-base")
        .join("refs")
        .join("main");
    write_file(&existing_model_ref, b"abc123");
    let existing_model = PathBuf::from(&config.search.embedding_cache_dir)
        .join("models--intfloat--multilingual-e5-base")
        .join("snapshots")
        .join("abc123")
        .join("onnx")
        .join("model.onnx");
    write_file(&existing_model, b"model");

    let models_source = create_models_source(&workspace, "manifest-hash");
    let models_archive = workspace.path().join("models.tar.gz");
    create_archive_from_dir(&models_source, &models_archive);

    let metadata_path = workspace.path().join("nova-build-metadata.json");
    write_file(
        &metadata_path,
        br#"{
  "contract_version":"v1",
  "manifest_hash":"manifest-hash",
  "manifest_version":"v12",
  "entity_count":42,
  "storage_instance_id":"analytics-prod",
  "dbt_nova_version":"0.0.2",
  "build_timestamp":"2026-03-02T00:00:00Z",
  "artifact_name_storage":"storage-asset",
  "artifact_name_models":"models-asset"
}"#,
    );

    config.storage_artifact_uri = "s3://bucket/storage.tar.gz".to_string();
    config.metadata_artifact_uri = to_file_uri(&metadata_path);
    config.models_artifact_uri = to_file_uri(&models_archive);
    config.artifact_fetch_policy = ArtifactFetchPolicy::IfMissing;

    let outcome = materialize_file_artifacts(&config, "manifest-hash")
        .expect("incomplete local models cache should trigger artifact hydration")
        .expect("artifact mode enabled");
    assert!(!outcome.storage_materialized);
    assert!(outcome.models_materialized);
    assert!(
        PathBuf::from(&config.search.embedding_cache_dir)
            .join("manifests")
            .join("manifest-hash")
            .join("dense__intfloat--multilingual-e5-base.rkyv.zst")
            .exists()
    );
    assert!(
        PathBuf::from(&config.search.embedding_cache_dir)
            .join("manifests")
            .join("manifest-hash")
            .join("sparse__Qdrant--Splade_PP_en_v1.rkyv.zst")
            .exists()
    );
}

#[test]
fn materialize_file_artifacts_policy_never_rejects_incomplete_local_models_cache() {
    let workspace = TempDir::new().expect("tempdir");
    let mut config = setup_config(&workspace);
    config.search.embedding_cache_dir = workspace
        .path()
        .join("embedding-cache")
        .to_string_lossy()
        .to_string();

    let existing_version_dir = PathBuf::from(&config.storage_dir)
        .join("instances")
        .join("analytics-prod")
        .join("versions")
        .join("existing");
    write_file(&existing_version_dir.join("entities.bin"), b"entities");

    let existing_model_ref = PathBuf::from(&config.search.embedding_cache_dir)
        .join("models--intfloat--multilingual-e5-base")
        .join("refs")
        .join("main");
    write_file(&existing_model_ref, b"abc123");
    let existing_model = PathBuf::from(&config.search.embedding_cache_dir)
        .join("models--intfloat--multilingual-e5-base")
        .join("snapshots")
        .join("abc123")
        .join("onnx")
        .join("model.onnx");
    write_file(&existing_model, b"model");

    let metadata_path = workspace.path().join("nova-build-metadata.json");
    write_file(
        &metadata_path,
        br#"{
  "contract_version":"v1",
  "manifest_hash":"manifest-hash",
  "manifest_version":"v12",
  "entity_count":42,
  "storage_instance_id":"analytics-prod",
  "dbt_nova_version":"0.0.2",
  "build_timestamp":"2026-03-02T00:00:00Z",
  "artifact_name_storage":"storage-asset",
  "artifact_name_models":"models-asset"
}"#,
    );

    config.storage_artifact_uri = "s3://bucket/storage.tar.gz".to_string();
    config.metadata_artifact_uri = to_file_uri(&metadata_path);
    config.models_artifact_uri = "gs://bucket/models.tar.gz".to_string();
    config.artifact_fetch_policy = ArtifactFetchPolicy::Never;

    let err = materialize_file_artifacts(&config, "manifest-hash")
        .expect_err("policy never should reject incomplete local models cache");
    assert!(err.to_string().contains("incomplete or invalid"));
}

#[test]
fn materialize_file_artifacts_supports_remote_cached_uris_when_policy_never() {
    let workspace = TempDir::new().expect("tempdir");
    let mut config = setup_config(&workspace);
    let existing_version_dir = PathBuf::from(&config.storage_dir)
        .join("instances")
        .join("analytics-prod")
        .join("versions")
        .join("existing");
    write_file(&existing_version_dir.join("entities.bin"), b"entities");

    let storage_source = create_storage_source(&workspace);
    let storage_archive = workspace.path().join("storage.tar.gz");
    create_archive_from_dir(&storage_source, &storage_archive);

    let metadata_path = workspace.path().join("nova-build-metadata.json");
    write_file(
        &metadata_path,
        br#"{
  "contract_version":"v1",
  "manifest_hash":"manifest-hash",
  "manifest_version":"v12",
  "entity_count":42,
  "storage_instance_id":"analytics-prod",
  "dbt_nova_version":"0.0.2",
  "build_timestamp":"2026-03-02T00:00:00Z",
  "artifact_name_storage":"storage-asset",
  "artifact_name_models":""
}"#,
    );

    let storage_uri = "s3://bucket/storage.tar.gz";
    let metadata_uri = "gs://bucket/nova-build-metadata.json";

    let cache_dir = config
        .artifacts_cache_dir()
        .expect("resolve artifacts cache dir");
    write_cached_artifact(&cache_dir, storage_uri, &storage_archive);
    write_cached_artifact(&cache_dir, metadata_uri, &metadata_path);

    config.storage_artifact_uri = storage_uri.to_string();
    config.metadata_artifact_uri = metadata_uri.to_string();
    config.artifact_fetch_policy = ArtifactFetchPolicy::Never;

    let outcome = materialize_file_artifacts(&config, "manifest-hash")
        .expect("materialization should succeed from cached remote metadata")
        .expect("artifact mode enabled");
    assert!(!outcome.storage_materialized);
    assert!(!outcome.models_materialized);
}

#[test]
fn materialize_file_artifacts_if_missing_uses_cached_dbfs_without_auth() {
    let _env_guard = lock_env();
    let _host_restore = EnvVarRestore::remove("DATABRICKS_HOST");
    let _token_restore = EnvVarRestore::remove("DATABRICKS_ACCESS_TOKEN");

    let workspace = TempDir::new().expect("tempdir");
    let mut config = setup_config(&workspace);
    let existing_version_dir = PathBuf::from(&config.storage_dir)
        .join("instances")
        .join("analytics-prod")
        .join("versions")
        .join("existing");
    write_file(&existing_version_dir.join("entities.bin"), b"entities");

    let storage_source = create_storage_source(&workspace);
    let storage_archive = workspace.path().join("storage.tar.gz");
    create_archive_from_dir(&storage_source, &storage_archive);

    let metadata_path = workspace.path().join("nova-build-metadata.json");
    write_file(
        &metadata_path,
        br#"{
  "contract_version":"v1",
  "manifest_hash":"manifest-hash",
  "manifest_version":"v12",
  "entity_count":42,
  "storage_instance_id":"analytics-prod",
  "dbt_nova_version":"0.0.2",
  "build_timestamp":"2026-03-02T00:00:00Z",
  "artifact_name_storage":"storage-asset",
  "artifact_name_models":""
}"#,
    );

    let storage_uri = "dbfs:/FileStore/storage.tar.gz";
    let metadata_uri = "dbfs:/FileStore/nova-build-metadata.json";
    let cache_dir = config
        .artifacts_cache_dir()
        .expect("resolve artifacts cache dir");
    write_cached_artifact(&cache_dir, storage_uri, &storage_archive);
    write_cached_artifact(&cache_dir, metadata_uri, &metadata_path);

    config.storage_artifact_uri = storage_uri.to_string();
    config.metadata_artifact_uri = metadata_uri.to_string();
    config.artifact_fetch_policy = ArtifactFetchPolicy::IfMissing;

    let outcome = materialize_file_artifacts(&config, "manifest-hash")
        .expect("cached DBFS artifacts should not require auth")
        .expect("artifact mode enabled");
    assert!(!outcome.storage_materialized);
}

#[test]
fn materialize_file_artifacts_dbfs_requires_databricks_auth() {
    let _env_guard = lock_env();
    let _host_restore = EnvVarRestore::remove("DATABRICKS_HOST");
    let _token_restore = EnvVarRestore::remove("DATABRICKS_ACCESS_TOKEN");

    let workspace = TempDir::new().expect("tempdir");
    let mut config = setup_config(&workspace);
    config.storage_artifact_uri = "dbfs:/FileStore/storage.tar.gz".to_string();
    config.metadata_artifact_uri = "dbfs:/FileStore/nova-build-metadata.json".to_string();
    config.artifact_fetch_policy = ArtifactFetchPolicy::IfMissing;

    let err = materialize_file_artifacts(&config, "manifest-hash")
        .expect_err("dbfs fetch without auth should fail");
    let message = err.to_string();
    assert!(message.contains("DATABRICKS_HOST not set"));
}
