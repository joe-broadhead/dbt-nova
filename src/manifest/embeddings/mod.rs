mod ann;
mod cache;
mod math;
mod text;

use ann::{AnnIndex, DenseEmbeddings, QuantizedEmbedding};
use math::{
    dot_product, dot_product_i8, normalize_vector, quantize_vector, score_desc_cmp, sparse_dot,
    top_k_scored,
};

use std::fmt::Display;
use std::fs;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use fastembed::{
    EmbeddingModel, InitOptions, RerankInitOptions, RerankerModel, SparseEmbedding,
    SparseInitOptions, SparseTextEmbedding, TextEmbedding, TextRerank,
};

use crate::config::{SearchColdStartPolicy, SearchConfig};
use crate::error::{DbtNovaError, Result};
use crate::manifest::rkyv_embeddings;
use crate::manifest::rkyv_sparse_embeddings;
use crate::manifest::rkyv_types::{CachedEmbeddings, CachedSparseEmbeddings};
use crate::manifest::semantic_cache::{
    SemanticCacheComponent, SemanticCachePaths, cache_paths, default_sparse_model_name,
};
use crate::manifest::store::EntityStore;
use tracing::{info, warn};

use super::SearchComponentBuild;

const PROXY_KEYS: [&str; 6] = [
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
];
const EMBEDDING_PROGRESS_LOG_EVERY_BATCHES: usize = 10;

pub use text::{
    embedding_text, embedding_text_from_archived, embedding_text_from_entity,
    embedding_text_from_json, embedding_text_from_payload,
};

use cache::embeddings_cache_dir;

pub struct VectorSearcher {
    model: LazyVectorQueryModel,
    embeddings: DenseEmbeddings,
    ann: Option<AnnIndex>,
}

pub struct SparseSearcher {
    model: LazySparseQueryModel,
    embeddings: Vec<(String, SparseEmbedding)>,
}

pub struct Reranker {
    model: LazyRerankerModel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RequiredLocalModelLayout {
    pub component: &'static str,
    pub model_code: String,
    pub model_file: String,
    pub additional_files: Vec<&'static str>,
}

struct LazyVectorQueryModel {
    model_name: EmbeddingModel,
    cache_dir: PathBuf,
    onnx_threads: usize,
    model: DeferredInit<TextEmbedding>,
}

struct LazySparseQueryModel {
    cache_dir: PathBuf,
    onnx_threads: usize,
    model: DeferredInit<SparseTextEmbedding>,
}

struct LazyRerankerModel {
    model_name: RerankerModel,
    cache_dir: PathBuf,
    onnx_threads: usize,
    model: DeferredInit<TextRerank>,
}

struct DeferredInit<T> {
    value: OnceLock<T>,
    init_lock: Mutex<()>,
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
        "Ensure model files are available in DBT_NOVA_EMBEDDINGS_CACHE_DIR, run `dbt-nova manifest warm` for the target manifest, or publish a models artifact. If you do not need this component, set {disable_env}=false."
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

fn component_missing_model_files_error(
    component_label: &str,
    disable_env: &str,
    cause: &str,
) -> DbtNovaError {
    component_runtime_error(
        component_label,
        "query model initialization",
        disable_env,
        cause,
    )
}

fn query_model_missing_files_warning(component_label: &str, disable_env: &str) -> String {
    format!(
        "{component_label} loaded reusable semantic state, but local query-model files are unavailable in DBT_NOVA_EMBEDDINGS_CACHE_DIR. The component is not query-ready until files are present or the lazy initializer succeeds. {}",
        model_guidance(disable_env)
    )
}

fn embeddable_entity_count(store: &EntityStore, config: &SearchConfig) -> Result<usize> {
    let mut count = 0usize;
    for unique_id in store.ids() {
        if let Some(entity) = store.get_archived(unique_id)?
            && !embedding_text_from_archived(entity, config).is_empty()
        {
            count += 1;
        }
    }
    Ok(count)
}

fn refuse_incomplete_cache_build<T>(
    component_label: &str,
    disable_env: &str,
    expected_items: usize,
    produced_items: usize,
    last_failure: Option<&str>,
    policy: SearchColdStartPolicy,
) -> Result<Option<SearchComponentBuild<T>>> {
    if produced_items == expected_items {
        return Ok(None);
    }
    let last_failure = last_failure.unwrap_or("unknown batch failure");
    let warning = component_init_warning(
        component_label,
        disable_env,
        &format!(
            "startup embedding generation produced an incomplete manifest-scoped cache payload; expected {expected_items} entries, produced {produced_items}. Last failure: {last_failure}"
        ),
    );
    if matches!(policy, SearchColdStartPolicy::Degrade) {
        Ok(Some(SearchComponentBuild::disabled(warning)))
    } else {
        Err(DbtNovaError::ServerError(warning))
    }
}

fn cache_startup_warning(
    component_label: &str,
    warm_flag: &str,
    disable_env: &str,
    paths: &SemanticCachePaths,
    cause: &str,
) -> String {
    let legacy_note = if paths.legacy_present() {
        " Legacy singleton cache files are ignored."
    } else {
        ""
    };
    format!(
        "{component_label} startup skipped because the manifest-scoped cache is unavailable at {}. Run `dbt-nova manifest warm --{warm_flag}` for this manifest, or publish a models artifact.{legacy_note} If you do not need this component, set {disable_env}=false. Cause: {cause}",
        paths.compressed_path.display()
    )
}

fn configured_embedding_model_name(config: &SearchConfig) -> String {
    let trimmed = config.embedding_model.trim();
    if trimmed.is_empty() {
        SearchConfig::default().embedding_model
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
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

fn model_repo_dir(cache_dir: &Path, model_code: &str) -> PathBuf {
    cache_dir.join(format!("models--{}", model_code.replace('/', "--")))
}

fn snapshot_dir_from_repo_dir(repo_dir: &Path) -> Result<PathBuf> {
    if !repo_dir.is_dir() {
        return Err(DbtNovaError::ServerError(format!(
            "missing model repository directory {}",
            repo_dir.display()
        )));
    }

    let snapshots_dir = repo_dir.join("snapshots");
    if !snapshots_dir.is_dir() {
        return Err(DbtNovaError::ServerError(format!(
            "missing snapshots directory in {}",
            repo_dir.display()
        )));
    }

    let refs_main = repo_dir.join("refs").join("main");
    if refs_main.is_file() {
        let revision = fs::read_to_string(&refs_main)?.trim().to_string();
        if !revision.is_empty() {
            let revision_snapshot = snapshots_dir.join(&revision);
            if revision_snapshot.is_dir() {
                return Ok(revision_snapshot);
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
        "unable to resolve local snapshot directory in {}",
        repo_dir.display()
    )))
}

fn validate_local_model_files(
    cache_dir: &Path,
    model_code: &str,
    model_file: &str,
    additional_files: &[&str],
) -> Result<()> {
    let repo_dir = model_repo_dir(cache_dir, model_code);
    let snapshot_dir = snapshot_dir_from_repo_dir(&repo_dir)?;
    let mut required_files = vec![
        model_file,
        "tokenizer.json",
        "config.json",
        "special_tokens_map.json",
        "tokenizer_config.json",
    ];
    required_files.extend_from_slice(additional_files);

    for relative_path in required_files {
        let path = snapshot_dir.join(relative_path);
        if !path.is_file() {
            return Err(DbtNovaError::ServerError(format!(
                "missing required local model file {} for {}",
                path.display(),
                model_code
            )));
        }
    }

    Ok(())
}

impl<T> DeferredInit<T> {
    fn new() -> Self {
        Self {
            value: OnceLock::new(),
            init_lock: Mutex::new(()),
        }
    }

    fn get_or_try_init<F>(&self, init: F) -> Result<&T>
    where
        F: FnOnce() -> Result<T>,
    {
        if let Some(value) = self.value.get() {
            return Ok(value);
        }

        let _guard = self
            .init_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(value) = self.value.get() {
            return Ok(value);
        }

        let value = init()?;
        let _ = self.value.set(value);
        self.value.get().ok_or_else(|| {
            DbtNovaError::ServerError("lazy model initialization did not persist".to_string())
        })
    }

    fn initialized(&self) -> bool {
        self.value.get().is_some()
    }
}

impl LazyVectorQueryModel {
    fn new(model_name: EmbeddingModel, cache_dir: PathBuf, onnx_threads: usize) -> Self {
        Self {
            model_name,
            cache_dir,
            onnx_threads,
            model: DeferredInit::new(),
        }
    }

    fn ensure_local_files_available(&self) -> Result<()> {
        let model_info = TextEmbedding::get_model_info(&self.model_name);
        let mut additional_files = Vec::new();
        if self.model_name == EmbeddingModel::MultilingualE5Large {
            additional_files.push("model.onnx_data");
        }
        validate_local_model_files(
            &self.cache_dir,
            &model_info.model_code,
            &model_info.model_file,
            &additional_files,
        )
        .map_err(|err| {
            component_missing_model_files_error(
                "vector search",
                "DBT_NOVA_SEARCH_ENABLE_VECTOR",
                &err.to_string(),
            )
        })
    }

    fn get_or_try_init(&self) -> Result<&TextEmbedding> {
        self.model.get_or_try_init(|| {
            info!(
                model_name = %self.model_name,
                cache_dir = %self.cache_dir.display(),
                "initializing vector query model lazily"
            );
            run_component_operation(
                "vector search",
                "query model initialization",
                "DBT_NOVA_SEARCH_ENABLE_VECTOR",
                || {
                    TextEmbedding::try_new(InitOptions {
                        model_name: self.model_name.clone(),
                        cache_dir: self.cache_dir.clone(),
                        show_download_progress: false,
                        threads: Some(self.onnx_threads),
                        ..Default::default()
                    })
                },
            )
        })
    }

    fn initialized(&self) -> bool {
        self.model.initialized()
    }

    fn local_files_present(&self) -> bool {
        self.ensure_local_files_available().is_ok()
    }

    fn query_ready(&self) -> bool {
        self.initialized() || self.local_files_present()
    }
}

impl LazySparseQueryModel {
    fn new(cache_dir: PathBuf, onnx_threads: usize) -> Self {
        Self {
            cache_dir,
            onnx_threads,
            model: DeferredInit::new(),
        }
    }

    fn ensure_local_files_available(&self) -> Result<()> {
        let model_info = SparseTextEmbedding::list_supported_models()
            .into_iter()
            .find(|model| model.model_code == default_sparse_model_name())
            .ok_or_else(|| {
                DbtNovaError::ServerError(format!(
                    "unsupported sparse model {}",
                    default_sparse_model_name()
                ))
            })?;
        validate_local_model_files(
            &self.cache_dir,
            &model_info.model_code,
            &model_info.model_file,
            &[],
        )
        .map_err(|err| {
            component_missing_model_files_error(
                "sparse search",
                "DBT_NOVA_SEARCH_ENABLE_SPARSE",
                &err.to_string(),
            )
        })
    }

    fn get_or_try_init(&self) -> Result<&SparseTextEmbedding> {
        self.model.get_or_try_init(|| {
            info!(
                model_name = %default_sparse_model_name(),
                cache_dir = %self.cache_dir.display(),
                "initializing sparse query model lazily"
            );
            run_component_operation(
                "sparse search",
                "query model initialization",
                "DBT_NOVA_SEARCH_ENABLE_SPARSE",
                || {
                    SparseTextEmbedding::try_new(SparseInitOptions {
                        cache_dir: self.cache_dir.clone(),
                        show_download_progress: false,
                        threads: Some(self.onnx_threads),
                        ..Default::default()
                    })
                },
            )
        })
    }

    fn initialized(&self) -> bool {
        self.model.initialized()
    }

    fn local_files_present(&self) -> bool {
        self.ensure_local_files_available().is_ok()
    }

    fn query_ready(&self) -> bool {
        self.initialized() || self.local_files_present()
    }
}

impl LazyRerankerModel {
    fn new(model_name: RerankerModel, cache_dir: PathBuf, onnx_threads: usize) -> Self {
        Self {
            model_name,
            cache_dir,
            onnx_threads,
            model: DeferredInit::new(),
        }
    }

    fn ensure_local_files_available(&self) -> Result<()> {
        let model_info = TextRerank::get_model_info(&self.model_name);
        validate_local_model_files(
            &self.cache_dir,
            &model_info.model_code,
            &model_info.model_file,
            &[],
        )
        .map_err(|err| {
            component_missing_model_files_error(
                "reranker",
                "DBT_NOVA_SEARCH_ENABLE_RERANKER",
                &err.to_string(),
            )
        })
    }

    fn get_or_try_init(&self) -> Result<&TextRerank> {
        self.model.get_or_try_init(|| {
            info!(
                model_name = %self.model_name,
                cache_dir = %self.cache_dir.display(),
                "initializing reranker model lazily"
            );
            run_component_operation(
                "reranker",
                "query model initialization",
                "DBT_NOVA_SEARCH_ENABLE_RERANKER",
                || {
                    TextRerank::try_new(RerankInitOptions {
                        model_name: self.model_name.clone(),
                        cache_dir: self.cache_dir.clone(),
                        show_download_progress: false,
                        threads: Some(self.onnx_threads),
                        ..Default::default()
                    })
                },
            )
        })
    }

    fn initialized(&self) -> bool {
        self.model.initialized()
    }

    fn local_files_present(&self) -> bool {
        self.ensure_local_files_available().is_ok()
    }

    fn query_ready(&self) -> bool {
        self.initialized() || self.local_files_present()
    }
}

pub(crate) fn required_embedding_model_layout(value: &str) -> RequiredLocalModelLayout {
    let model_name = resolve_embedding_model(value);
    let model_info = TextEmbedding::get_model_info(&model_name);
    let mut additional_files = Vec::new();
    if model_name == EmbeddingModel::MultilingualE5Large {
        additional_files.push("model.onnx_data");
    }

    RequiredLocalModelLayout {
        component: "vector",
        model_code: model_info.model_code,
        model_file: model_info.model_file,
        additional_files,
    }
}

pub(crate) fn required_sparse_model_layout() -> RequiredLocalModelLayout {
    let model_info = SparseTextEmbedding::list_supported_models()
        .into_iter()
        .find(|model| model.model_code == default_sparse_model_name())
        .expect("default sparse model should be supported");

    RequiredLocalModelLayout {
        component: "sparse",
        model_code: model_info.model_code,
        model_file: model_info.model_file,
        additional_files: Vec::new(),
    }
}

pub(crate) fn required_reranker_model_layout(value: &str) -> RequiredLocalModelLayout {
    let model_name = resolve_reranker_model(value);
    let model_info = TextRerank::get_model_info(&model_name);

    RequiredLocalModelLayout {
        component: "reranker",
        model_code: model_info.model_code,
        model_file: model_info.model_file,
        additional_files: Vec::new(),
    }
}

impl VectorSearcher {
    #[must_use]
    pub fn query_model_initialized(&self) -> bool {
        self.model.initialized()
    }

    #[must_use]
    pub fn query_model_files_present(&self) -> bool {
        self.model.local_files_present()
    }

    #[must_use]
    pub fn query_ready(&self) -> bool {
        self.model.query_ready()
    }

    /// Ensure the local vector query model files are available and the model can initialize.
    ///
    /// # Errors
    /// Returns an error when the model cannot be initialized or required local files are absent.
    pub fn warm_query_model(&self) -> Result<()> {
        let _ = self.model.get_or_try_init()?;
        self.model.ensure_local_files_available()
    }

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
        let manifest_hash = config.manifest_hash.as_deref().unwrap_or("");
        let model_name = configured_embedding_model_name(config);
        let cache_paths = cache_paths(
            config,
            SemanticCacheComponent::Dense,
            &model_name,
            manifest_hash,
        );
        let build_started = Instant::now();
        info!(
            manifest_hash,
            model_name = %model_name,
            entity_count = store.len(),
            batch_size = config.embedding_batch_size.max(1),
            cache_path = %cache_paths.compressed_path.display(),
            "starting vector semantic cache build"
        );
        let mut cached_embeddings = None;
        let expected_entries = embeddable_entity_count(store, config)?;
        let model = LazyVectorQueryModel::new(
            resolve_embedding_model(&model_name),
            cache_dir.clone(),
            config.onnx_threads,
        );

        if !config.force_rebuild_semantic_caches {
            match rkyv_embeddings::load_embeddings(
                config,
                &model_name,
                manifest_hash,
                Some(expected_entries),
                config.embeddings_max_decompressed_bytes,
            ) {
                rkyv_embeddings::EmbeddingsCacheLoad::Hit { cache, .. } => {
                    cached_embeddings = Some(cache);
                }
                rkyv_embeddings::EmbeddingsCacheLoad::Miss { paths, failure } => {
                    if matches!(config.cold_start_policy, SearchColdStartPolicy::Degrade) {
                        let warning = cache_startup_warning(
                            "vector search",
                            "vector",
                            "DBT_NOVA_SEARCH_ENABLE_VECTOR",
                            &paths,
                            &failure.summary(),
                        );
                        warn!(warning = %warning, "Vector search unavailable during startup");
                        return Ok(SearchComponentBuild::disabled(warning));
                    }
                    warn!(
                        cache_path = %paths.compressed_path.display(),
                        reason = %failure.summary(),
                        "Vector search cache unavailable; rebuilding manifest-scoped cache"
                    );
                }
            }
        }

        let batch_size = config.embedding_batch_size.max(1);
        let use_quant = config.enable_vector_quantization;
        if let Some(cached) = cached_embeddings {
            let cached_hyperplanes = cached.ann_hyperplanes.clone();
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
                DenseEmbeddings::Float(values) => cached_hyperplanes
                    .clone()
                    .and_then(|hyperplanes| {
                        AnnIndex::build_f32_with_hyperplanes(values, config, hyperplanes)
                    })
                    .or_else(|| AnnIndex::build_f32(values, config)),
                DenseEmbeddings::Quantized(values) => cached_hyperplanes
                    .and_then(|hyperplanes| {
                        AnnIndex::build_quantized_with_hyperplanes(values, config, hyperplanes)
                    })
                    .or_else(|| AnnIndex::build_quantized(values, config)),
            };
            let query_model_files_present = model.local_files_present();
            info!(
                manifest_hash,
                model_name = %model_name,
                query_model_files_present,
                elapsed_ms = build_started.elapsed().as_millis(),
                "vector semantic cache ready; deferring query model initialization"
            );
            let searcher = Self {
                model,
                embeddings,
                ann,
            };
            if query_model_files_present {
                return Ok(SearchComponentBuild::ready(searcher));
            }
            return Ok(SearchComponentBuild::ready_with_warning(
                searcher,
                query_model_missing_files_warning("vector search", "DBT_NOVA_SEARCH_ENABLE_VECTOR"),
            ));
        }
        let model_ref = match model.get_or_try_init() {
            Ok(model_ref) => model_ref,
            Err(err) => {
                let warning = component_init_warning(
                    "vector search",
                    "DBT_NOVA_SEARCH_ENABLE_VECTOR",
                    &err.to_string(),
                );
                warn!(warning = %warning, "Vector search unavailable during startup");
                return Ok(SearchComponentBuild::disabled(warning));
            }
        };
        info!(
            manifest_hash,
            model_name = %model_name,
            elapsed_ms = build_started.elapsed().as_millis(),
            "vector embedding model ready"
        );
        let mut embeddings_f32: Vec<(String, Vec<f32>)> = Vec::new();
        let mut embeddings_i8: Vec<QuantizedEmbedding> = Vec::new();
        let mut batch_ids: Vec<String> = Vec::with_capacity(batch_size);
        let mut batch_texts: Vec<String> = Vec::with_capacity(batch_size);
        let mut attempted_batches = 0usize;
        let mut last_batch_failure: Option<String> = None;
        let mut processed_items = 0usize;

        let mut flush_batch =
            |batch_ids: &mut Vec<String>, batch_texts: &mut Vec<String>| -> Result<()> {
                if batch_texts.is_empty() {
                    return Ok(());
                }
                attempted_batches += 1;

                let texts = std::mem::take(batch_texts);
                let ids = std::mem::take(batch_ids);
                let batch_len = ids.len();

                let mut vecs = match run_component_operation(
                    "vector search",
                    "embedding batch generation",
                    "DBT_NOVA_SEARCH_ENABLE_VECTOR",
                    || model_ref.embed(texts, Some(batch_size)),
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
                processed_items += batch_len;
                if attempted_batches == 1
                    || attempted_batches.is_multiple_of(EMBEDDING_PROGRESS_LOG_EVERY_BATCHES)
                {
                    info!(
                        manifest_hash,
                        model_name = %model_name,
                        attempted_batches,
                        processed_items,
                        produced_items = if use_quant {
                            embeddings_i8.len()
                        } else {
                            embeddings_f32.len()
                        },
                        elapsed_ms = build_started.elapsed().as_millis(),
                        "vector embedding warm progress"
                    );
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
        info!(
            manifest_hash,
            model_name = %model_name,
            attempted_batches,
            processed_items,
            produced_items,
            elapsed_ms = build_started.elapsed().as_millis(),
            "vector embedding generation finished"
        );
        if let Some(disabled) = refuse_incomplete_cache_build(
            "vector search",
            "DBT_NOVA_SEARCH_ENABLE_VECTOR",
            expected_entries,
            produced_items,
            last_batch_failure.as_deref(),
            config.cold_start_policy,
        )? {
            if let Some(warning) = disabled.warning.as_ref() {
                warn!(warning = %warning, "Vector search unavailable during startup");
            }
            return Ok(disabled);
        }

        let (embeddings, ann) = if use_quant {
            info!(
                manifest_hash,
                model_name = %model_name,
                entries = embeddings_i8.len(),
                elapsed_ms = build_started.elapsed().as_millis(),
                "building vector ANN index from quantized embeddings"
            );
            let ann = AnnIndex::build_quantized(&embeddings_i8, config);
            (DenseEmbeddings::Quantized(embeddings_i8), ann)
        } else {
            info!(
                manifest_hash,
                model_name = %model_name,
                entries = embeddings_f32.len(),
                elapsed_ms = build_started.elapsed().as_millis(),
                "building vector ANN index from float embeddings"
            );
            let ann = AnnIndex::build_f32(&embeddings_f32, config);
            (DenseEmbeddings::Float(embeddings_f32), ann)
        };

        let mut cache_ids = Vec::new();
        let mut cache_vectors = Vec::new();
        let mut is_quantized = false;
        info!(
            manifest_hash,
            model_name = %model_name,
            entries = produced_items,
            elapsed_ms = build_started.elapsed().as_millis(),
            "preparing vector cache payload"
        );
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

        let (ann_hyperplanes, ann_bucket_keys, ann_bucket_values) =
            ann.as_ref().map_or((None, None, None), |ann| {
                let (keys, values) = ann.cache_bucket_parts();
                (Some(ann.hyperplanes.clone()), Some(keys), Some(values))
            });

        let cache = CachedEmbeddings {
            schema_version: crate::manifest::rkyv_types::RKYV_SCHEMA_VERSION,
            model_name: model_name.clone(),
            manifest_hash: manifest_hash.to_string(),
            entity_ids: cache_ids,
            dense_embeddings: cache_vectors,
            is_quantized,
            sparse_indices: None,
            sparse_values: None,
            ann_hyperplanes,
            ann_bucket_keys,
            ann_bucket_values,
        };
        info!(
            manifest_hash,
            model_name = %model_name,
            entries = cache.entity_ids.len(),
            cache_path = %cache_paths.compressed_path.display(),
            elapsed_ms = build_started.elapsed().as_millis(),
            "persisting vector cache"
        );
        if let Err(err) = rkyv_embeddings::save_embeddings(&cache, config) {
            warn!(error = %err, "failed to save embeddings cache");
        } else {
            info!(
                manifest_hash,
                model_name = %model_name,
                entries = cache.entity_ids.len(),
                cache_path = %cache_paths.compressed_path.display(),
                elapsed_ms = build_started.elapsed().as_millis(),
                "persisted vector cache"
            );
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
            || {
                self.model
                    .get_or_try_init()?
                    .embed(vec![query.to_string()], None)
            },
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
    #[must_use]
    pub fn query_model_initialized(&self) -> bool {
        self.model.initialized()
    }

    #[must_use]
    pub fn query_model_files_present(&self) -> bool {
        self.model.local_files_present()
    }

    #[must_use]
    pub fn query_ready(&self) -> bool {
        self.model.query_ready()
    }

    /// Ensure the local sparse query model files are available and the model can initialize.
    ///
    /// # Errors
    /// Returns an error when the model cannot be initialized or required local files are absent.
    pub fn warm_query_model(&self) -> Result<()> {
        let _ = self.model.get_or_try_init()?;
        self.model.ensure_local_files_available()
    }

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
        let manifest_hash = config.manifest_hash.as_deref().unwrap_or("");
        let sparse_model_name = default_sparse_model_name().to_string();
        let cache_paths = cache_paths(
            config,
            SemanticCacheComponent::Sparse,
            &sparse_model_name,
            manifest_hash,
        );
        let build_started = Instant::now();
        info!(
            manifest_hash,
            model_name = %sparse_model_name,
            entity_count = store.len(),
            batch_size = config.sparse_embedding_batch_size.max(1),
            cache_path = %cache_paths.compressed_path.display(),
            "starting sparse semantic cache build"
        );
        let mut cached_embeddings = None;
        let model = LazySparseQueryModel::new(cache_dir.clone(), config.onnx_threads);
        let expected_entries = embeddable_entity_count(store, config)?;

        if !config.force_rebuild_semantic_caches {
            match rkyv_sparse_embeddings::load_sparse_embeddings(
                config,
                &sparse_model_name,
                manifest_hash,
                Some(expected_entries),
                config.embeddings_max_decompressed_bytes,
            ) {
                rkyv_sparse_embeddings::SparseEmbeddingsCacheLoad::Hit { cache, .. } => {
                    cached_embeddings = Some(cache);
                }
                rkyv_sparse_embeddings::SparseEmbeddingsCacheLoad::Miss { paths, failure } => {
                    if matches!(config.cold_start_policy, SearchColdStartPolicy::Degrade) {
                        let warning = cache_startup_warning(
                            "sparse search",
                            "sparse",
                            "DBT_NOVA_SEARCH_ENABLE_SPARSE",
                            &paths,
                            &failure.summary(),
                        );
                        warn!(warning = %warning, "Sparse search unavailable during startup");
                        return Ok(SearchComponentBuild::disabled(warning));
                    }
                    warn!(
                        cache_path = %paths.compressed_path.display(),
                        reason = %failure.summary(),
                        "Sparse search cache unavailable; rebuilding manifest-scoped cache"
                    );
                }
            }
        }

        let batch_size = config.sparse_embedding_batch_size.max(1);
        if let Some(cached) = cached_embeddings {
            let embeddings = cached
                .entity_ids
                .into_iter()
                .zip(cached.sparse_indices.into_iter().zip(cached.sparse_values))
                .map(|(id, (indices, values))| (id, SparseEmbedding { indices, values }))
                .collect::<Vec<_>>();
            let query_model_files_present = model.local_files_present();
            info!(
                manifest_hash,
                model_name = %sparse_model_name,
                query_model_files_present,
                elapsed_ms = build_started.elapsed().as_millis(),
                "sparse semantic cache ready; deferring query model initialization"
            );
            let searcher = Self { model, embeddings };
            if query_model_files_present {
                return Ok(SearchComponentBuild::ready(searcher));
            }
            return Ok(SearchComponentBuild::ready_with_warning(
                searcher,
                query_model_missing_files_warning("sparse search", "DBT_NOVA_SEARCH_ENABLE_SPARSE"),
            ));
        }
        let model_ref = match model.get_or_try_init() {
            Ok(model_ref) => model_ref,
            Err(err) => {
                let warning = component_init_warning(
                    "sparse search",
                    "DBT_NOVA_SEARCH_ENABLE_SPARSE",
                    &err.to_string(),
                );
                warn!(warning = %warning, "Sparse search unavailable during startup");
                return Ok(SearchComponentBuild::disabled(warning));
            }
        };
        info!(
            manifest_hash,
            model_name = %sparse_model_name,
            elapsed_ms = build_started.elapsed().as_millis(),
            "sparse embedding model ready"
        );
        let mut embeddings: Vec<(String, SparseEmbedding)> = Vec::new();
        let mut batch_ids: Vec<String> = Vec::with_capacity(batch_size);
        let mut batch_texts: Vec<String> = Vec::with_capacity(batch_size);
        let mut attempted_batches = 0usize;
        let mut last_batch_failure: Option<String> = None;
        let mut processed_items = 0usize;

        let mut flush_batch =
            |batch_ids: &mut Vec<String>, batch_texts: &mut Vec<String>| -> Result<()> {
                if batch_texts.is_empty() {
                    return Ok(());
                }
                attempted_batches += 1;

                let texts = std::mem::take(batch_texts);
                let ids = std::mem::take(batch_ids);
                let batch_len = ids.len();

                let mut vecs = match run_component_operation(
                    "sparse search",
                    "embedding batch generation",
                    "DBT_NOVA_SEARCH_ENABLE_SPARSE",
                    || model_ref.embed(texts, Some(batch_size)),
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
                processed_items += batch_len;
                if attempted_batches == 1
                    || attempted_batches.is_multiple_of(EMBEDDING_PROGRESS_LOG_EVERY_BATCHES)
                {
                    info!(
                        manifest_hash,
                        model_name = %sparse_model_name,
                        attempted_batches,
                        processed_items,
                        produced_items = embeddings.len(),
                        elapsed_ms = build_started.elapsed().as_millis(),
                        "sparse embedding warm progress"
                    );
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
        info!(
            manifest_hash,
            model_name = %sparse_model_name,
            attempted_batches,
            processed_items,
            produced_items = embeddings.len(),
            elapsed_ms = build_started.elapsed().as_millis(),
            "sparse embedding generation finished"
        );

        if let Some(disabled) = refuse_incomplete_cache_build(
            "sparse search",
            "DBT_NOVA_SEARCH_ENABLE_SPARSE",
            expected_entries,
            embeddings.len(),
            last_batch_failure.as_deref(),
            config.cold_start_policy,
        )? {
            if let Some(warning) = disabled.warning.as_ref() {
                warn!(warning = %warning, "Sparse search unavailable during startup");
            }
            return Ok(disabled);
        }

        let mut cache_ids = Vec::with_capacity(embeddings.len());
        let mut cache_indices = Vec::with_capacity(embeddings.len());
        let mut cache_values = Vec::with_capacity(embeddings.len());
        let total_sparse_terms: usize = embeddings
            .iter()
            .map(|(_, embedding)| embedding.indices.len())
            .sum();
        let avg_sparse_terms_per_entry = if embeddings.is_empty() {
            "0.00".to_string()
        } else {
            let scaled_average = total_sparse_terms.saturating_mul(100) / embeddings.len();
            format!("{}.{:02}", scaled_average / 100, scaled_average % 100)
        };
        info!(
            manifest_hash,
            model_name = %sparse_model_name,
            entries = embeddings.len(),
            total_sparse_terms,
            avg_sparse_terms_per_entry = %avg_sparse_terms_per_entry,
            elapsed_ms = build_started.elapsed().as_millis(),
            "preparing sparse cache payload"
        );
        for (id, embedding) in &embeddings {
            cache_ids.push(id.clone());
            cache_indices.push(embedding.indices.clone());
            cache_values.push(embedding.values.clone());
        }

        let cache = CachedSparseEmbeddings {
            schema_version: crate::manifest::rkyv_types::RKYV_SCHEMA_VERSION,
            model_name: sparse_model_name.clone(),
            manifest_hash: manifest_hash.to_string(),
            entity_ids: cache_ids,
            sparse_indices: cache_indices,
            sparse_values: cache_values,
        };
        info!(
            manifest_hash,
            model_name = %sparse_model_name,
            entries = cache.entity_ids.len(),
            total_sparse_terms,
            cache_path = %cache_paths.compressed_path.display(),
            elapsed_ms = build_started.elapsed().as_millis(),
            "persisting sparse cache"
        );
        if let Err(err) = rkyv_sparse_embeddings::save_sparse_embeddings(&cache, config) {
            warn!(error = %err, "failed to save sparse embeddings cache");
        } else {
            info!(
                manifest_hash,
                model_name = %sparse_model_name,
                entries = cache.entity_ids.len(),
                total_sparse_terms,
                cache_path = %cache_paths.compressed_path.display(),
                elapsed_ms = build_started.elapsed().as_millis(),
                "persisted sparse cache"
            );
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
            || {
                self.model
                    .get_or_try_init()?
                    .embed(vec![query.to_string()], None)
            },
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
    #[must_use]
    pub fn initialized(&self) -> bool {
        self.model.initialized()
    }

    #[must_use]
    pub fn query_model_files_present(&self) -> bool {
        self.model.local_files_present()
    }

    #[must_use]
    pub fn query_ready(&self) -> bool {
        self.model.query_ready()
    }

    /// Ensure the local reranker model files are available and the model can initialize.
    ///
    /// # Errors
    /// Returns an error when the model cannot be initialized or required local files are absent.
    pub fn warm_query_model(&self) -> Result<()> {
        let _ = self.model.get_or_try_init()?;
        self.model.ensure_local_files_available()
    }

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
        let model = LazyRerankerModel::new(model_name, cache_dir, config.onnx_threads);
        let query_model_files_present = model.local_files_present();
        info!(
            query_model_files_present,
            "reranker ready; deferring model initialization until first use"
        );
        let reranker = Self { model };
        if query_model_files_present {
            Ok(SearchComponentBuild::ready(reranker))
        } else {
            Ok(SearchComponentBuild::ready_with_warning(
                reranker,
                query_model_missing_files_warning("reranker", "DBT_NOVA_SEARCH_ENABLE_RERANKER"),
            ))
        }
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
            || {
                self.model
                    .get_or_try_init()?
                    .rerank(query, docs, false, None)
            },
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

#[cfg(test)]
mod tests;
