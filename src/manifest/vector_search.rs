#[derive(Debug)]
pub struct SearchComponentBuild<T> {
    pub component: Option<T>,
    pub warning: Option<String>,
}

impl<T> SearchComponentBuild<T> {
    pub fn ready(component: T) -> Self {
        Self {
            component: Some(component),
            warning: None,
        }
    }

    pub fn ready_with_warning(component: T, warning: String) -> Self {
        Self {
            component: Some(component),
            warning: Some(warning),
        }
    }

    #[must_use]
    pub fn disabled(warning: String) -> Self {
        Self {
            component: None,
            warning: Some(warning),
        }
    }

    #[must_use]
    pub fn unavailable() -> Self {
        Self {
            component: None,
            warning: None,
        }
    }

    pub fn into_parts(self) -> (Option<T>, Option<String>) {
        (self.component, self.warning)
    }
}

#[cfg(feature = "embeddings")]
#[path = "embeddings/mod.rs"]
mod vector_search_embeddings;
#[cfg(feature = "embeddings")]
pub(crate) use vector_search_embeddings::{
    RequiredLocalModelLayout, required_embedding_model_layout, required_reranker_model_layout,
    required_sparse_model_layout,
};
#[cfg(feature = "embeddings")]
pub use vector_search_embeddings::{
    Reranker, SparseSearcher, VectorSearcher, embedding_text, embedding_text_from_archived,
    embedding_text_from_entity, embedding_text_from_json, embedding_text_from_payload,
};

#[cfg(not(feature = "embeddings"))]
#[path = "vector_search_stub.rs"]
mod vector_search_stub;
#[cfg(not(feature = "embeddings"))]
pub use vector_search_stub::{
    Reranker, SparseSearcher, VectorSearcher, embedding_text, embedding_text_from_archived,
    embedding_text_from_entity, embedding_text_from_json, embedding_text_from_payload,
};
