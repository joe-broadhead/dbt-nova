use std::collections::{HashMap, HashSet};

/// Schema version for cache invalidation.
pub const RKYV_SCHEMA_VERSION: u32 = 4;

/// All manifest indexes bundled for persistence.
#[derive(Clone, Debug, rkyv_derive::Archive, rkyv_derive::Serialize, rkyv_derive::Deserialize)]
#[rkyv(bytecheck())]
pub struct PersistedIndexes {
    pub schema_version: u32,
    pub manifest_hash: String,

    // Lineage
    pub parent_map: HashMap<String, Vec<String>>,
    pub child_map: HashMap<String, Vec<String>>,

    // Classification
    pub by_resource_type: HashMap<String, Vec<String>>,
    pub by_package: HashMap<String, Vec<String>>,
    pub by_tag: HashMap<String, HashSet<String>>,
    pub by_database_schema: HashMap<String, Vec<String>>,
    pub name_to_keys: HashMap<String, Vec<String>>,
    pub by_path_prefix: HashMap<String, Vec<String>>,

    // Test coverage
    pub tests_by_entity: HashMap<String, Vec<String>>,
    pub tests_by_column: HashMap<String, Vec<String>>,

    // Entity lookup/build support
    pub unique_id_to_resource_type: HashMap<String, String>,
    pub unique_id_to_path: HashMap<String, String>,
    pub unique_id_to_tag_strings: HashMap<String, Vec<String>>,

    // Discovery metadata
    pub entity_counts: HashMap<String, usize>,
    pub manifest_metadata_json: String,
}

/// Cached embeddings with invalidation metadata.
#[derive(Clone, Debug, rkyv_derive::Archive, rkyv_derive::Serialize, rkyv_derive::Deserialize)]
#[rkyv(bytecheck())]
pub struct CachedEmbeddings {
    pub schema_version: u32,
    pub model_name: String,
    pub manifest_hash: String,

    pub entity_ids: Vec<String>,
    pub dense_embeddings: Vec<Vec<f32>>,
    pub is_quantized: bool,

    pub sparse_indices: Option<Vec<Vec<u32>>>,
    pub sparse_values: Option<Vec<Vec<f32>>>,

    pub ann_hyperplanes: Option<Vec<Vec<f32>>>,
    pub ann_bucket_keys: Option<Vec<u64>>,
    pub ann_bucket_values: Option<Vec<Vec<usize>>>,
}

/// Cached sparse embeddings with invalidation metadata.
#[derive(Clone, Debug, rkyv_derive::Archive, rkyv_derive::Serialize, rkyv_derive::Deserialize)]
#[rkyv(bytecheck())]
pub struct CachedSparseEmbeddings {
    pub schema_version: u32,
    pub model_name: String,
    pub manifest_hash: String,

    pub entity_ids: Vec<String>,
    pub sparse_indices: Vec<Vec<usize>>,
    pub sparse_values: Vec<Vec<f32>>,
}
