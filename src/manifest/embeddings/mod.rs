mod cache;
mod text;

use std::collections::{HashMap, HashSet};
use std::fmt::Display;
use std::panic::{AssertUnwindSafe, catch_unwind};

use fastembed::{
    EmbeddingModel, InitOptions, RerankInitOptions, RerankerModel, SparseEmbedding,
    SparseInitOptions, SparseTextEmbedding, TextEmbedding, TextRerank,
};

use crate::config::SearchConfig;
use crate::error::{DbtNovaError, Result};
use crate::manifest::rkyv_embeddings;
use crate::manifest::rkyv_sparse_embeddings;
use crate::manifest::rkyv_types::{CachedEmbeddings, CachedSparseEmbeddings};
use crate::manifest::store::EntityStore;
use tracing::warn;

use super::SearchComponentBuild;

const PROXY_KEYS: [&str; 6] = [
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
];

pub use text::{
    embedding_text, embedding_text_from_archived, embedding_text_from_entity,
    embedding_text_from_json, embedding_text_from_payload,
};

use cache::embeddings_cache_dir;

pub struct VectorSearcher {
    model: TextEmbedding,
    embeddings: DenseEmbeddings,
    ann: Option<AnnIndex>,
}

pub struct SparseSearcher {
    model: SparseTextEmbedding,
    embeddings: Vec<(String, SparseEmbedding)>,
}

pub struct Reranker {
    model: TextRerank,
}

fn panic_payload_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic".to_string()
    }
}

fn model_guidance(disable_env: &str) -> String {
    format!(
        "Ensure model files are available in DBT_NOVA_EMBEDDINGS_CACHE_DIR, pre-warm them with --warm-models, or publish a models artifact. If you do not need this component, set {disable_env}=false."
    )
}

fn component_init_warning(component_label: &str, disable_env: &str, cause: &str) -> String {
    format!(
        "{component_label} initialization failed; disabling {component_label} for this run. {} Cause: {cause}",
        model_guidance(disable_env)
    )
}

fn component_runtime_error(
    component_label: &str,
    action_label: &str,
    disable_env: &str,
    cause: &str,
) -> DbtNovaError {
    DbtNovaError::ServerError(format!(
        "{component_label} {action_label} failed. {} Cause: {cause}",
        model_guidance(disable_env)
    ))
}

fn disable_on_total_batch_failure<T>(
    component_label: &str,
    disable_env: &str,
    attempted_batches: usize,
    produced_items: usize,
    last_failure: Option<&str>,
) -> Option<SearchComponentBuild<T>> {
    if attempted_batches == 0 || produced_items > 0 {
        return None;
    }
    let last_failure = last_failure?;
    Some(SearchComponentBuild::disabled(component_init_warning(
        component_label,
        disable_env,
        &format!(
            "startup embedding generation failed for all batches. Last failure: {last_failure}"
        ),
    )))
}

fn build_optional_component<T, E, F>(
    component_label: &str,
    disable_env: &str,
    build: F,
) -> SearchComponentBuild<T>
where
    E: Display,
    F: FnOnce() -> std::result::Result<T, E>,
{
    match catch_unwind(AssertUnwindSafe(build)) {
        Ok(Ok(component)) => SearchComponentBuild::ready(component),
        Ok(Err(err)) => SearchComponentBuild::disabled(component_init_warning(
            component_label,
            disable_env,
            &err.to_string(),
        )),
        Err(payload) => SearchComponentBuild::disabled(component_init_warning(
            component_label,
            disable_env,
            &panic_payload_message(payload.as_ref()),
        )),
    }
}

fn run_component_operation<T, E, F>(
    component_label: &str,
    action_label: &str,
    disable_env: &str,
    operation: F,
) -> Result<T>
where
    E: Display,
    F: FnOnce() -> std::result::Result<T, E>,
{
    match catch_unwind(AssertUnwindSafe(operation)) {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(err)) => Err(component_runtime_error(
            component_label,
            action_label,
            disable_env,
            &err.to_string(),
        )),
        Err(payload) => Err(component_runtime_error(
            component_label,
            action_label,
            disable_env,
            &panic_payload_message(payload.as_ref()),
        )),
    }
}

enum DenseEmbeddings {
    Float(Vec<(String, Vec<f32>)>),
    Quantized(Vec<QuantizedEmbedding>),
}

struct QuantizedEmbedding {
    id: String,
    values: Vec<i8>,
}

struct AnnIndex {
    hyperplanes: Vec<Vec<f32>>,
    buckets: HashMap<u64, Vec<usize>>,
    bits: usize,
    max_candidates: usize,
    min_candidates: usize,
    hamming: usize,
}

impl AnnIndex {
    fn build_f32(embeddings: &[(String, Vec<f32>)], config: &SearchConfig) -> Option<Self> {
        if !config.enable_vector_ann || embeddings.is_empty() {
            return None;
        }

        let dim = embeddings.first().map_or(0, |(_, v)| v.len());
        if dim == 0 {
            return None;
        }

        let bits = config.vector_ann_bits.clamp(4, 63);
        let seed = random_hyperplane_seed();
        let hyperplanes = generate_hyperplanes(bits, dim, seed);
        let mut buckets: HashMap<u64, Vec<usize>> = HashMap::new();

        for (idx, (_, vec)) in embeddings.iter().enumerate() {
            let sig = signature_f32(vec, &hyperplanes);
            buckets.entry(sig).or_default().push(idx);
        }

        Some(Self {
            hyperplanes,
            buckets,
            bits,
            max_candidates: config.vector_ann_max_candidates.max(1),
            min_candidates: config.vector_ann_min_candidates,
            hamming: config.vector_ann_hamming.min(bits),
        })
    }

    fn build_quantized(embeddings: &[QuantizedEmbedding], config: &SearchConfig) -> Option<Self> {
        if !config.enable_vector_ann || embeddings.is_empty() {
            return None;
        }

        let dim = embeddings.first().map_or(0, |e| e.values.len());
        if dim == 0 {
            return None;
        }

        let bits = config.vector_ann_bits.clamp(4, 63);
        let seed = random_hyperplane_seed();
        let hyperplanes = generate_hyperplanes(bits, dim, seed);
        let mut buckets: HashMap<u64, Vec<usize>> = HashMap::new();

        for (idx, embedding) in embeddings.iter().enumerate() {
            let sig = signature_i8(&embedding.values, &hyperplanes);
            buckets.entry(sig).or_default().push(idx);
        }

        Some(Self {
            hyperplanes,
            buckets,
            bits,
            max_candidates: config.vector_ann_max_candidates.max(1),
            min_candidates: config.vector_ann_min_candidates,
            hamming: config.vector_ann_hamming.min(bits),
        })
    }

    fn candidates_f32(&self, query_vec: &[f32], top_k: usize) -> Option<Vec<usize>> {
        let sig = signature_f32(query_vec, &self.hyperplanes);
        let mut seen: HashSet<usize> = HashSet::new();
        let mut candidates: Vec<usize> = Vec::new();

        let push_bucket =
            |bucket: u64, candidates: &mut Vec<usize>, seen: &mut HashSet<usize>| -> bool {
                if let Some(ids) = self.buckets.get(&bucket) {
                    for id in ids {
                        if seen.insert(*id) {
                            candidates.push(*id);
                            if candidates.len() >= self.max_candidates {
                                return true;
                            }
                        }
                    }
                }
                false
            };

        if push_bucket(sig, &mut candidates, &mut seen) {
            return Some(candidates);
        }

        if self.hamming >= 1 {
            for bit in 0..self.bits {
                if push_bucket(sig ^ (1u64 << bit), &mut candidates, &mut seen) {
                    return Some(candidates);
                }
            }
        }

        if self.hamming >= 2 {
            for i in 0..self.bits {
                for j in (i + 1)..self.bits {
                    if push_bucket(sig ^ (1u64 << i) ^ (1u64 << j), &mut candidates, &mut seen) {
                        return Some(candidates);
                    }
                }
            }
        }

        let min_needed = self.min_candidates.max(top_k.saturating_mul(4));
        if candidates.len() < min_needed {
            return None;
        }

        Some(candidates)
    }

    fn candidates_i8(&self, query_vec: &[i8], top_k: usize) -> Option<Vec<usize>> {
        let sig = signature_i8(query_vec, &self.hyperplanes);
        let mut seen: HashSet<usize> = HashSet::new();
        let mut candidates: Vec<usize> = Vec::new();

        let push_bucket =
            |bucket: u64, candidates: &mut Vec<usize>, seen: &mut HashSet<usize>| -> bool {
                if let Some(ids) = self.buckets.get(&bucket) {
                    for id in ids {
                        if seen.insert(*id) {
                            candidates.push(*id);
                            if candidates.len() >= self.max_candidates {
                                return true;
                            }
                        }
                    }
                }
                false
            };

        if push_bucket(sig, &mut candidates, &mut seen) {
            return Some(candidates);
        }

        if self.hamming >= 1 {
            for bit in 0..self.bits {
                if push_bucket(sig ^ (1u64 << bit), &mut candidates, &mut seen) {
                    return Some(candidates);
                }
            }
        }

        if self.hamming >= 2 {
            for i in 0..self.bits {
                for j in (i + 1)..self.bits {
                    if push_bucket(sig ^ (1u64 << i) ^ (1u64 << j), &mut candidates, &mut seen) {
                        return Some(candidates);
                    }
                }
            }
        }

        let min_needed = self.min_candidates.max(top_k.saturating_mul(4));
        if candidates.len() < min_needed {
            return None;
        }

        Some(candidates)
    }
}

impl VectorSearcher {
    /// Build the vector search index and embeddings.
    ///
    /// # Errors
    /// Returns an error if embeddings cannot be generated or cached.
    #[allow(clippy::too_many_lines)]
    pub fn build(store: &EntityStore, config: &SearchConfig) -> Result<SearchComponentBuild<Self>> {
        if !config.enable_vector_search {
            return Ok(SearchComponentBuild::unavailable());
        }

        validate_proxy_env_vars()?;
        let cache_dir = embeddings_cache_dir(config);

        let (model, warning) =
            build_optional_component("vector search", "DBT_NOVA_SEARCH_ENABLE_VECTOR", || {
                TextEmbedding::try_new(InitOptions {
                    model_name: resolve_embedding_model(&config.embedding_model),
                    cache_dir: cache_dir.clone(),
                    show_download_progress: false,
                    ..Default::default()
                })
            })
            .into_parts();
        let Some(model) = model else {
            if let Some(warning) = warning {
                warn!(warning = %warning, "Vector search unavailable during startup");
                return Ok(SearchComponentBuild::disabled(warning));
            }
            return Ok(SearchComponentBuild::unavailable());
        };

        let batch_size = config.embedding_batch_size.max(1);
        let use_quant = config.enable_vector_quantization;
        let manifest_hash = config.manifest_hash.as_deref().unwrap_or("");
        let model_name = resolve_embedding_model(&config.embedding_model).to_string();

        if let Some(cached) = rkyv_embeddings::try_load_embeddings(
            &cache_dir,
            &model_name,
            manifest_hash,
            config.embeddings_max_decompressed_bytes,
        ) {
            let embeddings = if cached.is_quantized {
                let quantized = cached
                    .entity_ids
                    .into_iter()
                    .zip(cached.dense_embeddings)
                    .map(|(id, values)| {
                        let values = quantize_vector(&values);
                        QuantizedEmbedding { id, values }
                    })
                    .collect::<Vec<_>>();
                DenseEmbeddings::Quantized(quantized)
            } else {
                DenseEmbeddings::Float(
                    cached
                        .entity_ids
                        .into_iter()
                        .zip(cached.dense_embeddings)
                        .collect(),
                )
            };

            let ann = match &embeddings {
                DenseEmbeddings::Float(values) => AnnIndex::build_f32(values, config),
                DenseEmbeddings::Quantized(values) => AnnIndex::build_quantized(values, config),
            };

            return Ok(SearchComponentBuild::ready(Self {
                model,
                embeddings,
                ann,
            }));
        }
        let mut embeddings_f32: Vec<(String, Vec<f32>)> = Vec::new();
        let mut embeddings_i8: Vec<QuantizedEmbedding> = Vec::new();
        let mut batch_ids: Vec<String> = Vec::with_capacity(batch_size);
        let mut batch_texts: Vec<String> = Vec::with_capacity(batch_size);
        let mut attempted_batches = 0usize;
        let mut last_batch_failure: Option<String> = None;

        let mut flush_batch =
            |batch_ids: &mut Vec<String>, batch_texts: &mut Vec<String>| -> Result<()> {
                if batch_texts.is_empty() {
                    return Ok(());
                }
                attempted_batches += 1;

                let texts = std::mem::take(batch_texts);
                let ids = std::mem::take(batch_ids);

                let mut vecs = match run_component_operation(
                    "vector search",
                    "embedding batch generation",
                    "DBT_NOVA_SEARCH_ENABLE_VECTOR",
                    || model.embed(texts, Some(batch_size)),
                ) {
                    Ok(vecs) => vecs,
                    Err(err) => {
                        last_batch_failure = Some(err.to_string());
                        warn!(error = %err, "Vector embedding batch failed; skipping batch");
                        return Ok(());
                    }
                };

                if vecs.len() != ids.len() {
                    last_batch_failure = Some(format!(
                        "embedding batch returned unexpected size; expected {}, actual {}",
                        ids.len(),
                        vecs.len()
                    ));
                    warn!(
                        expected = ids.len(),
                        actual = vecs.len(),
                        "Vector embedding batch returned unexpected size; skipping batch"
                    );
                    return Ok(());
                }

                for (id, mut vec) in ids.into_iter().zip(vecs.drain(..)) {
                    normalize_vector(&mut vec);
                    if use_quant {
                        let quant = quantize_vector(&vec);
                        embeddings_i8.push(QuantizedEmbedding { id, values: quant });
                    } else {
                        embeddings_f32.push((id, vec));
                    }
                }

                batch_ids.reserve(batch_size);
                batch_texts.reserve(batch_size);
                Ok(())
            };

        for unique_id in store.ids() {
            if let Some(entity) = store.get_archived(unique_id)? {
                let text = embedding_text_from_archived(entity, config);
                if text.is_empty() {
                    continue;
                }
                batch_ids.push(unique_id.clone());
                batch_texts.push(text);

                if batch_texts.len() >= batch_size {
                    flush_batch(&mut batch_ids, &mut batch_texts)?;
                }
            }
        }

        flush_batch(&mut batch_ids, &mut batch_texts)?;

        let produced_items = if use_quant {
            embeddings_i8.len()
        } else {
            embeddings_f32.len()
        };
        if let Some(disabled) = disable_on_total_batch_failure(
            "vector search",
            "DBT_NOVA_SEARCH_ENABLE_VECTOR",
            attempted_batches,
            produced_items,
            last_batch_failure.as_deref(),
        ) {
            if let Some(warning) = disabled.warning.as_ref() {
                warn!(warning = %warning, "Vector search unavailable during startup");
            }
            return Ok(disabled);
        }

        let (embeddings, ann) = if use_quant {
            let ann = AnnIndex::build_quantized(&embeddings_i8, config);
            (DenseEmbeddings::Quantized(embeddings_i8), ann)
        } else {
            let ann = AnnIndex::build_f32(&embeddings_f32, config);
            (DenseEmbeddings::Float(embeddings_f32), ann)
        };

        let mut cache_ids = Vec::new();
        let mut cache_vectors = Vec::new();
        let mut is_quantized = false;
        match &embeddings {
            DenseEmbeddings::Float(values) => {
                for (id, vec) in values {
                    cache_ids.push(id.clone());
                    cache_vectors.push(vec.clone());
                }
            }
            DenseEmbeddings::Quantized(values) => {
                is_quantized = true;
                for embedding in values {
                    cache_ids.push(embedding.id.clone());
                    cache_vectors.push(
                        embedding
                            .values
                            .iter()
                            .map(|v| f32::from(*v))
                            .collect::<Vec<f32>>(),
                    );
                }
            }
        }

        let cache = CachedEmbeddings {
            schema_version: crate::manifest::rkyv_types::RKYV_SCHEMA_VERSION,
            model_name,
            manifest_hash: manifest_hash.to_string(),
            entity_ids: cache_ids,
            dense_embeddings: cache_vectors,
            is_quantized,
            sparse_indices: None,
            sparse_values: None,
            ann_hyperplanes: None,
            ann_bucket_keys: None,
            ann_bucket_values: None,
        };
        if let Err(err) = rkyv_embeddings::save_embeddings(&cache, &cache_dir) {
            warn!(error = %err, "failed to save embeddings cache");
        }

        Ok(SearchComponentBuild::ready(Self {
            model,
            embeddings,
            ann,
        }))
    }

    /// Search the dense embeddings for the nearest neighbors.
    ///
    /// # Errors
    /// Returns an error if embedding generation fails.
    pub fn search(&self, query: &str, top_k: usize) -> Result<Vec<(String, f32)>> {
        if query.trim().is_empty() {
            return Ok(vec![]);
        }
        let mut vecs = run_component_operation(
            "vector search",
            "embedding generation",
            "DBT_NOVA_SEARCH_ENABLE_VECTOR",
            || self.model.embed(vec![query.to_string()], None),
        )?;
        let mut query_vec = vecs.pop().unwrap_or_default();
        if query_vec.is_empty() {
            return Ok(vec![]);
        }
        normalize_vector(&mut query_vec);

        match &self.embeddings {
            DenseEmbeddings::Float(embeddings) => {
                let scored = if let Some(ann) = &self.ann {
                    if let Some(candidates) = ann.candidates_f32(&query_vec, top_k) {
                        candidates
                            .into_iter()
                            .filter_map(|idx| {
                                embeddings
                                    .get(idx)
                                    .map(|(_, vec)| (idx, dot_product(&query_vec, vec)))
                            })
                            .collect()
                    } else {
                        embeddings
                            .iter()
                            .enumerate()
                            .map(|(idx, (_, vec))| (idx, dot_product(&query_vec, vec)))
                            .collect()
                    }
                } else {
                    embeddings
                        .iter()
                        .enumerate()
                        .map(|(idx, (_, vec))| (idx, dot_product(&query_vec, vec)))
                        .collect()
                };
                let scored = top_k_scored(scored, top_k);
                let mut results = Vec::with_capacity(scored.len());
                for (idx, score) in scored {
                    if let Some((id, _)) = embeddings.get(idx) {
                        results.push((id.clone(), score));
                    }
                }
                Ok(results)
            }
            DenseEmbeddings::Quantized(embeddings) => {
                let query_i8 = quantize_vector(&query_vec);
                let scored = if let Some(ann) = &self.ann {
                    if let Some(candidates) = ann.candidates_i8(&query_i8, top_k) {
                        candidates
                            .into_iter()
                            .filter_map(|idx| {
                                embeddings
                                    .get(idx)
                                    .map(|e| (idx, dot_product_i8(&query_i8, &e.values)))
                            })
                            .collect()
                    } else {
                        embeddings
                            .iter()
                            .enumerate()
                            .map(|(idx, e)| (idx, dot_product_i8(&query_i8, &e.values)))
                            .collect()
                    }
                } else {
                    embeddings
                        .iter()
                        .enumerate()
                        .map(|(idx, e)| (idx, dot_product_i8(&query_i8, &e.values)))
                        .collect()
                };
                let scored = top_k_scored(scored, top_k);
                let mut results = Vec::with_capacity(scored.len());
                for (idx, score) in scored {
                    if let Some(entry) = embeddings.get(idx) {
                        results.push((entry.id.clone(), score));
                    }
                }
                Ok(results)
            }
        }
    }
}

impl SparseSearcher {
    /// Build the sparse search index.
    ///
    /// # Errors
    /// Returns an error if sparse embeddings cannot be generated or cached.
    #[allow(clippy::too_many_lines)]
    pub fn build(store: &EntityStore, config: &SearchConfig) -> Result<SearchComponentBuild<Self>> {
        if !config.enable_sparse_search {
            return Ok(SearchComponentBuild::unavailable());
        }

        validate_proxy_env_vars()?;
        let cache_dir = embeddings_cache_dir(config);
        let (model, warning) =
            build_optional_component("sparse search", "DBT_NOVA_SEARCH_ENABLE_SPARSE", || {
                SparseTextEmbedding::try_new(SparseInitOptions {
                    cache_dir: cache_dir.clone(),
                    show_download_progress: false,
                    ..Default::default()
                })
            })
            .into_parts();
        let Some(model) = model else {
            if let Some(warning) = warning {
                warn!(warning = %warning, "Sparse search unavailable during startup");
                return Ok(SearchComponentBuild::disabled(warning));
            }
            return Ok(SearchComponentBuild::unavailable());
        };

        let batch_size = config.embedding_batch_size.max(1);
        let manifest_hash = config.manifest_hash.as_deref().unwrap_or("");
        let sparse_model_name = SparseInitOptions::default().model_name.to_string();

        if let Some(cached) = rkyv_sparse_embeddings::try_load_sparse_embeddings(
            &cache_dir,
            &sparse_model_name,
            manifest_hash,
            config.embeddings_max_decompressed_bytes,
        ) {
            let embeddings = cached
                .entity_ids
                .into_iter()
                .zip(cached.sparse_indices.into_iter().zip(cached.sparse_values))
                .map(|(id, (indices, values))| (id, SparseEmbedding { indices, values }))
                .collect::<Vec<_>>();
            return Ok(SearchComponentBuild::ready(Self { model, embeddings }));
        }
        let mut embeddings: Vec<(String, SparseEmbedding)> = Vec::new();
        let mut batch_ids: Vec<String> = Vec::with_capacity(batch_size);
        let mut batch_texts: Vec<String> = Vec::with_capacity(batch_size);
        let mut attempted_batches = 0usize;
        let mut last_batch_failure: Option<String> = None;

        let mut flush_batch =
            |batch_ids: &mut Vec<String>, batch_texts: &mut Vec<String>| -> Result<()> {
                if batch_texts.is_empty() {
                    return Ok(());
                }
                attempted_batches += 1;

                let texts = std::mem::take(batch_texts);
                let ids = std::mem::take(batch_ids);

                let mut vecs = match run_component_operation(
                    "sparse search",
                    "embedding batch generation",
                    "DBT_NOVA_SEARCH_ENABLE_SPARSE",
                    || model.embed(texts, Some(batch_size)),
                ) {
                    Ok(vecs) => vecs,
                    Err(err) => {
                        last_batch_failure = Some(err.to_string());
                        warn!(error = %err, "Sparse embedding batch failed; skipping batch");
                        return Ok(());
                    }
                };

                if vecs.len() != ids.len() {
                    last_batch_failure = Some(format!(
                        "embedding batch returned unexpected size; expected {}, actual {}",
                        ids.len(),
                        vecs.len()
                    ));
                    warn!(
                        expected = ids.len(),
                        actual = vecs.len(),
                        "Sparse embedding batch returned unexpected size; skipping batch"
                    );
                    return Ok(());
                }

                for (id, vec) in ids.into_iter().zip(vecs.drain(..)) {
                    embeddings.push((id, vec));
                }

                batch_ids.reserve(batch_size);
                batch_texts.reserve(batch_size);
                Ok(())
            };

        for unique_id in store.ids() {
            if let Some(entity) = store.get_archived(unique_id)? {
                let text = embedding_text_from_archived(entity, config);
                if text.is_empty() {
                    continue;
                }
                batch_ids.push(unique_id.clone());
                batch_texts.push(text);

                if batch_texts.len() >= batch_size {
                    flush_batch(&mut batch_ids, &mut batch_texts)?;
                }
            }
        }

        flush_batch(&mut batch_ids, &mut batch_texts)?;

        if let Some(disabled) = disable_on_total_batch_failure(
            "sparse search",
            "DBT_NOVA_SEARCH_ENABLE_SPARSE",
            attempted_batches,
            embeddings.len(),
            last_batch_failure.as_deref(),
        ) {
            if let Some(warning) = disabled.warning.as_ref() {
                warn!(warning = %warning, "Sparse search unavailable during startup");
            }
            return Ok(disabled);
        }

        let mut cache_ids = Vec::with_capacity(embeddings.len());
        let mut cache_indices = Vec::with_capacity(embeddings.len());
        let mut cache_values = Vec::with_capacity(embeddings.len());
        for (id, embedding) in &embeddings {
            cache_ids.push(id.clone());
            cache_indices.push(embedding.indices.clone());
            cache_values.push(embedding.values.clone());
        }

        let cache = CachedSparseEmbeddings {
            schema_version: crate::manifest::rkyv_types::RKYV_SCHEMA_VERSION,
            model_name: sparse_model_name,
            manifest_hash: manifest_hash.to_string(),
            entity_ids: cache_ids,
            sparse_indices: cache_indices,
            sparse_values: cache_values,
        };
        if let Err(err) = rkyv_sparse_embeddings::save_sparse_embeddings(&cache, &cache_dir) {
            warn!(error = %err, "failed to save sparse embeddings cache");
        }

        Ok(SearchComponentBuild::ready(Self { model, embeddings }))
    }

    /// Search the sparse embeddings for the nearest neighbors.
    ///
    /// # Errors
    /// Returns an error if embedding generation fails.
    pub fn search(&self, query: &str, top_k: usize) -> Result<Vec<(String, f32)>> {
        if query.trim().is_empty() {
            return Ok(vec![]);
        }
        let mut vecs = run_component_operation(
            "sparse search",
            "embedding generation",
            "DBT_NOVA_SEARCH_ENABLE_SPARSE",
            || self.model.embed(vec![query.to_string()], None),
        )?;
        let query_vec = vecs.pop().ok_or_else(|| {
            DbtNovaError::ServerError("Sparse embedding returned empty vector".to_string())
        })?;

        let scored: Vec<(usize, f32)> = self
            .embeddings
            .iter()
            .enumerate()
            .map(|(idx, (_, vec))| (idx, sparse_dot(&query_vec, vec)))
            .collect();
        let scored = top_k_scored(scored, top_k);

        let mut results = Vec::with_capacity(scored.len());
        for (idx, score) in scored {
            if let Some((id, _)) = self.embeddings.get(idx) {
                results.push((id.clone(), score));
            }
        }
        Ok(results)
    }
}

impl Reranker {
    /// Build the reranker model.
    ///
    /// # Errors
    /// Returns an error if the reranker model cannot be initialized.
    pub fn build(config: &SearchConfig) -> Result<SearchComponentBuild<Self>> {
        if !config.enable_reranker {
            return Ok(SearchComponentBuild::unavailable());
        }
        validate_proxy_env_vars()?;
        let cache_dir = embeddings_cache_dir(config);
        let model_name = resolve_reranker_model(&config.reranker_model);
        let (model, warning) =
            build_optional_component("reranker", "DBT_NOVA_SEARCH_ENABLE_RERANKER", || {
                TextRerank::try_new(RerankInitOptions {
                    model_name,
                    cache_dir,
                    show_download_progress: false,
                    ..Default::default()
                })
            })
            .into_parts();
        let Some(model) = model else {
            if let Some(warning) = warning {
                warn!(warning = %warning, "Reranker unavailable during startup");
                return Ok(SearchComponentBuild::disabled(warning));
            }
            return Ok(SearchComponentBuild::unavailable());
        };
        Ok(SearchComponentBuild::ready(Self { model }))
    }

    /// Rerank candidate documents using the configured model.
    ///
    /// # Errors
    /// Returns an error if the reranker fails to score the documents.
    pub fn rerank(
        &self,
        query: &str,
        documents: &[String],
        top_n: usize,
    ) -> Result<Vec<(usize, f32)>> {
        if documents.is_empty() {
            return Ok(vec![]);
        }
        let docs: Vec<&str> = documents.iter().map(String::as_str).collect();
        let results = run_component_operation(
            "reranker",
            "document scoring",
            "DBT_NOVA_SEARCH_ENABLE_RERANKER",
            || self.model.rerank(query, docs, false, None),
        )?;

        let mut scored: Vec<(usize, f32)> =
            results.into_iter().map(|r| (r.index, r.score)).collect();
        scored.sort_by(score_desc_cmp);
        scored.truncate(top_n);
        Ok(scored)
    }
}

fn validate_proxy_env_vars() -> Result<()> {
    for key in PROXY_KEYS {
        let Some(value) = std::env::var_os(key) else {
            continue;
        };
        let trimmed = value.to_string_lossy().trim().to_string();
        if trimmed.is_empty() {
            continue;
        }
        if !trimmed.contains("://") {
            return Err(DbtNovaError::ServerError(format!(
                "Invalid proxy environment variable '{key}'. Expected an absolute URL such as 'http://proxy.internal:8080'."
            )));
        }
    }
    Ok(())
}

fn resolve_reranker_model(value: &str) -> RerankerModel {
    let normalized = value.trim().to_lowercase();
    match normalized.as_str() {
        "jina-reranker-v1-turbo-en"
        | "jinaai/jina-reranker-v1-turbo-en"
        | "jina-v1-turbo-en"
        | "jina-turbo"
        | "turbo" => RerankerModel::JINARerankerV1TurboEn,
        "jina-reranker-v2-base-multilingual"
        | "jinaai/jina-reranker-v2-base-multilingual"
        | "jina-v2-multilingual"
        | "jina-multilingual"
        | "multilingual" => RerankerModel::JINARerankerV2BaseMultiligual,
        "bge-reranker-base" | "baai/bge-reranker-base" | "bge-base" | "bge" => {
            RerankerModel::BGERerankerBase
        }
        _ => {
            warn!(
                value = %value,
                "Unknown reranker model; defaulting to jina-reranker-v2-base-multilingual"
            );
            RerankerModel::JINARerankerV2BaseMultiligual
        }
    }
}

fn resolve_embedding_model(value: &str) -> EmbeddingModel {
    let normalized = value.trim().to_lowercase();
    match normalized.as_str() {
        "multilingual-e5-base"
        | "intfloat/multilingual-e5-base"
        | "e5-base"
        | "e5"
        | "multilingual" => EmbeddingModel::MultilingualE5Base,
        "multilingual-e5-large" | "intfloat/multilingual-e5-large" | "e5-large" => {
            EmbeddingModel::MultilingualE5Large
        }
        "multilingual-e5-small" | "intfloat/multilingual-e5-small" | "e5-small" => {
            EmbeddingModel::MultilingualE5Small
        }
        "bge-small-en-v1.5" | "baai/bge-small-en-v1.5" | "bge-small" => {
            EmbeddingModel::BGESmallENV15
        }
        "bge-base-en-v1.5" | "baai/bge-base-en-v1.5" | "bge-base" => EmbeddingModel::BGEBaseENV15,
        "bge-large-en-v1.5" | "baai/bge-large-en-v1.5" | "bge-large" => {
            EmbeddingModel::BGELargeENV15
        }
        "all-minilm-l6-v2" | "allminilm-l6-v2" | "minilm-l6" | "minilm" => {
            EmbeddingModel::AllMiniLML6V2
        }
        "all-minilm-l12-v2" | "allminilm-l12-v2" | "minilm-l12" => EmbeddingModel::AllMiniLML12V2,
        "nomic-embed-text-v1" | "nomic-ai/nomic-embed-text-v1" | "nomic-v1" => {
            EmbeddingModel::NomicEmbedTextV1
        }
        "nomic-embed-text-v1.5" | "nomic-ai/nomic-embed-text-v1.5" | "nomic-v1.5" | "nomic" => {
            EmbeddingModel::NomicEmbedTextV15
        }
        "mxbai-embed-large-v1" | "mixedbread-ai/mxbai-embed-large-v1" | "mxbai-large" => {
            EmbeddingModel::MxbaiEmbedLargeV1
        }
        "gte-base-en-v1.5" | "alibaba-nlp/gte-base-en-v1.5" | "gte-base" => {
            EmbeddingModel::GTEBaseENV15
        }
        "gte-large-en-v1.5" | "alibaba-nlp/gte-large-en-v1.5" | "gte-large" => {
            EmbeddingModel::GTELargeENV15
        }
        _ => {
            warn!(
                value = %value,
                "Unknown embedding model; defaulting to multilingual-e5-base"
            );
            EmbeddingModel::MultilingualE5Base
        }
    }
}

fn normalize_vector(vec: &mut [f32]) {
    let norm = vec.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 0.0 {
        for v in vec.iter_mut() {
            *v /= norm;
        }
    }
}

fn random_hyperplane_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| {
            d.as_secs()
                ^ u64::from(d.subsec_nanos())
                ^ u64::from(std::process::id()).rotate_left(13)
        })
        .unwrap_or(0x9E37_79B9_7F4A_7C15)
}

fn generate_hyperplanes(bits: usize, dim: usize, seed: u64) -> Vec<Vec<f32>> {
    let mut rng = XorShift64::new(seed);
    let mut planes = Vec::with_capacity(bits);
    for _ in 0..bits {
        let mut vec = Vec::with_capacity(dim);
        for _ in 0..dim {
            vec.push(rng.next_f32());
        }
        normalize_vector(&mut vec);
        planes.push(vec);
    }
    planes
}

struct XorShift64 {
    state: u64,
}

impl XorShift64 {
    fn new(seed: u64) -> Self {
        let seed = if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        };
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
    fn next_f32(&mut self) -> f32 {
        let value = self.next_u64();
        let unit = (value as f64) / (u64::MAX as f64);
        (unit * 2.0 - 1.0) as f32
    }
}

const QUANT_SCALE: f32 = 1.0 / 127.0;

#[allow(clippy::cast_possible_truncation)]
fn quantize_vector(vec: &[f32]) -> Vec<i8> {
    vec.iter()
        .map(|v| (v * 127.0).round().clamp(-127.0, 127.0) as i8)
        .collect()
}

fn top_k_scored(mut scored: Vec<(usize, f32)>, top_k: usize) -> Vec<(usize, f32)> {
    if top_k == 0 || scored.is_empty() {
        return Vec::new();
    }
    if scored.len() > top_k {
        scored.select_nth_unstable_by(top_k, score_desc_cmp);
        scored.truncate(top_k);
    }
    scored.sort_by(score_desc_cmp);
    scored
}

fn score_desc_cmp(a: &(usize, f32), b: &(usize, f32)) -> std::cmp::Ordering {
    let score_a = if a.1.is_finite() {
        a.1
    } else {
        f32::NEG_INFINITY
    };
    let score_b = if b.1.is_finite() {
        b.1
    } else {
        f32::NEG_INFINITY
    };
    match score_b.total_cmp(&score_a) {
        std::cmp::Ordering::Equal => a.0.cmp(&b.0),
        other => other,
    }
}

#[inline]
fn dot_product(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    #[cfg(target_arch = "x86_64")]
    {
        if std::is_x86_feature_detected!("avx2") {
            // SAFETY: `a` and `b` are valid slices; we only read up to `min(len)` and use
            // unaligned loads (`*_loadu_ps`), so alignment is not required.
            unsafe { return dot_product_avx2(a, b) };
        }
        if std::is_x86_feature_detected!("sse") {
            // SAFETY: `a` and `b` are valid slices; we only read up to `min(len)` and use
            // unaligned loads (`*_loadu_ps`), so alignment is not required.
            unsafe { return dot_product_sse(a, b) };
        }
    }
    dot_product_scalar(a, b)
}

#[inline]
fn dot_product_scalar(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

#[allow(clippy::cast_precision_loss)]
fn dot_product_i8(a: &[i8], b: &[i8]) -> f32 {
    let mut acc: i32 = 0;
    let len = a.len().min(b.len());
    for i in 0..len {
        acc += i32::from(a[i]) * i32::from(b[i]);
    }
    (acc as f32) * (QUANT_SCALE * QUANT_SCALE)
}

#[cfg(target_arch = "x86_64")]
#[inline]
unsafe fn dot_product_avx2(a: &[f32], b: &[f32]) -> f32 {
    // SAFETY: `a` and `b` are valid for at least `len` elements. The loop bounds ensure
    // `_mm256_loadu_ps` reads within the slice, and the remainder loop handles any tail.
    use std::arch::x86_64::{
        _mm256_add_ps, _mm256_loadu_ps, _mm256_mul_ps, _mm256_setzero_ps, _mm256_storeu_ps,
    };
    let len = a.len().min(b.len());
    // SAFETY: caller guarantees AVX2 support before calling this function.
    let mut sum = unsafe { _mm256_setzero_ps() };
    let mut i = 0usize;
    while i + 8 <= len {
        // SAFETY: `i + 8 <= len` guarantees each 8-f32 unaligned load is in-bounds.
        let va = unsafe { _mm256_loadu_ps(a[i..].as_ptr()) };
        // SAFETY: `i + 8 <= len` guarantees each 8-f32 unaligned load is in-bounds.
        let vb = unsafe { _mm256_loadu_ps(b[i..].as_ptr()) };
        // SAFETY: AVX2 support is guaranteed by caller.
        let prod = unsafe { _mm256_mul_ps(va, vb) };
        // SAFETY: AVX2 support is guaranteed by caller.
        sum = unsafe { _mm256_add_ps(sum, prod) };
        i += 8;
    }
    let mut tmp = [0f32; 8];
    // SAFETY: `tmp` has capacity for 8 f32 values.
    unsafe { _mm256_storeu_ps(tmp.as_mut_ptr(), sum) };
    let mut total = tmp.iter().sum::<f32>();
    for j in i..len {
        total += a[j] * b[j];
    }
    total
}

#[cfg(target_arch = "x86_64")]
#[inline]
unsafe fn dot_product_sse(a: &[f32], b: &[f32]) -> f32 {
    // SAFETY: `a` and `b` are valid for at least `len` elements. The loop bounds ensure
    // `_mm_loadu_ps` reads within the slice, and the remainder loop handles any tail.
    use std::arch::x86_64::{_mm_add_ps, _mm_loadu_ps, _mm_mul_ps, _mm_setzero_ps, _mm_storeu_ps};
    let len = a.len().min(b.len());
    // SAFETY: caller guarantees SSE support before calling this function.
    let mut sum = unsafe { _mm_setzero_ps() };
    let mut i = 0usize;
    while i + 4 <= len {
        // SAFETY: `i + 4 <= len` guarantees each 4-f32 unaligned load is in-bounds.
        let va = unsafe { _mm_loadu_ps(a[i..].as_ptr()) };
        // SAFETY: `i + 4 <= len` guarantees each 4-f32 unaligned load is in-bounds.
        let vb = unsafe { _mm_loadu_ps(b[i..].as_ptr()) };
        // SAFETY: SSE support is guaranteed by caller.
        let prod = unsafe { _mm_mul_ps(va, vb) };
        // SAFETY: SSE support is guaranteed by caller.
        sum = unsafe { _mm_add_ps(sum, prod) };
        i += 4;
    }
    let mut tmp = [0f32; 4];
    // SAFETY: `tmp` has capacity for 4 f32 values.
    unsafe { _mm_storeu_ps(tmp.as_mut_ptr(), sum) };
    let mut total = tmp.iter().sum::<f32>();
    for j in i..len {
        total += a[j] * b[j];
    }
    total
}

fn signature_f32(vec: &[f32], hyperplanes: &[Vec<f32>]) -> u64 {
    let mut sig = 0u64;
    for (i, plane) in hyperplanes.iter().enumerate() {
        let dot = dot_product(vec, plane);
        if dot >= 0.0 {
            sig |= 1u64 << i;
        }
    }
    sig
}

fn signature_i8(vec: &[i8], hyperplanes: &[Vec<f32>]) -> u64 {
    let mut sig = 0u64;
    for (i, plane) in hyperplanes.iter().enumerate() {
        let mut dot = 0.0f32;
        let len = vec.len().min(plane.len());
        for j in 0..len {
            dot += f32::from(vec[j]) * QUANT_SCALE * plane[j];
        }
        if dot >= 0.0 {
            sig |= 1u64 << i;
        }
    }
    sig
}

fn sparse_dot(query: &SparseEmbedding, doc: &SparseEmbedding) -> f32 {
    let mut score = 0.0f32;
    let mut qi = 0usize;
    let mut di = 0usize;

    while qi < query.indices.len() && di < doc.indices.len() {
        let q_idx = query.indices[qi];
        let d_idx = doc.indices[di];
        match q_idx.cmp(&d_idx) {
            std::cmp::Ordering::Equal => {
                score += query.values[qi] * doc.values[di];
                qi += 1;
                di += 1;
            }
            std::cmp::Ordering::Less => {
                qi += 1;
            }
            std::cmp::Ordering::Greater => {
                di += 1;
            }
        }
    }

    score
}

#[cfg(test)]
mod tests {
    use std::sync::{LazyLock, Mutex};

    use super::{
        build_optional_component, disable_on_total_batch_failure, run_component_operation,
        validate_proxy_env_vars,
    };

    static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    fn with_proxy_env<F>(key: &str, value: Option<&str>, f: F)
    where
        F: FnOnce(),
    {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let original = std::env::var_os(key);
        unsafe {
            match value {
                Some(next) => std::env::set_var(key, next),
                None => std::env::remove_var(key),
            }
        }
        f();
        unsafe {
            match original {
                Some(previous) => std::env::set_var(key, previous),
                None => std::env::remove_var(key),
            }
        }
    }

    #[test]
    fn proxy_validation_accepts_absolute_url() {
        with_proxy_env("HTTPS_PROXY", Some("http://proxy.internal:8080"), || {
            let result = validate_proxy_env_vars();
            assert!(result.is_ok());
        });
    }

    #[test]
    fn proxy_validation_rejects_non_url_values() {
        with_proxy_env("HTTPS_PROXY", Some("proxy.internal:8080"), || {
            let result = validate_proxy_env_vars();
            assert!(result.is_err());
            let error_text = result
                .expect_err("invalid proxy should return error")
                .to_string();
            assert!(error_text.contains("Invalid proxy environment variable 'HTTPS_PROXY'"));
        });
    }

    #[test]
    fn build_optional_component_converts_panics_into_disabled_warnings() {
        let result = build_optional_component::<(), String, _>(
            "vector search",
            "DBT_NOVA_SEARCH_ENABLE_VECTOR",
            || panic!("missing onnx/model.onnx"),
        );
        let (component, warning) = result.into_parts();
        assert!(component.is_none());
        let warning = warning.expect("panic should produce warning");
        assert!(warning.contains("vector search initialization failed"));
        assert!(warning.contains("DBT_NOVA_EMBEDDINGS_CACHE_DIR"));
        assert!(warning.contains("DBT_NOVA_SEARCH_ENABLE_VECTOR=false"));
        assert!(warning.contains("missing onnx/model.onnx"));
    }

    #[test]
    fn run_component_operation_converts_panics_into_server_errors() {
        let error = run_component_operation::<(), String, _>(
            "sparse search",
            "embedding generation",
            "DBT_NOVA_SEARCH_ENABLE_SPARSE",
            || panic!("failed to retrieve model.onnx"),
        )
        .expect_err("panic should surface as server error");
        let message = error.to_string();
        assert!(message.contains("sparse search embedding generation failed"));
        assert!(message.contains("DBT_NOVA_EMBEDDINGS_CACHE_DIR"));
        assert!(message.contains("DBT_NOVA_SEARCH_ENABLE_SPARSE=false"));
        assert!(message.contains("failed to retrieve model.onnx"));
    }

    #[test]
    fn disable_on_total_batch_failure_returns_warning_when_all_batches_fail() {
        let disabled = disable_on_total_batch_failure::<()>(
            "vector search",
            "DBT_NOVA_SEARCH_ENABLE_VECTOR",
            2,
            0,
            Some("missing onnx/model.onnx"),
        )
        .expect("all failed batches should disable component");
        let (component, warning) = disabled.into_parts();
        assert!(component.is_none());
        let warning = warning.expect("warning");
        assert!(warning.contains("startup embedding generation failed for all batches"));
        assert!(warning.contains("missing onnx/model.onnx"));
    }

    #[test]
    fn disable_on_total_batch_failure_keeps_component_ready_after_partial_success() {
        let disabled = disable_on_total_batch_failure::<()>(
            "sparse search",
            "DBT_NOVA_SEARCH_ENABLE_SPARSE",
            2,
            1,
            Some("failed to retrieve model.onnx"),
        );
        assert!(disabled.is_none());
    }
}
