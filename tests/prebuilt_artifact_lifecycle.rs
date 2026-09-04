//! Integration tests for remote prebuilt artifact lifecycle wiring.
use std::fs::{self, File};
use std::io::ErrorKind;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use dbt_nova::ManifestSearch;
use dbt_nova::config::{ArtifactFetchPolicy, DbtNovaConfig, SearchConfig};
use flate2::Compression;
use flate2::write::GzEncoder;
use tar::Builder;
use tempfile::TempDir;

static LIFECYCLE_TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

fn fixture_manifest_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("nova_manifest.json")
}

fn build_config(storage_dir: &Path) -> DbtNovaConfig {
    let search = SearchConfig {
        enable_vector_search: false,
        enable_sparse_search: false,
        enable_reranker: false,
        embedding_cache_dir: storage_dir
            .join("models-cache")
            .to_string_lossy()
            .to_string(),
        ..SearchConfig::default()
    };

    DbtNovaConfig {
        manifest_path: fixture_manifest_path().to_string_lossy().to_string(),
        storage_dir: storage_dir.to_string_lossy().to_string(),
        storage_instance_id: "analytics-prod".to_string(),
        // Keep lifecycle tests deterministic: the archive source snapshot should not
        // be mutated by background refresh while it is being packaged.
        manifest_refresh_secs: 0,
        search,
        ..DbtNovaConfig::default()
    }
}

fn write_file(path: &Path, content: &[u8]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent dirs");
    }
    let mut file = File::create(path).expect("create file");
    file.write_all(content).expect("write file");
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
    tar.append_dir(&root_name, source_dir)
        .expect("append root dir to archive");

    let mut stack = vec![(source_dir.to_path_buf(), PathBuf::from(&root_name))];
    while let Some((current_abs, current_rel)) = stack.pop() {
        let entries = match fs::read_dir(&current_abs) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => panic!("read archive source dir {}: {error}", current_abs.display()),
        };

        for entry_result in entries {
            let entry = match entry_result {
                Ok(entry) => entry,
                Err(error) if error.kind() == ErrorKind::NotFound => continue,
                Err(error) => panic!("read archive entry in {}: {error}", current_abs.display()),
            };

            let file_name = entry.file_name();
            if file_name.to_string_lossy().starts_with(".tmp") {
                continue;
            }

            let entry_abs = entry.path();
            let entry_rel = current_rel.join(file_name);
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(error) if error.kind() == ErrorKind::NotFound => continue,
                Err(error) => panic!("read entry type {}: {error}", entry_abs.display()),
            };

            if file_type.is_dir() {
                tar.append_dir(&entry_rel, &entry_abs)
                    .unwrap_or_else(|error| panic!("append dir {}: {error}", entry_abs.display()));
                stack.push((entry_abs, entry_rel));
                continue;
            }

            if file_type.is_file() {
                let file_name = entry_rel
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_default();
                if file_name.ends_with(".lock")
                    || file_name.ends_with(".tmp")
                    || file_name.contains(".tmp.")
                {
                    continue;
                }

                let bytes = match fs::read(&entry_abs) {
                    Ok(bytes) => bytes,
                    Err(error) if error.kind() == ErrorKind::NotFound => continue,
                    Err(error) => panic!("read entry file {}: {error}", entry_abs.display()),
                };
                let mut header = tar::Header::new_gnu();
                header.set_size(u64::try_from(bytes.len()).expect("archive entry size fits u64"));
                header.set_mode(0o644);
                header.set_entry_type(tar::EntryType::Regular);
                header.set_cksum();
                tar.append_data(&mut header, &entry_rel, bytes.as_slice())
                    .unwrap_or_else(|error| panic!("append file {}: {error}", entry_abs.display()));
            }
        }
    }
    let encoder = tar.into_inner().expect("finish tar stream");
    encoder.finish().expect("finish gzip stream");
}

fn to_file_uri(path: &Path) -> String {
    format!("file://{}", path.display())
}

struct SourceStorageFixture {
    root: PathBuf,
    manifest_hash: String,
    manifest_version: String,
}

fn build_source_storage(workspace: &TempDir) -> SourceStorageFixture {
    let source_storage_dir = workspace.path().join("source-storage");
    let source_config = build_config(&source_storage_dir);
    let source_storage_root = source_config
        .storage_root_dir()
        .expect("source storage root should resolve");
    let load = ManifestSearch::new(source_config).expect("source manifest build should succeed");
    assert!(load.search.entity_count() > 0);
    assert!(
        source_storage_root.is_dir(),
        "source storage root should exist"
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let health = runtime.block_on(load.search.health_snapshot());
    assert_eq!(
        health["manifest"]["storage_format_version"],
        serde_json::json!("nova-storage-v2")
    );
    SourceStorageFixture {
        root: source_storage_root,
        manifest_hash: health["manifest"]["hash"]
            .as_str()
            .expect("manifest hash")
            .to_string(),
        manifest_version: health["manifest"]["version"]
            .as_str()
            .expect("manifest version")
            .to_string(),
    }
}

fn write_metadata(
    path: &Path,
    manifest_hash: &str,
    manifest_version: &str,
    include_models_artifact: bool,
) {
    let artifact_name_models = if include_models_artifact {
        "models-asset"
    } else {
        ""
    };
    let payload = serde_json::json!({
        "contract_version": "v2",
        "storage_format_version": "nova-storage-v2",
        "manifest_hash": manifest_hash,
        "manifest_version": manifest_version,
        "entity_count": 1,
        "storage_instance_id": "analytics-prod",
        "dbt_nova_version": "0.0.2",
        "build_timestamp": "2026-03-02T00:00:00Z",
        "artifact_name_storage": "storage-asset",
        "artifact_name_models": artifact_name_models
    });
    write_file(
        path,
        serde_json::to_string(&payload)
            .expect("serialize metadata")
            .as_bytes(),
    );
}

#[test]
fn manifest_load_materializes_file_artifacts_before_reuse() {
    let _guard = LIFECYCLE_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let workspace = TempDir::new().expect("tempdir");
    let source = build_source_storage(&workspace);

    let storage_archive_path = workspace.path().join("storage.tar.gz");
    create_archive_from_dir(&source.root, &storage_archive_path);

    let metadata_path = workspace.path().join("nova-build-metadata.json");
    write_metadata(
        &metadata_path,
        &source.manifest_hash,
        &source.manifest_version,
        false,
    );

    let destination_storage_dir = workspace.path().join("destination-storage");
    let mut destination_config = build_config(&destination_storage_dir);
    destination_config.storage_artifact_uri = to_file_uri(&storage_archive_path);
    destination_config.metadata_artifact_uri = to_file_uri(&metadata_path);
    destination_config.artifact_fetch_policy = ArtifactFetchPolicy::Always;

    let load =
        ManifestSearch::new(destination_config).expect("artifact-backed load should succeed");
    assert!(
        load.entity_store_reused,
        "storage artifact should be reused"
    );
    assert!(load.tantivy_reused, "tantivy index should be reused");
    assert!(load.indexes_reused, "indexes cache should be reused");
    assert!(load.search.entity_count() > 0);

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let health = runtime.block_on(load.search.health_snapshot());
    let artifact_consumer = &health["artifact_consumer"];
    assert_eq!(artifact_consumer["enabled"], serde_json::json!(true));
    assert_eq!(
        artifact_consumer["fetch_policy"],
        serde_json::json!("always")
    );
    assert_eq!(
        artifact_consumer["metadata_validated"],
        serde_json::json!(true)
    );
    assert_eq!(
        artifact_consumer["storage_materialized"],
        serde_json::json!(true)
    );
    assert!(artifact_consumer["last_evaluated_at_ms"].as_u64().is_some());
    assert!(
        artifact_consumer["last_materialized_at_ms"]
            .as_u64()
            .is_some()
    );
}

#[test]
fn manifest_load_rejects_artifact_metadata_hash_mismatch() {
    let _guard = LIFECYCLE_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let workspace = TempDir::new().expect("tempdir");
    let source = build_source_storage(&workspace);

    let storage_archive_path = workspace.path().join("storage.tar.gz");
    create_archive_from_dir(&source.root, &storage_archive_path);

    let metadata_path = workspace.path().join("nova-build-metadata.json");
    write_metadata(
        &metadata_path,
        "mismatched-hash",
        &source.manifest_version,
        false,
    );

    let destination_storage_dir = workspace.path().join("destination-storage");
    let mut destination_config = build_config(&destination_storage_dir);
    destination_config.storage_artifact_uri = to_file_uri(&storage_archive_path);
    destination_config.metadata_artifact_uri = to_file_uri(&metadata_path);
    destination_config.artifact_fetch_policy = ArtifactFetchPolicy::Always;

    let error = match ManifestSearch::new(destination_config) {
        Ok(_) => panic!("hash mismatch should fail"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("manifest hash mismatch"));
}

#[test]
fn manifest_load_read_only_rejects_artifact_materialization_when_missing() {
    let _guard = LIFECYCLE_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let workspace = TempDir::new().expect("tempdir");
    let source = build_source_storage(&workspace);

    let storage_archive_path = workspace.path().join("storage.tar.gz");
    create_archive_from_dir(&source.root, &storage_archive_path);

    let metadata_path = workspace.path().join("nova-build-metadata.json");
    write_metadata(
        &metadata_path,
        &source.manifest_hash,
        &source.manifest_version,
        false,
    );

    let destination_storage_dir = workspace.path().join("destination-storage");
    let mut destination_config = build_config(&destination_storage_dir);
    destination_config.storage_artifact_uri = to_file_uri(&storage_archive_path);
    destination_config.metadata_artifact_uri = to_file_uri(&metadata_path);
    destination_config.artifact_fetch_policy = ArtifactFetchPolicy::IfMissing;
    destination_config.storage_read_only = true;

    let error = match ManifestSearch::new(destination_config) {
        Ok(_) => panic!("read-only mode should reject required materialization"),
        Err(error) => error,
    };
    assert!(error.to_string().contains(
        "Storage is read-only; cannot materialize storage artifacts on first-run hydration"
    ));
    assert!(error.to_string().contains(
        "unset DBT_NOVA_STORAGE_READ_ONLY and use DBT_NOVA_ARTIFACT_FETCH_POLICY=if_missing|always"
    ));
}
