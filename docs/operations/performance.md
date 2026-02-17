# Performance Tuning

## Search Latency

### Benchmark Profiles (Reproducible)

The table below is generated from the `search_eval` harness on the fixture
manifest/qrels and should be treated as a profile reference, not a hard SLA.

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

### Reducing Latency

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

### Baseline Requirements

| Manifest Size | Memory |
|---------------|--------|
| 1,000 entities | ~200MB |
| 10,000 entities | ~800MB |
| 50,000 entities | ~3GB |

### Reducing Memory

1. Disable unused features:
   ```
   DBT_NOVA_SEARCH_ENABLE_SPARSE=false
   ```

2. Limit cache sizes:
   ```
   DBT_NOVA_COLUMN_LINEAGE_MAX_CANDIDATES=5000
   ```

## Notes

- The first load builds indexes and embeddings; subsequent runs reuse rkyv caches.
- Latency depends on manifest size, enabled features, and storage IO.

## Measuring Embedding Uplift

Use the manual search evaluation harness to quantify quality and latency deltas
between lexical-only and hybrid search.

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
