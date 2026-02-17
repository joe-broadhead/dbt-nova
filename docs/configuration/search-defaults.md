# Search Defaults

This document describes the **default search configuration** (the behavior you
get out‑of‑box unless environment variables override it).

## Enabled by Default

### Lexical Search (Tantivy)
- **BM25 full‑text** search
- **Query parser** with boolean/phrase/field queries
- **N‑grams**: `min=3`, `max=3`, `boost=0.35`
- **Fuzzy matching** (when request `fuzzy=true`):
  - `min_length=4`, `mid_length=7`, `max_distance=2`
- **Highlights**:
  - format: `text`
  - max chars: `240`
  - max fields: `5`
- **Suggestions** (“did you mean”)

### Hybrid Search
- **RRF fusion** (`k=60`, `overfetch=3`)
- **Dense vectors**: enabled
  - model: `intfloat/multilingual-e5-base`
  - top‑K: `200`
  - ANN buckets: enabled
- **Sparse vectors**: enabled (SPLADE)
  - top‑K: `200`
- **Cross‑encoder reranker**: enabled
  - model: `jinaai/jina-reranker-v2-base-multilingual`
  - rerank top‑N: `20`

### Persona‑aware Ranking
If `persona` is provided, ranking weights are tuned for:
`analyst`, `engineer`, `governance`.

`DBT_NOVA_SEARCH_DEFAULT_PERSONA` can set a default when `persona` is omitted.

### Nova Meta Boosting
Search boosts on:
- `meta.nova.synonyms`
- `meta.nova.domains`
- `meta.nova.use_cases`
- `meta.nova.measures`
- `meta.nova.metric(s)`
- `meta.nova.governance` (sensitivity, pii, compliance)

## Off by Default

- **Vector quantization** (8‑bit): `DBT_NOVA_SEARCH_ENABLE_VECTOR_QUANTIZATION=false`

## Where to Configure

See:
- [Configuration Reference](reference.md)
- [Nova Search Ranking](../features/search-ranking.md)

## Default SearchConfig (Reference)

This mirrors `SearchConfig::default()` from `src/config/search.rs`.
Canonical defaults are also captured in `docs/config_defaults.json`
(generated via `scripts/update_config_reference.sh`).

### Core Limits

| Setting | Default | Description |
|---------|---------|-------------|
| `default_limit` | 50 | Results per page when not specified |
| `max_page_size` | 2000 | Maximum results per page |
| `max_query_length` | 2000 | Maximum search query length |
| `search_timeout_ms` | 30000 | Search timeout (30 seconds) |

### Field Boosts

| Field | Boost | Description |
|-------|-------|-------------|
| `alias` | 18.0 | Highest priority for exact alias matches |
| `name` | 12.0 | Model/source name matches |
| `nova_metric` | 10.0 | KPI names and synonyms |
| `nova_measures` | 8.0 | Measure definitions |
| `nova_synonyms` | 7.0 | Business term aliases |
| `description` | 6.0 | Entity descriptions |
| `column` | 4.0 | Column names and descriptions |
| `tag` | 3.0 | dbt tags |
| `path` | 2.0 | File paths |
| `code` | 1.5 | SQL code (lowest priority) |

### Hybrid Search

| Setting | Default | Description |
|---------|---------|-------------|
| `enable_rrf` | true | Reciprocal Rank Fusion enabled |
| `rrf_k` | 60 | RRF smoothing constant |
| `enable_vector_search` | true | Dense vector search enabled |
| `vector_top_k` | 200 | Vector candidates before fusion |
| `enable_sparse_search` | true | SPLADE sparse vectors enabled |
| `sparse_top_k` | 200 | Sparse candidates before fusion |
| `enable_reranker` | true | Cross-encoder reranker enabled |
| `rerank_top_n` | 20 | Results to rerank |

### Models

| Setting | Default |
|---------|---------|
| `embedding_model` | `intfloat/multilingual-e5-base` |
| `reranker_model` | `jinaai/jina-reranker-v2-base-multilingual` |

!!! tip "Full Reference"
    For all configuration options including environment variables, see
    [Configuration Reference](reference.md) and [Search Ranking](../features/search-ranking.md).
