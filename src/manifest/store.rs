use std::collections::HashMap;
use std::fs::{self, File as StdFile};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use blake3::Hasher;
use memmap2::Mmap;
use rkyv::api::high::{access, from_bytes, to_bytes};
use rkyv::rancor::Error as RkyvError;
use serde::{Deserialize, Serialize};
use tracing::instrument;

use crate::error::{DbtNovaError, Result};
use crate::manifest::entity::{ArchivedEntity, Entity};
use crate::utils::unique_suffix;

const ENTITY_DATA_FILENAME: &str = "entities.bin";
const ENTITY_INDEX_FILENAME: &str = "entities.idx";
const ENTITY_CHECKSUM_FILENAME: &str = "entities.checksum.json";
const ENTITY_SCHEMA_VERSION: u32 = 7;

#[derive(Debug, Serialize, Deserialize)]
struct EntityChecksums {
    #[serde(default)]
    schema_version: u32,
    data_hash: String,
    data_len: u64,
    index_hash: String,
    index_len: u64,
}

/// On-disk store for manifest entities with memory-mapped reads.
pub struct EntityStore {
    data_path: PathBuf,
    _data_file: StdFile,
    data_map: Mmap,
    index: HashMap<String, (u64, u64)>,
}

/// Builder for the entity store (writes entities and index files).
pub struct EntityStoreBuilder {
    data_tmp: PathBuf,
    data_path: PathBuf,
    index_path: PathBuf,
    storage_dir: PathBuf,
    writer: BufWriter<StdFile>,
    index: HashMap<String, (u64, u64)>,
    offset: u64,
}

impl EntityStoreBuilder {
    /// Create a new builder for a storage directory.
    ///
    /// # Errors
    /// Returns an error if the storage directory or temporary files cannot be created.
    pub fn new(storage_dir: &Path) -> Result<Self> {
        fs::create_dir_all(storage_dir)?;

        let suffix = unique_suffix();

        let data_path = storage_dir.join(ENTITY_DATA_FILENAME);
        let index_path = storage_dir.join(ENTITY_INDEX_FILENAME);
        let data_tmp = storage_dir.join(format!("{ENTITY_DATA_FILENAME}.{suffix}.tmp"));

        let writer = BufWriter::new(StdFile::create(&data_tmp)?);

        Ok(Self {
            data_tmp,
            data_path,
            index_path,
            storage_dir: storage_dir.to_path_buf(),
            writer,
            index: HashMap::new(),
            offset: 0,
        })
    }

    /// Add an entity to the store by `unique_id`.
    ///
    /// # Errors
    /// Returns an error if the entity cannot be serialized or written to disk.
    pub fn add(&mut self, unique_id: &str, entity: &Entity) -> Result<()> {
        let bytes = to_bytes::<RkyvError>(entity).map_err(|e| {
            DbtNovaError::ServerError(format!("Failed to serialize entity {unique_id}: {e}"))
        })?;
        let len = bytes.len() as u64;
        self.writer.write_all(&bytes)?;
        self.index.insert(unique_id.to_string(), (self.offset, len));
        self.offset += len;
        Ok(())
    }

    /// Finalize the store, write index/checksum files, and open the store.
    ///
    /// # Errors
    /// Returns an error if any store files fail to write or validate.
    pub fn finish(mut self) -> Result<EntityStore> {
        self.writer.flush()?;

        if let Err(err) = fs::rename(&self.data_tmp, &self.data_path) {
            if self.data_path.exists() {
                fs::remove_file(&self.data_path)?;
                fs::rename(&self.data_tmp, &self.data_path)?;
            } else {
                return Err(DbtNovaError::ServerError(err.to_string()));
            }
        }

        let index_tmp = self.index_path.with_file_name(format!(
            "{}.{}.tmp",
            ENTITY_INDEX_FILENAME,
            unique_suffix()
        ));
        let index_file = StdFile::create(&index_tmp)?;
        serde_json::to_writer(BufWriter::new(index_file), &self.index).map_err(|err| {
            DbtNovaError::ManifestError(format!("Failed to serialize entity index: {err}"))
        })?;
        if let Err(err) = fs::rename(&index_tmp, &self.index_path) {
            if self.index_path.exists() {
                fs::remove_file(&self.index_path)?;
                fs::rename(&index_tmp, &self.index_path)?;
            } else {
                return Err(DbtNovaError::ServerError(err.to_string()));
            }
        }

        self.write_checksums()?;

        EntityStore::open(&self.storage_dir)
    }

    fn write_checksums(&self) -> Result<()> {
        let (data_hash, data_len) = hash_file(&self.data_path)?;
        let (index_hash, index_len) = hash_file(&self.index_path)?;

        let checksums = EntityChecksums {
            schema_version: ENTITY_SCHEMA_VERSION,
            data_hash,
            data_len,
            index_hash,
            index_len,
        };

        let checksum_path = self.storage_dir.join(ENTITY_CHECKSUM_FILENAME);
        let checksum_tmp = checksum_path.with_file_name(format!(
            "{}.{}.tmp",
            ENTITY_CHECKSUM_FILENAME,
            unique_suffix()
        ));
        let file = StdFile::create(&checksum_tmp)?;
        serde_json::to_writer(BufWriter::new(file), &checksums).map_err(|err| {
            DbtNovaError::ManifestError(format!("Failed to serialize entity checksums: {err}"))
        })?;
        if let Err(err) = fs::rename(&checksum_tmp, &checksum_path) {
            if checksum_path.exists() {
                fs::remove_file(&checksum_path)?;
                fs::rename(&checksum_tmp, &checksum_path)?;
            } else {
                return Err(DbtNovaError::ServerError(err.to_string()));
            }
        }
        Ok(())
    }
}

impl EntityStore {
    #[instrument(level = "info", skip(storage_dir), fields(storage_dir = %storage_dir.display()))]
    /// Open an existing entity store from disk.
    ///
    /// # Errors
    /// Returns an error if the store files are missing, invalid, or fail checksum validation.
    pub fn open(storage_dir: &Path) -> Result<Self> {
        let data_path = storage_dir.join(ENTITY_DATA_FILENAME);
        let index_path = storage_dir.join(ENTITY_INDEX_FILENAME);
        let checksum_path = storage_dir.join(ENTITY_CHECKSUM_FILENAME);

        if checksum_path.exists() {
            let file = StdFile::open(&checksum_path)?;
            let checksums: EntityChecksums = serde_json::from_reader(BufReader::new(file))
                .map_err(|e| {
                    DbtNovaError::ServerError(format!("Invalid entity checksum file: {e}"))
                })?;
            if checksums.schema_version != ENTITY_SCHEMA_VERSION {
                return Err(DbtNovaError::ServerError(format!(
                    "Entity store schema version mismatch ({} != {})",
                    checksums.schema_version, ENTITY_SCHEMA_VERSION
                )));
            }
            let (data_hash, data_len) = hash_file(&data_path)?;
            if data_len != checksums.data_len || data_hash != checksums.data_hash {
                return Err(DbtNovaError::ServerError(
                    "Entity store checksum mismatch for entities.bin".to_string(),
                ));
            }
            let (index_hash, index_len) = hash_file(&index_path)?;
            if index_len != checksums.index_len || index_hash != checksums.index_hash {
                return Err(DbtNovaError::ServerError(
                    "Entity store checksum mismatch for entities.idx".to_string(),
                ));
            }
        } else {
            return Err(DbtNovaError::ServerError(
                "Entity checksum file missing; refusing to open store".to_string(),
            ));
        }

        let file = StdFile::open(&index_path)?;
        let index: HashMap<String, (u64, u64)> = serde_json::from_reader(BufReader::new(file))?;
        let data_file = StdFile::open(&data_path)?;
        // SAFETY: `data_file` outlives the mmap, the mapping is read-only, and the file is not
        // mutated while the store is in use. We validate checksums before mapping to detect
        // corruption, and all slice access is bounds-checked against the mmap length.
        let data_map = unsafe { Mmap::map(&data_file)? };

        Ok(Self {
            data_path,
            _data_file: data_file,
            data_map,
            index,
        })
    }

    /// Fetch an entity by `unique_id` (async wrapper).
    ///
    /// # Errors
    /// Returns an error if the entity data cannot be read or decoded.
    #[allow(clippy::unused_async)]
    pub async fn get(&self, unique_id: &str) -> Result<Option<Entity>> {
        self.get_internal(unique_id)
    }

    /// Fetch an entity by `unique_id` without async overhead.
    ///
    /// # Errors
    /// Returns an error if the entity data cannot be read or decoded.
    pub fn get_blocking(&self, unique_id: &str) -> Result<Option<Entity>> {
        self.get_internal(unique_id)
    }

    fn get_internal(&self, unique_id: &str) -> Result<Option<Entity>> {
        let (offset, len) = match self.index.get(unique_id) {
            Some(entry) => *entry,
            None => return Ok(None),
        };

        let start = usize::try_from(offset).map_err(|_| {
            DbtNovaError::ServerError(format!("Entity store offset out of bounds for {unique_id}"))
        })?;
        let len = usize::try_from(len).map_err(|_| {
            DbtNovaError::ServerError(format!("Entity store length out of bounds for {unique_id}"))
        })?;
        let end = start.saturating_add(len);
        if end > self.data_map.len() {
            return Err(DbtNovaError::ServerError(format!(
                "Entity store index out of bounds for {unique_id}"
            )));
        }
        let bytes = &self.data_map[start..end];

        let entity = from_bytes::<Entity, RkyvError>(bytes).map_err(|rkyv_err| {
            DbtNovaError::ServerError(format!(
                "Failed to deserialize entity {unique_id}: {rkyv_err}"
            ))
        })?;
        Ok(Some(entity))
    }

    /// Fetch an entity by `unique_id` as an archived reference.
    ///
    /// # Errors
    /// Returns an error if the entity data cannot be read or decoded.
    pub fn get_archived(&self, unique_id: &str) -> Result<Option<&ArchivedEntity>> {
        let (offset, len) = match self.index.get(unique_id) {
            Some(entry) => *entry,
            None => return Ok(None),
        };

        let start = usize::try_from(offset).map_err(|_| {
            DbtNovaError::ServerError(format!("Entity store offset out of bounds for {unique_id}"))
        })?;
        let len = usize::try_from(len).map_err(|_| {
            DbtNovaError::ServerError(format!("Entity store length out of bounds for {unique_id}"))
        })?;
        let end = start.saturating_add(len);
        if end > self.data_map.len() {
            return Err(DbtNovaError::ServerError(format!(
                "Entity store index out of bounds for {unique_id}"
            )));
        }
        let bytes = &self.data_map[start..end];

        let archived = access::<ArchivedEntity, RkyvError>(bytes).map_err(|rkyv_err| {
            DbtNovaError::ServerError(format!(
                "Failed to access archived entity {unique_id}: {rkyv_err}"
            ))
        })?;
        Ok(Some(archived))
    }

    /// Returns true if the entity exists in the store.
    #[must_use]
    pub fn contains(&self, unique_id: &str) -> bool {
        self.index.contains_key(unique_id)
    }

    /// Number of entities in the store.
    #[must_use]
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// True when the store contains no entities.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// Iterator over all `unique_ids` in the store.
    pub fn ids(&self) -> impl Iterator<Item = &String> {
        self.index.keys()
    }

    /// Absolute path to the entity data file.
    #[must_use]
    pub fn data_path(&self) -> &Path {
        &self.data_path
    }
}

fn hash_file(path: &Path) -> Result<(String, u64)> {
    let file = StdFile::open(path)?;
    let len = file.metadata()?.len();
    let mut reader = BufReader::new(file);
    let mut hasher = Hasher::new();
    let mut buf = [0u8; 8192];
    loop {
        let read = reader.read(&mut buf)?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    Ok((hasher.finalize().to_hex().to_string(), len))
}
