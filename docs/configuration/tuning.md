# Search Tuning Reference

This document collects the primary “magic numbers” used in ranking and
performance tuning. The defaults are chosen for high signal with minimal noise.

## Field Boost Values

| Value | Field | Rationale |
|-------|-------|-----------|
| `18.0` | `alias_boost` | Highest priority; exact alias matches indicate strong relevance |
| `12.0` | `name_boost` | Model/source names are primary identifiers |
| `10.0` | `nova_metric` | KPI/metric names have high business value |
| `8.0` | `nova_measures` | Measure definitions support analyst workflows |
| `7.0` | `nova_synonyms` | Business term aliases improve discoverability |
| `6.0` | `description` | Descriptions provide context but may be verbose |
| `4.0` | `column` | Column names are often technical, lower priority |
| `3.0` | `tag` | Tags are useful but often inconsistent |
| `2.0` | `path` | File paths rarely match user intent |
| `1.5` | `code` | SQL code has high false-positive rate |

## Hybrid Search Constants

| Value | Setting | Rationale |
|-------|---------|-----------|
| `60.0` | `rrf_k` | Standard RRF smoothing constant |
| `3` | `overfetch_multiplier` / `DBT_NOVA_SEARCH_RRF_OVERFETCH` | Fetch 3x results for better reranking |
| `200` | `vector_top_k` | Balance recall and latency |
| `20` | `rerank_top_n` | Cross-encoder is expensive; limit to top results |

## Fuzzy Matching

| Value | Setting | Rationale |
|-------|---------|-----------|
| `0.75` | `levenshtein_threshold` | 75% similarity catches typos without false positives |
| `4` | `fuzzy_min_length` | Avoid fuzzy matching for very short tokens |
| `2` | `fuzzy_max_distance` | Allow up to 2 edits |

## Scoring Adjustments

| Value | Setting | Rationale |
|-------|---------|-----------|
| `0.6` | `DBT_NOVA_SEARCH_STAGING_DEBOOST_FACTOR` | De-boost entities mapped to a staging layer (`staging`/`stage`/`stg`) |
| `1.15` | `DBT_NOVA_SEARCH_MEASURE_MATCH_MULTIPLIER` | Boost results whose `meta.nova.measures` align with query terms |
| `1.2` | `DBT_NOVA_SEARCH_METRIC_MATCH_MULTIPLIER` | Boost KPI/metric term matches |
| `1.2` | `DBT_NOVA_SEARCH_SYNONYM_MATCH_MULTIPLIER` | Boost business synonym matches |
| `true` | `DBT_NOVA_SEARCH_PHRASE_BOOST` | Prefer exact multi-word business phrases and high single-field query coverage |
| `1.08` | `DBT_NOVA_SEARCH_CANONICAL_MATCH_MULTIPLIER` | Baseline uplift for canonical entities |
| `1.35` | `DBT_NOVA_SEARCH_CANONICAL_META_MATCH_MULTIPLIER` | Extra uplift when canonical + meta terms both match |
| `2.5` | `DBT_NOVA_SEARCH_CANONICAL_META_MATCH_BONUS` | Additive bonus for strong canonical/meta alignment |
| `2.0` | `DBT_NOVA_SEARCH_ENGINEER_EXACT_MATCH_MULTIPLIER` | Engineer bias toward exact ID/name matches |

## Analyst Semantic Multipliers

| Value | Setting |
|-------|---------|
| `1.09` | `DBT_NOVA_SEARCH_ANALYST_METRIC_DEF_MULTIPLIER` |
| `1.05` | `DBT_NOVA_SEARCH_ANALYST_MEASURE_DEF_MULTIPLIER` |
| `1.05` | `DBT_NOVA_SEARCH_ANALYST_GRAIN_MULTIPLIER` |
| `1.03` | `DBT_NOVA_SEARCH_ANALYST_TIME_FIELD_MULTIPLIER` |
| `1.03` | `DBT_NOVA_SEARCH_ANALYST_DIM_OVERLAP_ONE_MULTIPLIER` |
| `1.06` | `DBT_NOVA_SEARCH_ANALYST_DIM_OVERLAP_TWO_MULTIPLIER` |
| `1.09` | `DBT_NOVA_SEARCH_ANALYST_DIM_OVERLAP_THREE_PLUS_MULTIPLIER` |
| `0.96` | `DBT_NOVA_SEARCH_ANALYST_MISSING_METRIC_MEASURE_MULTIPLIER` |
| `0.97` | `DBT_NOVA_SEARCH_ANALYST_MISSING_GRAIN_MULTIPLIER` |
| `0.85` | `DBT_NOVA_SEARCH_ANALYST_MIN_MULTIPLIER` |
| `1.35` | `DBT_NOVA_SEARCH_ANALYST_MAX_MULTIPLIER` |
