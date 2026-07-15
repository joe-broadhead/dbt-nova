# Performance Tuning

## Default Lexical Scale Guard

Nova's v0.0.x production promise is fast default metadata discovery over a dbt
manifest. Vector search, sparse search, and reranking are optional semantic
search components; they are measured separately and are not part of the default
CI scale guard.

The bounded lexical guard generates a synthetic multi-package dbt manifest and
measures manifest load, lexical search, indicator inventory, column inventory,
context lookup, and health/readiness snapshot paths without warehouse
credentials or model downloads.

Command:

```bash
scripts/check_lexical_scale.sh
```

Useful local size overrides:

```bash
DBT_NOVA_SCALE_MODELS=5000 \
DBT_NOVA_SCALE_PACKAGES=5 \
DBT_NOVA_SCALE_COLUMNS=10 \
DBT_NOVA_SCALE_REF_FANOUT=3 \
DBT_NOVA_SCALE_LOAD_ITERATIONS=2 \
DBT_NOVA_SCALE_ITERATIONS=3 \
scripts/check_lexical_scale.sh
```

Captured: 2026-07-12 on Darwin arm64, Apple M5, 24 GB RAM,
`cargo test --locked --no-default-features`, vector/sparse/reranker disabled.

| models | packages | columns/model | ref fanout | indicators | load p50/p95 ms | search p50/p95 ms | indicator inventory p50/p95 ms | column inventory p50/p95 ms | context p50/p95 ms | health p50/p95 ms |
|--------|----------|---------------|------------|------------|-----------------|-------------------|---------------------------------|-----------------------------|-------------------|------------------|
| 300 | 3 | 8 | 2 | 30 | 751 / 779 | 47 / 51 | 7 / 7 | 12 / 12 | 2 / 2 | 0 / 0 |
| 1,000 | 5 | 10 | 3 | 100 | 2,058 / 2,837 | 120 / 135 | 24 / 25 | 50 / 50 | 3 / 3 | 0 / 0 |
| 5,000 | 5 | 10 | 3 | 500 | 9,744 / 10,233 | 410 / 594 | 140 / 159 | 230 / 255 | 2 / 3 | 0 / 0 |

CI runs the 300-model guard with conservative thresholds:

| Path | CI p95 threshold |
|------|------------------|
| manifest load | 30,000 ms |
| lexical search | 2,500 ms |
| indicator inventory | 2,500 ms |
| column inventory | 2,500 ms |
| get_context | 2,500 ms |
| health snapshot | 1,000 ms |

The guard is intentionally a regression tripwire, not a public latency SLA. If a
threshold needs to move, rerun the harness, update the captured baseline here,
and explain the workload or machine change in the PR.

### Memory Notes

The lexical guard does not assert RSS because runner accounting varies across
macOS and Linux. It does avoid optional model downloads and semantic cache
warmup, so it is a clean signal for manifest parsing, indexing, and scan paths.
For local sizing, wrap the script in your platform's time utility and record
maximum resident memory next to the latency table:

```bash
# Linux
/usr/bin/time -v scripts/check_lexical_scale.sh

# macOS
/usr/bin/time -l scripts/check_lexical_scale.sh
```

Use `cargo bench --bench search_bench` for deeper local microbenchmarks. For
large hosted manifests, measure with the same semantic search components,
storage backend, and artifact reuse posture that production will use.

## Optional Semantic Search Evaluation

Optional vector, sparse, and reranker search is characterized separately from
the default lexical scale guard. The table below is generated from the
`search_eval` harness on the fixture manifest/qrels and should be treated as a
profile reference, not a hard SLA.

Nightly/manual automation lives in
`.github/workflows/hybrid-search-characterization.yml`. It runs outside PR CI,
restores a model cache when available, uploads the raw eval log plus
`/usr/bin/time -v` resource usage, and records quality, latency, lifecycle, and
maximum RSS snippets in the workflow summary. If model assets are unavailable
and downloads are not explicitly allowed, the workflow records lexical-only
output and marks the hybrid profile as skipped instead of making PR or release
CI flaky.

Command:

```bash
DBT_NOVA_EVAL_ENABLE_HYBRID=1 \
DBT_NOVA_EVAL_ENABLE_LIFECYCLE=1 \
cargo test --locked --test search_eval compare_lexical_vs_hybrid_search_quality -- --ignored --nocapture
```

For CI smoke and other network-restricted runs:

```bash
DBT_NOVA_EVAL_ENABLE_HYBRID=0 \
DBT_NOVA_EVAL_ENABLE_LIFECYCLE=0 \
DBT_NOVA_EVAL_ALLOW_EMBEDDING_DOWNLOAD=0 \
cargo test --locked --test search_eval compare_lexical_vs_hybrid_search_quality -- --ignored --nocapture
```

Captured: 2026-02-07 (fixture workload, `k=10`, 10 evaluation queries)

| Profile | hit_rate | recall | mrr | ndcg | mean_ms | p95_ms |
|---------|----------|--------|-----|------|---------|--------|
| lexical_only | 1.0000 | 1.0000 | 0.8667 | 0.8808 | 15.35 | 22.61 |
| hybrid | 1.0000 | 1.0000 | 1.0000 | 0.9808 | 342.03 | 390.72 |
| delta (hybrid - lexical) | 0.0000 | 0.0000 | 0.1333 | 0.1000 | 326.68 | 368.11 |

Lifecycle timings from the same run:

| Profile | cold_start_ms | reload_swap_ms |
|---------|---------------|----------------|
| lexical_only | 599.68 | 713.92 |
| hybrid | 5015.34 | 5927.24 |
| delta (hybrid - lexical) | 4415.66 | 5213.31 |

### Reducing Optional Semantic Search Latency

1. **Disable reranker** for interactive search:
   ```
   DBT_NOVA_SEARCH_ENABLE_RERANKER=false
   ```

2. **Reduce vector candidates**:
   ```
   DBT_NOVA_SEARCH_VECTOR_TOP_K=100
   ```

3. **Enable quantization** for large manifests:
   ```
   DBT_NOVA_SEARCH_ENABLE_VECTOR_QUANTIZATION=true
   ```

## Memory Usage

### Reducing Optional Semantic Search Memory

1. Disable unused features:
   ```
   DBT_NOVA_SEARCH_ENABLE_SPARSE=false
   ```

2. Limit cache sizes:
   ```
   DBT_NOVA_COLUMN_LINEAGE_MAX_CANDIDATES=5000
   ```

## Notes

- The first load builds indexes and, when enabled, optional semantic search
  artifacts; subsequent runs reuse rkyv caches.
- Latency depends on manifest size, enabled features, and storage IO.

## Measuring Optional Semantic Search Uplift

Use the manual search evaluation harness to quantify quality and latency deltas
between lexical-only search and optional semantic search components.

### Evaluate on fixture qrels

```bash
DBT_NOVA_EVAL_ALLOW_EMBEDDING_DOWNLOAD=1 \
DBT_NOVA_EVAL_REQUIRE_MODELS=1 \
cargo test --test search_eval -- --ignored --nocapture
```

The report includes:

- `hit_rate@k`
- `recall@k`
- `mrr@k`
- `ndcg@k`
- mean and p95 latency
- cold startup time
- reload swap time (time from reload trigger until new manifest hash becomes active)

### Evaluate on your manifest/qrels

```bash
DBT_NOVA_EVAL_MANIFEST_PATH=/path/to/manifest.json \
DBT_NOVA_EVAL_QRELS_PATH=/path/to/qrels.json \
DBT_NOVA_EVAL_EMBEDDINGS_CACHE_DIR="$HOME/.dbt-nova-models" \
DBT_NOVA_EVAL_ALLOW_EMBEDDING_DOWNLOAD=1 \
DBT_NOVA_EVAL_TOP_K=10 \
DBT_NOVA_EVAL_MIN_QUERY_COUNT=25 \
cargo test --test search_eval -- --ignored --nocapture
```

For large manifests you can increase reload timing timeout:

```bash
DBT_NOVA_EVAL_RELOAD_TIMEOUT_SECS=1200 \
cargo test --test search_eval -- --ignored --nocapture
```

Disable lifecycle timing (quality-only run):

```bash
DBT_NOVA_EVAL_ENABLE_LIFECYCLE=0 \
DBT_NOVA_EVAL_ALLOW_EMBEDDING_DOWNLOAD=1 \
cargo test --test search_eval -- --ignored --nocapture
```

### Optional assertions (for CI gates)

```bash
DBT_NOVA_EVAL_ASSERT_HYBRID_NONDECREASING=1 \
DBT_NOVA_EVAL_ASSERT_MIN_DELTA_MRR=0.02 \
DBT_NOVA_EVAL_ASSERT_MIN_DELTA_RECALL=0.03 \
DBT_NOVA_EVAL_ASSERT_MAX_COLD_START_MS=60000 \
DBT_NOVA_EVAL_ASSERT_MAX_RELOAD_SWAP_MS=90000 \
DBT_NOVA_EVAL_ALLOW_EMBEDDING_DOWNLOAD=1 \
DBT_NOVA_EVAL_REQUIRE_MODELS=1 \
cargo test --test search_eval -- --ignored --nocapture
```

### qrels format

`tests/fixtures/search_eval_qrels.json` is the reference shape.
Each query defines expected relevant `unique_id`s with optional graded relevance.

Recommended for release evaluation:

- at least 25-50 queries
- no duplicate query IDs
- no duplicate relevant `unique_id` entries within a query
- positive relevance grades only
