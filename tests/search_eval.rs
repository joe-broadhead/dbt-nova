//! Manual search quality evaluation harness.
//!
//! This test compares lexical-only search against hybrid search (lexical + vector + sparse + reranker)
//! on a judged query set (qrels) and reports quality/latency metrics.
//!
//! Run with:
//!   cargo test --test search_eval -- --ignored --nocapture
//!
//! Optional environment variables:
//!   DBT_NOVA_EVAL_QRELS_PATH=/path/to/qrels.json
//!   DBT_NOVA_EVAL_MANIFEST_PATH=/path/to/manifest.json
//!   DBT_NOVA_EVAL_EMBEDDINGS_CACHE_DIR=/path/to/model_cache
//!   DBT_NOVA_EVAL_TOP_K=10
//!   DBT_NOVA_EVAL_ENABLE_HYBRID=1|0
//!   DBT_NOVA_EVAL_ENABLE_LIFECYCLE=1|0
//!   DBT_NOVA_EVAL_ALLOW_EMBEDDING_DOWNLOAD=1|0
//!   DBT_NOVA_EVAL_REQUIRE_MODELS=1|0
//!   DBT_NOVA_EVAL_RELOAD_TIMEOUT_SECS=600
//!   DBT_NOVA_EVAL_MIN_QUERY_COUNT=10
//!   DBT_NOVA_EVAL_ASSERT_HYBRID_NONDECREASING=1|0
//!   DBT_NOVA_EVAL_ASSERT_MIN_DELTA_MRR=0.02
//!   DBT_NOVA_EVAL_ASSERT_MIN_DELTA_RECALL=0.03
//!   DBT_NOVA_EVAL_ASSERT_MAX_COLD_START_MS=60000
//!   DBT_NOVA_EVAL_ASSERT_MAX_RELOAD_SWAP_MS=90000

#[allow(dead_code)]
#[path = "support/config.rs"]
mod support_config;

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use dbt_nova::config::SearchConfig;
use dbt_nova::manifest::search::ManifestStatus;
use dbt_nova::params::{DetailLevel, PaginationParams, ReloadManifestParams, SearchParams};
use dbt_nova::{DbtNovaConfig, ManifestSearch, ManifestSearchHandle};
use serde::Deserialize;
use serde_json::Value as JsonValue;
use support_config::{TestStorageGuard, apply_test_storage};

#[derive(Debug, Deserialize)]
struct EvalSuite {
    #[serde(default)]
    manifest_path: Option<String>,
    #[serde(default)]
    top_k: Option<usize>,
    queries: Vec<EvalQuery>,
}

#[derive(Debug, Deserialize)]
struct EvalQuery {
    id: String,
    query: String,
    #[serde(default)]
    persona: Option<String>,
    #[serde(default)]
    resource_types: Vec<String>,
    relevant: Vec<RelevantDoc>,
}

#[derive(Debug, Deserialize)]
struct RelevantDoc {
    unique_id: String,
    #[serde(default = "default_grade")]
    grade: f32,
}

#[derive(Debug, Clone)]
struct QueryMetrics {
    id: String,
    top_hit: Option<String>,
    recall_at_k: f64,
    mrr_at_k: f64,
    ndcg_at_k: f64,
    latency_ms: f64,
}

#[derive(Debug, Clone)]
struct ProfileMetrics {
    profile: &'static str,
    query_count: usize,
    hit_rate_at_k: f64,
    recall_at_k: f64,
    mrr_at_k: f64,
    ndcg_at_k: f64,
    mean_latency_ms: f64,
    p95_latency_ms: f64,
    per_query: Vec<QueryMetrics>,
}

#[derive(Debug, Clone)]
struct LifecycleMetrics {
    profile: &'static str,
    cold_start_ms: f64,
    reload_swap_ms: f64,
}

#[derive(Debug, Clone, Copy)]
struct Profile {
    name: &'static str,
    vector: bool,
    sparse: bool,
    reranker: bool,
}

const LEXICAL_PROFILE: Profile = Profile {
    name: "lexical_only",
    vector: false,
    sparse: false,
    reranker: false,
};

const HYBRID_PROFILE: Profile = Profile {
    name: "hybrid",
    vector: true,
    sparse: true,
    reranker: true,
};

fn default_grade() -> f32 {
    1.0
}

fn env_flag_with_default(name: &str, default: bool) -> bool {
    std::env::var(name)
        .ok()
        .map(|v| {
            matches!(
                v.trim().to_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(default)
}

fn env_flag(name: &str) -> bool {
    env_flag_with_default(name, false)
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(default)
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(default)
}

fn env_f64(name: &str) -> Option<f64> {
    std::env::var(name).ok().and_then(|v| v.parse::<f64>().ok())
}

fn resolve_qrels_path() -> PathBuf {
    std::env::var("DBT_NOVA_EVAL_QRELS_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests")
                .join("fixtures")
                .join("search_eval_qrels.json")
        })
}

fn resolve_manifest_path(suite: &EvalSuite, qrels_path: &Path) -> PathBuf {
    if let Ok(path) = std::env::var("DBT_NOVA_EVAL_MANIFEST_PATH") {
        return PathBuf::from(path);
    }
    if let Some(path) = &suite.manifest_path {
        let candidate = PathBuf::from(path);
        if candidate.is_absolute() || candidate.exists() {
            return candidate;
        }
        let from_qrels_dir = qrels_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(&candidate);
        if from_qrels_dir.exists() {
            return from_qrels_dir;
        }
        return PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(candidate);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("nova_manifest.json")
}

fn resolve_embedding_cache_dir() -> String {
    if let Ok(path) = std::env::var("DBT_NOVA_EVAL_EMBEDDINGS_CACHE_DIR") {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home)
            .join(".dbt-nova-models")
            .to_string_lossy()
            .to_string();
    }
    String::new()
}

fn load_suite(path: &Path) -> EvalSuite {
    let raw = fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("failed to read qrels file '{}': {e}", path.display()));
    serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("failed to parse qrels file '{}': {e}", path.display()))
}

fn with_download_guard<T>(allow_download: bool, build: impl FnOnce() -> T) -> T {
    if allow_download {
        return build();
    }

    let _hf_hub_offline = ScopedEnvVar::set("HF_HUB_OFFLINE", "1");
    let _hf_hub_transfer = ScopedEnvVar::set("HUGGINGFACE_HUB_OFFLINE", "1");

    build()
}

#[derive(Debug)]
struct ScopedEnvVar {
    key: &'static str,
    previous: Option<OsString>,
    changed: bool,
}

impl ScopedEnvVar {
    fn set(key: &'static str, value: &str) -> Self {
        let previous = std::env::var_os(key);
        let changed = previous.as_deref() != Some(std::ffi::OsStr::new(value));
        if changed {
            unsafe {
                std::env::set_var(key, value);
            }
        }
        Self {
            key,
            previous,
            changed,
        }
    }
}

impl Drop for ScopedEnvVar {
    fn drop(&mut self) {
        if !self.changed {
            return;
        }
        if let Some(previous) = self.previous.clone() {
            unsafe {
                std::env::set_var(self.key, previous);
            }
        } else {
            unsafe {
                std::env::remove_var(self.key);
            }
        }
    }
}

fn build_profile_config(
    manifest_path: &Path,
    profile: Profile,
    guard: &TestStorageGuard,
) -> DbtNovaConfig {
    let mut cfg = DbtNovaConfig {
        manifest_path: manifest_path.to_string_lossy().to_string(),
        search: SearchConfig {
            enable_vector_search: profile.vector,
            enable_sparse_search: profile.sparse,
            enable_reranker: profile.reranker,
            embedding_cache_dir: resolve_embedding_cache_dir(),
            ..SearchConfig::default()
        },
        storage_read_only: false,
        ..DbtNovaConfig::default()
    };
    apply_test_storage(&mut cfg, guard);
    cfg
}

fn profile_requires_model_components(profile: Profile) -> bool {
    profile.vector || profile.sparse || profile.reranker
}

fn build_searcher(
    manifest_path: &Path,
    profile: Profile,
    allow_download: bool,
    require_models: bool,
) -> ManifestSearch {
    let guard = TestStorageGuard::new();
    let cfg = build_profile_config(manifest_path, profile, &guard);
    let searcher = with_download_guard(allow_download, || {
        ManifestSearch::new(cfg)
            .map(|loaded| loaded.search)
            .unwrap_or_else(|e| {
                let extra = if !allow_download && profile_requires_model_components(profile) {
                    " (embeddings/sparse/reranker downloads were disabled via DBT_NOVA_EVAL_ALLOW_EMBEDDING_DOWNLOAD=0)"
                } else {
                    ""
                };
                panic!(
                    "failed to build searcher for profile '{}' with manifest '{}': {e}{extra}",
                    profile.name,
                    manifest_path.display()
                )
            })
    });

    if require_models {
        let mut missing_components = Vec::new();
        if profile.vector && !searcher.vector_search_ready() {
            missing_components.push("vector");
        }
        if profile.sparse && !searcher.sparse_search_ready() {
            missing_components.push("sparse");
        }
        if profile.reranker && !searcher.reranker_ready() {
            missing_components.push("reranker");
        }

        if !missing_components.is_empty() {
            panic!(
                "profile '{}' requires model-backed components [{}] but they were not initialized",
                profile.name,
                missing_components.join(", ")
            );
        }
    }

    searcher
}

fn extract_ranked_ids(response: &JsonValue) -> Vec<String> {
    response
        .get("data")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            item.get("unique_id")
                .and_then(JsonValue::as_str)
                .map(ToString::to_string)
        })
        .collect()
}

fn compute_recall_at_k(ranked: &[String], relevance: &HashMap<&str, f64>, k: usize) -> f64 {
    let relevant_total = relevance.values().filter(|&&g| g > 0.0).count();
    if relevant_total == 0 {
        return 0.0;
    }
    let hits = ranked
        .iter()
        .take(k)
        .filter(|id| relevance.get(id.as_str()).copied().unwrap_or(0.0) > 0.0)
        .count();
    hits as f64 / relevant_total as f64
}

fn compute_mrr_at_k(ranked: &[String], relevance: &HashMap<&str, f64>, k: usize) -> f64 {
    for (idx, id) in ranked.iter().take(k).enumerate() {
        if relevance.get(id.as_str()).copied().unwrap_or(0.0) > 0.0 {
            return 1.0 / (idx as f64 + 1.0);
        }
    }
    0.0
}

fn dcg_at_k(ranked: &[String], relevance: &HashMap<&str, f64>, k: usize) -> f64 {
    ranked
        .iter()
        .take(k)
        .enumerate()
        .map(|(idx, id)| {
            let rel = relevance.get(id.as_str()).copied().unwrap_or(0.0);
            if rel <= 0.0 {
                return 0.0;
            }
            let gain = 2.0_f64.powf(rel) - 1.0;
            let discount = (idx as f64 + 2.0).log2();
            gain / discount
        })
        .sum()
}

fn idcg_at_k(relevance: &HashMap<&str, f64>, k: usize) -> f64 {
    let mut rels: Vec<f64> = relevance.values().copied().filter(|g| *g > 0.0).collect();
    rels.sort_by(|a, b| b.partial_cmp(a).unwrap_or(Ordering::Equal));
    rels.into_iter()
        .take(k)
        .enumerate()
        .map(|(idx, rel)| {
            let gain = 2.0_f64.powf(rel) - 1.0;
            let discount = (idx as f64 + 2.0).log2();
            gain / discount
        })
        .sum()
}

fn compute_ndcg_at_k(ranked: &[String], relevance: &HashMap<&str, f64>, k: usize) -> f64 {
    let idcg = idcg_at_k(relevance, k);
    if idcg == 0.0 {
        return 0.0;
    }
    dcg_at_k(ranked, relevance, k) / idcg
}

fn percentile(mut values: Vec<f64>, p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    let idx = ((values.len() - 1) as f64 * p).round() as usize;
    values[idx]
}

fn validate_suite(suite: &EvalSuite, qrels_path: &Path, min_query_count: usize) {
    assert!(
        suite.queries.len() >= min_query_count,
        "qrels '{}' has {} queries; requires >= {}",
        qrels_path.display(),
        suite.queries.len(),
        min_query_count
    );

    let mut seen_query_ids = HashSet::new();
    for query in &suite.queries {
        assert!(
            seen_query_ids.insert(query.id.clone()),
            "qrels '{}' has duplicate query id '{}'",
            qrels_path.display(),
            query.id
        );
        assert!(
            !query.relevant.is_empty(),
            "qrels '{}' query '{}' has no relevant documents",
            qrels_path.display(),
            query.id
        );

        let mut seen_unique_ids = HashSet::new();
        for rel in &query.relevant {
            assert!(
                rel.grade > 0.0,
                "qrels '{}' query '{}' has non-positive grade for '{}'",
                qrels_path.display(),
                query.id,
                rel.unique_id
            );
            assert!(
                seen_unique_ids.insert(rel.unique_id.clone()),
                "qrels '{}' query '{}' has duplicate relevant unique_id '{}'",
                qrels_path.display(),
                query.id,
                rel.unique_id
            );
        }
    }
}

async fn evaluate_profile(
    profile: Profile,
    manifest_path: &Path,
    suite: &EvalSuite,
    top_k: usize,
    allow_download: bool,
    require_models: bool,
) -> ProfileMetrics {
    let searcher = build_searcher(manifest_path, profile, allow_download, require_models);
    let mut per_query = Vec::with_capacity(suite.queries.len());

    for case in &suite.queries {
        let params = SearchParams {
            query: case.query.clone(),
            resource_types: case.resource_types.clone(),
            persona: case.persona.clone(),
            detail: DetailLevel::Standard,
            min_score: None,
            fuzzy: false,
            include_highlights: false,
            include_sql: false,
            explain: false,
            pagination: PaginationParams {
                limit: top_k.max(1),
                offset: 0,
            },
        };

        let start = Instant::now();
        let response = searcher.search(&params).await.unwrap_or_else(|e| {
            panic!(
                "search failed for profile '{}' case '{}' query '{}': {e}",
                profile.name, case.id, case.query
            )
        });
        let latency_ms = start.elapsed().as_secs_f64() * 1000.0;
        let ranked_ids = extract_ranked_ids(&response);

        let relevance: HashMap<&str, f64> = case
            .relevant
            .iter()
            .map(|rel| (rel.unique_id.as_str(), f64::from(rel.grade)))
            .collect();

        let recall_at_k = compute_recall_at_k(&ranked_ids, &relevance, top_k);
        let mrr_at_k = compute_mrr_at_k(&ranked_ids, &relevance, top_k);
        let ndcg_at_k = compute_ndcg_at_k(&ranked_ids, &relevance, top_k);

        per_query.push(QueryMetrics {
            id: case.id.clone(),
            top_hit: ranked_ids.first().cloned(),
            recall_at_k,
            mrr_at_k,
            ndcg_at_k,
            latency_ms,
        });
    }

    let query_count = per_query.len();
    let hit_rate_at_k = if query_count == 0 {
        0.0
    } else {
        per_query.iter().filter(|m| m.mrr_at_k > 0.0).count() as f64 / query_count as f64
    };
    let recall_at_k = if query_count == 0 {
        0.0
    } else {
        per_query.iter().map(|m| m.recall_at_k).sum::<f64>() / query_count as f64
    };
    let mrr_at_k = if query_count == 0 {
        0.0
    } else {
        per_query.iter().map(|m| m.mrr_at_k).sum::<f64>() / query_count as f64
    };
    let ndcg_at_k = if query_count == 0 {
        0.0
    } else {
        per_query.iter().map(|m| m.ndcg_at_k).sum::<f64>() / query_count as f64
    };

    let latencies: Vec<f64> = per_query.iter().map(|m| m.latency_ms).collect();
    let mean_latency_ms = if query_count == 0 {
        0.0
    } else {
        latencies.iter().sum::<f64>() / query_count as f64
    };
    let p95_latency_ms = percentile(latencies, 0.95);

    ProfileMetrics {
        profile: profile.name,
        query_count,
        hit_rate_at_k,
        recall_at_k,
        mrr_at_k,
        ndcg_at_k,
        mean_latency_ms,
        p95_latency_ms,
        per_query,
    }
}

fn manifest_hash_from_snapshot(snapshot: &JsonValue) -> String {
    snapshot
        .get("manifest")
        .and_then(|v| v.get("hash"))
        .and_then(JsonValue::as_str)
        .unwrap_or("")
        .to_string()
}

fn touch_manifest(manifest_path: &Path) {
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(manifest_path)
        .unwrap_or_else(|e| {
            panic!(
                "failed to open manifest '{}' for mutation: {e}",
                manifest_path.display()
            )
        });
    // Trailing whitespace preserves valid JSON while changing the content hash.
    file.write_all(b"\n").unwrap_or_else(|e| {
        panic!(
            "failed to mutate manifest '{}': {e}",
            manifest_path.display()
        )
    });
    file.flush().unwrap_or_else(|e| {
        panic!(
            "failed to flush manifest '{}': {e}",
            manifest_path.display()
        )
    });
}

fn format_status(status: ManifestStatus) -> String {
    match status {
        ManifestStatus::Loading { elapsed_ms } => format!("loading(elapsed_ms={elapsed_ms})"),
        ManifestStatus::Ready { entity_count } => format!("ready(entity_count={entity_count})"),
        ManifestStatus::Refreshing {
            elapsed_ms,
            entity_count,
        } => {
            format!("refreshing(elapsed_ms={elapsed_ms}, entity_count={entity_count})")
        }
        ManifestStatus::Failed { error } => format!("failed(error={error})"),
    }
}

async fn wait_for_hash_change(
    handle: &ManifestSearchHandle,
    old_hash: &str,
    timeout: Duration,
    profile_name: &str,
) {
    let deadline = Instant::now() + timeout;
    loop {
        if Instant::now() >= deadline {
            let status = format_status(handle.status().await);
            let refresh_stats = handle.refresh_stats_snapshot().await;
            panic!(
                "reload timeout for profile '{}' after {}s; status={} refresh_stats={}",
                profile_name,
                timeout.as_secs(),
                status,
                refresh_stats
            );
        }
        if let Ok(searcher) = handle.get().await {
            let snapshot = searcher.health_snapshot().await;
            let hash = manifest_hash_from_snapshot(&snapshot);
            if !hash.is_empty() && hash != old_hash {
                return;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn measure_lifecycle(
    profile: Profile,
    manifest_path: &Path,
    reload_timeout: Duration,
    allow_download: bool,
    require_models: bool,
) -> LifecycleMetrics {
    let temp_dir = tempfile::tempdir().expect("tempdir for lifecycle manifest");
    let local_manifest = temp_dir.path().join("manifest.json");
    fs::copy(manifest_path, &local_manifest).unwrap_or_else(|e| {
        panic!(
            "failed to copy manifest '{}' into lifecycle workspace: {e}",
            manifest_path.display()
        )
    });

    let guard = TestStorageGuard::new();
    let mut cfg = build_profile_config(&local_manifest, profile, &guard);
    cfg.manifest_refresh_secs = 0;

    let startup_start = Instant::now();
    let handle = with_download_guard(allow_download, || ManifestSearchHandle::spawn(cfg));
    let initial = handle
        .wait_ready()
        .await
        .unwrap_or_else(|e| panic!("profile '{}' cold start failed: {e}", profile.name));
    let cold_start_ms = startup_start.elapsed().as_secs_f64() * 1000.0;
    let initial_snapshot = initial.health_snapshot().await;
    if require_models && profile_requires_model_components(profile) {
        let mut missing_components = Vec::new();
        if profile.vector && !initial.vector_search_ready() {
            missing_components.push("vector");
        }
        if profile.sparse && !initial.sparse_search_ready() {
            missing_components.push("sparse");
        }
        if profile.reranker && !initial.reranker_ready() {
            missing_components.push("reranker");
        }

        if !missing_components.is_empty() {
            panic!(
                "profile '{}' requires model-backed components [{}] but they were not initialized",
                profile.name,
                missing_components.join(", ")
            );
        }
    }
    let initial_hash = manifest_hash_from_snapshot(&initial_snapshot);
    assert!(
        !initial_hash.is_empty(),
        "profile '{}' did not expose manifest hash in health snapshot",
        profile.name
    );

    touch_manifest(&local_manifest);

    let reload_start = Instant::now();
    handle
        .reload(&ReloadManifestParams {
            manifest_uri: None,
            manifest_path: None,
            refresh_secs: None,
            storage_instance_id: None,
        })
        .await
        .unwrap_or_else(|e| panic!("profile '{}' reload trigger failed: {e}", profile.name));
    wait_for_hash_change(&handle, &initial_hash, reload_timeout, profile.name).await;
    let reload_swap_ms = reload_start.elapsed().as_secs_f64() * 1000.0;

    LifecycleMetrics {
        profile: profile.name,
        cold_start_ms,
        reload_swap_ms,
    }
}

fn print_report(top_k: usize, lexical: &ProfileMetrics, hybrid: Option<&ProfileMetrics>) {
    println!("\n=== Search Quality Evaluation @k={top_k} ===");
    println!(
        "{:<14} {:>6} {:>10} {:>10} {:>10} {:>10} {:>12} {:>12}",
        "profile", "n", "hit_rate", "recall", "mrr", "ndcg", "mean_ms", "p95_ms"
    );

    let print_row = |m: &ProfileMetrics| {
        println!(
            "{:<14} {:>6} {:>10.4} {:>10.4} {:>10.4} {:>10.4} {:>12.2} {:>12.2}",
            m.profile,
            m.query_count,
            m.hit_rate_at_k,
            m.recall_at_k,
            m.mrr_at_k,
            m.ndcg_at_k,
            m.mean_latency_ms,
            m.p95_latency_ms
        );
    };

    print_row(lexical);
    if let Some(hybrid) = hybrid {
        print_row(hybrid);
        println!(
            "{:<14} {:>6} {:>10.4} {:>10.4} {:>10.4} {:>10.4} {:>12.2} {:>12.2}",
            "delta(h-l)",
            "",
            hybrid.hit_rate_at_k - lexical.hit_rate_at_k,
            hybrid.recall_at_k - lexical.recall_at_k,
            hybrid.mrr_at_k - lexical.mrr_at_k,
            hybrid.ndcg_at_k - lexical.ndcg_at_k,
            hybrid.mean_latency_ms - lexical.mean_latency_ms,
            hybrid.p95_latency_ms - lexical.p95_latency_ms
        );
    }

    println!("\nTop hit per query (lexical):");
    for case in &lexical.per_query {
        println!(
            "  {:<30} top_hit={} recall={:.3} mrr={:.3} ndcg={:.3}",
            case.id,
            case.top_hit.clone().unwrap_or_else(|| "-".to_string()),
            case.recall_at_k,
            case.mrr_at_k,
            case.ndcg_at_k
        );
    }
    if let Some(hybrid) = hybrid {
        println!("\nTop hit per query (hybrid):");
        for case in &hybrid.per_query {
            println!(
                "  {:<30} top_hit={} recall={:.3} mrr={:.3} ndcg={:.3}",
                case.id,
                case.top_hit.clone().unwrap_or_else(|| "-".to_string()),
                case.recall_at_k,
                case.mrr_at_k,
                case.ndcg_at_k
            );
        }
    }
}

fn print_lifecycle_report(lexical: &LifecycleMetrics, hybrid: Option<&LifecycleMetrics>) {
    println!("\n=== Lifecycle Timing (Cold Start + Reload Swap) ===");
    println!(
        "{:<14} {:>16} {:>16}",
        "profile", "cold_start_ms", "reload_swap_ms"
    );

    let print_row = |m: &LifecycleMetrics| {
        println!(
            "{:<14} {:>16.2} {:>16.2}",
            m.profile, m.cold_start_ms, m.reload_swap_ms
        );
    };

    print_row(lexical);
    if let Some(hybrid) = hybrid {
        print_row(hybrid);
        println!(
            "{:<14} {:>16.2} {:>16.2}",
            "delta(h-l)",
            hybrid.cold_start_ms - lexical.cold_start_ms,
            hybrid.reload_swap_ms - lexical.reload_swap_ms
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "manual evaluation harness; run explicitly to compare lexical vs hybrid relevance"]
async fn compare_lexical_vs_hybrid_search_quality() {
    let qrels_path = resolve_qrels_path();
    let suite = load_suite(&qrels_path);
    let min_query_count = env_usize("DBT_NOVA_EVAL_MIN_QUERY_COUNT", 1);
    validate_suite(&suite, &qrels_path, min_query_count);

    let manifest_path = resolve_manifest_path(&suite, &qrels_path);
    assert!(
        manifest_path.exists(),
        "manifest path does not exist: {}",
        manifest_path.display()
    );

    let top_k = env_usize("DBT_NOVA_EVAL_TOP_K", suite.top_k.unwrap_or(10)).max(1);
    let run_hybrid = env_flag("DBT_NOVA_EVAL_ENABLE_HYBRID")
        || std::env::var("DBT_NOVA_EVAL_ENABLE_HYBRID").is_err();
    let run_lifecycle = env_flag("DBT_NOVA_EVAL_ENABLE_LIFECYCLE")
        || std::env::var("DBT_NOVA_EVAL_ENABLE_LIFECYCLE").is_err();
    let allow_embedding_download =
        env_flag_with_default("DBT_NOVA_EVAL_ALLOW_EMBEDDING_DOWNLOAD", true);
    let require_models = env_flag("DBT_NOVA_EVAL_REQUIRE_MODELS");
    let reload_timeout = Duration::from_secs(env_u64("DBT_NOVA_EVAL_RELOAD_TIMEOUT_SECS", 600));

    let lexical = evaluate_profile(
        LEXICAL_PROFILE,
        &manifest_path,
        &suite,
        top_k,
        allow_embedding_download,
        false,
    )
    .await;
    let hybrid = if run_hybrid {
        Some(
            evaluate_profile(
                HYBRID_PROFILE,
                &manifest_path,
                &suite,
                top_k,
                allow_embedding_download,
                require_models,
            )
            .await,
        )
    } else {
        None
    };

    print_report(top_k, &lexical, hybrid.as_ref());
    let lifecycle_lexical = if run_lifecycle {
        Some(
            measure_lifecycle(
                LEXICAL_PROFILE,
                &manifest_path,
                reload_timeout,
                allow_embedding_download,
                false,
            )
            .await,
        )
    } else {
        None
    };
    let lifecycle_hybrid = if run_lifecycle && run_hybrid {
        Some(
            measure_lifecycle(
                HYBRID_PROFILE,
                &manifest_path,
                reload_timeout,
                allow_embedding_download,
                require_models,
            )
            .await,
        )
    } else {
        None
    };
    if let Some(lexical_lifecycle) = &lifecycle_lexical {
        print_lifecycle_report(lexical_lifecycle, lifecycle_hybrid.as_ref());
    }

    if env_flag("DBT_NOVA_EVAL_ASSERT_HYBRID_NONDECREASING")
        && let Some(hybrid) = &hybrid
    {
        assert!(
            hybrid.recall_at_k >= lexical.recall_at_k,
            "hybrid recall_at_k ({:.4}) is below lexical ({:.4})",
            hybrid.recall_at_k,
            lexical.recall_at_k
        );
        assert!(
            hybrid.mrr_at_k >= lexical.mrr_at_k,
            "hybrid mrr_at_k ({:.4}) is below lexical ({:.4})",
            hybrid.mrr_at_k,
            lexical.mrr_at_k
        );
        assert!(
            hybrid.ndcg_at_k >= lexical.ndcg_at_k,
            "hybrid ndcg_at_k ({:.4}) is below lexical ({:.4})",
            hybrid.ndcg_at_k,
            lexical.ndcg_at_k
        );
    }

    if let (Some(min_delta), Some(hybrid)) =
        (env_f64("DBT_NOVA_EVAL_ASSERT_MIN_DELTA_MRR"), &hybrid)
    {
        let delta = hybrid.mrr_at_k - lexical.mrr_at_k;
        assert!(
            delta >= min_delta,
            "expected hybrid mrr delta >= {:.4}, got {:.4}",
            min_delta,
            delta
        );
    }

    if let (Some(min_delta), Some(hybrid)) =
        (env_f64("DBT_NOVA_EVAL_ASSERT_MIN_DELTA_RECALL"), &hybrid)
    {
        let delta = hybrid.recall_at_k - lexical.recall_at_k;
        assert!(
            delta >= min_delta,
            "expected hybrid recall delta >= {:.4}, got {:.4}",
            min_delta,
            delta
        );
    }

    if let Some(max_ms) = env_f64("DBT_NOVA_EVAL_ASSERT_MAX_COLD_START_MS") {
        if let Some(lexical_lifecycle) = &lifecycle_lexical {
            assert!(
                lexical_lifecycle.cold_start_ms <= max_ms,
                "lexical cold start exceeded max: {:.2} > {:.2} ms",
                lexical_lifecycle.cold_start_ms,
                max_ms
            );
        }
        if let Some(hybrid_lifecycle) = &lifecycle_hybrid {
            assert!(
                hybrid_lifecycle.cold_start_ms <= max_ms,
                "hybrid cold start exceeded max: {:.2} > {:.2} ms",
                hybrid_lifecycle.cold_start_ms,
                max_ms
            );
        }
    }

    if let Some(max_ms) = env_f64("DBT_NOVA_EVAL_ASSERT_MAX_RELOAD_SWAP_MS") {
        if let Some(lexical_lifecycle) = &lifecycle_lexical {
            assert!(
                lexical_lifecycle.reload_swap_ms <= max_ms,
                "lexical reload swap exceeded max: {:.2} > {:.2} ms",
                lexical_lifecycle.reload_swap_ms,
                max_ms
            );
        }
        if let Some(hybrid_lifecycle) = &lifecycle_hybrid {
            assert!(
                hybrid_lifecycle.reload_swap_ms <= max_ms,
                "hybrid reload swap exceeded max: {:.2} > {:.2} ms",
                hybrid_lifecycle.reload_swap_ms,
                max_ms
            );
        }
    }
}
