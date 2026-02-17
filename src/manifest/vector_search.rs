#[cfg(feature = "embeddings")]
#[path = "embeddings/mod.rs"]
mod vector_search_embeddings;
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
