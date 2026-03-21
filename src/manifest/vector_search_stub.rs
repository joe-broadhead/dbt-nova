use crate::config::SearchConfig;
use crate::error::Result;
use crate::manifest::entity::{ArchivedEntity, Entity};
use crate::manifest::store::EntityStore;
use crate::manifest::vector_search::SearchComponentBuild;
use serde_json::Value as JsonValue;
use tracing::warn;

pub struct VectorSearcher;
pub struct SparseSearcher;
pub struct Reranker;

impl VectorSearcher {
    pub fn build(
        _store: &EntityStore,
        _config: &SearchConfig,
    ) -> Result<SearchComponentBuild<Self>> {
        if _config.enable_vector_search {
            let warning =
                "Vector search requested but embeddings feature is disabled; rebuild with --features embeddings"
                    .to_string();
            warn!(
                "Vector search requested but embeddings feature is disabled; rebuild with --features embeddings"
            );
            return Ok(SearchComponentBuild::disabled(warning));
        }
        Ok(SearchComponentBuild::unavailable())
    }

    pub fn search(&self, _query: &str, _top_k: usize) -> Result<Vec<(String, f32)>> {
        Ok(Vec::new())
    }
}

impl SparseSearcher {
    pub fn build(
        _store: &EntityStore,
        _config: &SearchConfig,
    ) -> Result<SearchComponentBuild<Self>> {
        if _config.enable_sparse_search {
            let warning =
                "Sparse search requested but embeddings feature is disabled; rebuild with --features embeddings"
                    .to_string();
            warn!(
                "Sparse search requested but embeddings feature is disabled; rebuild with --features embeddings"
            );
            return Ok(SearchComponentBuild::disabled(warning));
        }
        Ok(SearchComponentBuild::unavailable())
    }

    pub fn search(&self, _query: &str, _top_k: usize) -> Result<Vec<(String, f32)>> {
        Ok(Vec::new())
    }
}

impl Reranker {
    pub fn build(_config: &SearchConfig) -> Result<SearchComponentBuild<Self>> {
        if _config.enable_reranker {
            let warning =
                "Reranker requested but embeddings feature is disabled; rebuild with --features embeddings"
                    .to_string();
            warn!(
                "Reranker requested but embeddings feature is disabled; rebuild with --features embeddings"
            );
            return Ok(SearchComponentBuild::disabled(warning));
        }
        Ok(SearchComponentBuild::unavailable())
    }

    pub fn rerank(
        &self,
        _query: &str,
        _documents: &[String],
        _top_n: usize,
    ) -> Result<Vec<(usize, f32)>> {
        Ok(Vec::new())
    }
}

pub fn embedding_text(entity: &Entity, _config: &SearchConfig) -> String {
    entity.name.clone().unwrap_or_default()
}

pub fn embedding_text_from_entity(entity: &Entity, _config: &SearchConfig) -> String {
    embedding_text(entity, _config)
}

pub fn embedding_text_from_archived(entity: &ArchivedEntity, _config: &SearchConfig) -> String {
    entity.name_str().unwrap_or("").to_string()
}

pub fn embedding_text_from_payload(payload_json: &str, _config: &SearchConfig) -> String {
    let value: JsonValue = serde_json::from_str(payload_json).unwrap_or(JsonValue::Null);
    embedding_text_from_json(&value, _config)
}

pub fn embedding_text_from_json(entity_json: &JsonValue, _config: &SearchConfig) -> String {
    entity_json
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}
