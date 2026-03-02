use dbt_nova::config::{ArtifactFetchPolicy, DbtNovaConfig};
use dbt_nova::manifest::prebuilt_assets_resolver::materialize_file_artifacts;
use flate2::Compression;
use flate2::write::GzEncoder;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use tar::Builder;
use tempfile::TempDir;

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

fn setup_config(workspace: &TempDir) -> DbtNovaConfig {
    let manifest_path = workspace.path().join("nova_manifest.json");
    write_file(&manifest_path, br#"{"metadata":{"dbt_version":"1.8.0"}}"#);

    DbtNovaConfig {
        manifest_path: manifest_path.to_string_lossy().to_string(),
        storage_dir: workspace
            .path()
            .join(".dbt-nova")
            .to_string_lossy()
            .to_string(),
        storage_instance_id: "analytics-prod".to_string(),
        ..DbtNovaConfig::default()
    }
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

fn create_models_source(workspace: &TempDir) -> PathBuf {
    let models_source = workspace.path().join("models-source");
    let model_path = models_source
        .join("models--intfloat--multilingual-e5-base")
        .join("snapshots")
        .join("main")
        .join("onnx")
        .join("model.onnx");
    write_file(&model_path, b"model");
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

    let models_source = create_models_source(&workspace);
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
            .join("snapshots")
            .join("main")
            .join("onnx")
            .join("model.onnx")
            .exists()
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
fn materialize_file_artifacts_rejects_non_file_scheme_for_p2() {
    let workspace = TempDir::new().expect("tempdir");
    let mut config = setup_config(&workspace);
    config.storage_artifact_uri = "s3://bucket/storage.tar.gz".to_string();
    config.metadata_artifact_uri = "s3://bucket/nova-build-metadata.json".to_string();

    let err = materialize_file_artifacts(&config, "manifest-hash")
        .expect_err("non-file URIs should be rejected by file resolver");
    assert!(err.to_string().contains("non-file URI scheme"));
}
