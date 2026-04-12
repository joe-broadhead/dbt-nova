# Nova Search Ranking (Design Spec)

This document defines how Nova meta is indexed and ranked to improve discoverability
without increasing metadata maintenance overhead.

## Goals

- Make canonical datasets and metrics surface first.
- Allow analysts to search by business terms (synonyms, use cases, domains).
- Support KPI discovery without creating duplicate per‑slice metric models.

## Indexed Nova Fields

Nova meta is indexed into dedicated search fields:

| Meta location | Field | Purpose |
| --- | --- | --- |
| `meta.nova.synonyms` | `nova_synonyms` | Business terms for the dataset |
| `meta.nova.domains` | `nova_domains` | Domain routing (e.g., web, ecommerce) |
| `meta.nova.use_cases` | `nova_use_cases` | Intent queries (e.g., weekly_report) |
| `meta.nova.measures[].name` + `synonyms[]` | `nova_measures` | Measure discovery (sessions, revenue) |
| `meta.nova.measures[].description` + `expression` + `field` | `nova_measures` | Measure definition keywords and physical column hints |
| `meta.nova.metric(s).name` + `synonyms[]` | `nova_metric` | KPI discovery (conversion_rate) |
| `meta.nova.metric(s).description` + `expression` | `nova_metric` | Metric definition keywords |
| `meta.nova.governance.sensitivity` | `nova_sensitivity` | Governance discovery (none/public/internal/confidential/low/medium/high/restricted) |
| `meta.nova.governance.pii` | `nova_pii` | PII signal (pii + level) |
| `meta.nova.governance.compliance[]` | `nova_compliance` | Compliance frameworks (gdpr, soc2, hipaa) |

Column‑level `meta.nova.synonyms`, `semantic_type`, and `example_values` are appended
to the `columns` search field to improve column discovery (e.g., “UK” → `country_code`).

## Default Boosts (Config)

These values are tuned for high signal with minimal noise:

```
DBT_NOVA_ALIAS_BOOST=18.0
DBT_NOVA_NAME_BOOST=12.0
DBT_NOVA_DESCRIPTION_BOOST=6.0
DBT_NOVA_COLUMN_BOOST=4.0
DBT_NOVA_TAG_BOOST=3.0
DBT_NOVA_PATH_BOOST=2.0
DBT_NOVA_CODE_BOOST=1.5

DBT_NOVA_META_SYNONYMS_BOOST=7.0
DBT_NOVA_META_MEASURES_BOOST=8.0
DBT_NOVA_META_METRIC_BOOST=10.0
DBT_NOVA_META_SENSITIVITY_BOOST=6.0
DBT_NOVA_META_PII_BOOST=8.0
DBT_NOVA_META_COMPLIANCE_BOOST=6.0
DBT_NOVA_META_DOMAINS_BOOST=4.0
DBT_NOVA_META_USE_CASES_BOOST=4.0
DBT_NOVA_SEARCH_STAGING_DEBOOST_FACTOR=0.6
DBT_NOVA_SEARCH_ANALYST_CANDIDATE_FALSE_DEBOOST_FACTOR=0.45
DBT_NOVA_SEARCH_ENGINEER_CANDIDATE_FALSE_DEBOOST_FACTOR=1.0
DBT_NOVA_SEARCH_GOVERNANCE_CANDIDATE_FALSE_DEBOOST_FACTOR=1.0
DBT_NOVA_SEARCH_MEASURE_MATCH_MULTIPLIER=1.15
DBT_NOVA_SEARCH_METRIC_MATCH_MULTIPLIER=1.20
DBT_NOVA_SEARCH_ANALYST_SEMANTIC_MATCH_MULTIPLIER=1.35
DBT_NOVA_SEARCH_NON_ANALYST_SEMANTIC_MATCH_MULTIPLIER=1.05
DBT_NOVA_SEARCH_SEMANTIC_NAME_MATCH_MULTIPLIER=1.12
DBT_NOVA_SEARCH_SEMANTIC_SYNONYM_MATCH_MULTIPLIER=1.08
DBT_NOVA_SEARCH_SEMANTIC_DEFINITION_MATCH_MULTIPLIER=1.03
DBT_NOVA_SEARCH_SEMANTIC_CANONICAL_MATCH_MULTIPLIER=1.25
DBT_NOVA_SEARCH_SEMANTIC_CANONICAL_MATCH_BONUS=1.5
DBT_NOVA_SEARCH_SYNONYM_MATCH_MULTIPLIER=1.20
DBT_NOVA_SEARCH_CANONICAL_MATCH_MULTIPLIER=1.08
DBT_NOVA_SEARCH_CANONICAL_META_MATCH_MULTIPLIER=1.35
DBT_NOVA_SEARCH_CANONICAL_META_MATCH_BONUS=2.5
DBT_NOVA_SEARCH_ENGINEER_EXACT_MATCH_MULTIPLIER=2.0
```

## Ranking Behavior

1) **Exact business terms** (synonyms/metric names) score close to `name` matches.  
2) **Nova measures and metrics** are next‑highest so KPI‑driven queries surface
   the canonical model even without a standalone dbt metric resource.
3) **Domains/use_cases** provide intent routing but stay below synonyms/measures.
4) `description`, `columns`, `tags`, `path`, and `code` remain as backstops.
5) **Staging-layer models** are de‑boosted so curated models rank higher.
   This deboost is layer-rule driven (`DBT_NOVA_LAYER_RULES_JSON`) and applies
   when the resolved layer is `staging`, `stage`, or `stg`.
6) **Persona candidate hints** can de‑boost models that are still useful for
   engineering/governance discovery but should not dominate analyst discovery.
   Set `meta.nova.search.candidates.analyst: false` to mark helper or ops models
   as lower-signal for analysts while keeping them searchable.
7) **Query-aware semantic matching** prefers Nova measure/metric name and synonym
   matches over field/description/expression matches, and boosts semantic matches
   more strongly for `persona=analyst` than other personas.
8) **Canonical semantic matches** boost entities when the matched
   `meta.nova.measures[]` or `meta.nova.metric(s)` definition is canonical.
   Effective canonicality comes from either the matched semantic entry
   (`canonical: true`) or the owning entity’s `meta.nova.canonical: true`.
9) **Entity-level Nova meta re‑ranking** still boosts canonical models when query
   tokens match `meta.nova.measures`, `meta.nova.metric(s)`, or `meta.nova.synonyms`.
10) **Governance terms** (sensitivity/pii/compliance) get elevated when present.
11) **Analyst semantic readiness** adds a bounded multiplier for entities with metric/measure
   definitions, grain/time-field metadata, and query-aligned dimensions. Non-semantic entities
   receive a slight de-boost.
12) **Analyst near-tie hinting** emits `analysis_hints` when top candidates are within a small score
   gap, prompting `get_entity`/`get_context` validation before SQL generation.

## Persona Candidate Metadata

Use `meta.nova.search.candidates` when a model should remain searchable but
should not rank as highly for a specific persona:

```yaml
meta:
  nova:
    search:
      candidates:
        analyst: false
```

Semantics:
- Missing candidate metadata defaults to `true`.
- `false` is a ranking hint, not a filter.
- Exact matches on name, alias, unique id, or file path bypass the deboost.
- Current defaults only deboost analysts; engineer and governance defaults remain neutral.

## Canonical Semantic Metadata

You can mark the owning entity as canonical:

```yaml
meta:
  nova:
    canonical: true
```

and/or mark an individual matched measure or metric as canonical:

```yaml
meta:
  nova:
    measures:
      - name: gmv
        canonical: true
```

```yaml
meta:
  nova:
    metrics:
      - name: average_order_value
        canonical: true
```

This is useful when the same business term appears in multiple tables, but only
one definition should surface first for analyst discovery.

## Scoring Pipeline (Lexical + Vector + RRF + Reranker)

Nova fuses multiple retrieval channels into a single ranked list:

1) **Lexical search** uses Tantivy BM25 with field boosts.
2) **Vector** (dense) and **sparse** retrievers produce their own ranked lists.
3) **RRF fusion** combines all retrievers into a single score.
4) **Reranker** (if enabled) rescales the top‑N results using a cross‑encoder.

### Weighted RRF formula

For each retriever list, Nova assigns a score based on rank position:

```
rrf_score = weight / (k + rank + 1)
```

- `weight` is the persona‑specific weight for that retriever (e.g., `vector`, `bm25`, `sparse`).
- `k` is the RRF smoothing constant (`DBT_NOVA_SEARCH_RRF_K`).
- `rank` is 0‑based within that retriever list.

Final RRF scores are summed across retrievers to produce the fused ranking.

### Reranker adjustments

If `DBT_NOVA_SEARCH_ENABLE_RERANKER=true`, Nova reranks the top‑N fused hits
(`DBT_NOVA_SEARCH_RERANK_TOP_N`) using a cross‑encoder model. The reranker
reorders the top‑N results while keeping the remainder of the list in fused order.


## Persona Weights

Persona-specific weights (analyst/engineer/governance) are applied after base scoring.
They amplify different signals without changing the underlying Nova metadata fields.

Current defaults (configurable via the persona weight env vars below):

| Signal | Analyst | Engineer | Governance | Default |
| --- | --- | --- | --- | --- |
| bm25 | 1.0 | 1.5 | 1.2 | 1.0 |
| ngram | 1.1 | 1.0 | 1.0 | 1.0 |
| fuzzy | 1.0 | 0.8 | 1.0 | 1.0 |
| vector | 1.5 | 0.8 | 1.0 | 1.0 |
| sparse | 1.2 | 1.0 | 1.3 | 1.0 |
| measures | 1.3 | 0.9 | 0.9 | 1.0 |
| metrics | 1.4 | 0.9 | 0.9 | 1.0 |
| synonyms | 1.2 | 0.9 | 1.0 | 1.0 |
| docs | 1.2 | 0.9 | 1.3 | 1.0 |
| tests | 1.2 | 1.0 | 1.4 | 1.0 |
| tags | 1.0 | 0.9 | 1.4 | 1.0 |
| path | 0.9 | 1.3 | 1.0 | 1.0 |

If you need different weightings, override via env:

```
DBT_NOVA_SEARCH_PERSONA_ANALYST_WEIGHTS="vector=1.6,docs=1.3"
DBT_NOVA_SEARCH_PERSONA_ENGINEER_WEIGHTS="bm25=1.6,path=1.4"
DBT_NOVA_SEARCH_PERSONA_GOVERNANCE_WEIGHTS="tags=1.6,tests=1.5"
DBT_NOVA_SEARCH_PERSONA_DEFAULT_WEIGHTS="bm25=1.0"
```

Any weight not listed remains at its default.

## Persona Resource-Type Priorities

After lexical/vector/sparse fusion, Nova applies persona-aware resource-type multipliers to reduce
cross-persona noise:

- **Analyst**: boosts `semantic_model`, `saved_query`, and curated `model` results,
  applies a smaller boost to standalone dbt `metric` resources, and relies on
  matched Nova metric/measure definitions to surface canonical analyst-ready models first.
- **Engineer**: boosts `model`, `source`, `test`, and `macro`; slightly de-boosts pure business
  artifacts (`metric`, `semantic_model`, `saved_query`).
- **Governance**: strongly boosts `test`, and also boosts `model`, `source`, and `exposure`;
  de-boosts `macro`.

These multipliers are deterministic defaults in code and complement persona retriever weights.
The goal is to keep top-K results high-signal for each agent persona without requiring strict
resource-type filters on every call.

## Why This Avoids Metric Model Explosion

With `nova_metric` and `nova_measures` indexed:

- Query: “conversion rate” → matches `metric(s).synonyms` in canonical base model.
- Query: “sessions” → matches measure name/synonyms.

Analysts can then apply filters (web/app, country, device) at query time instead of
having one metric model per slice.

## Search Payload Preview

When a query matches Nova metrics or measures, standard search results include a
compact `semantic_preview`:

- `matched_measures`
- `matched_metrics`
- per-match fields:
  - `name`
  - `description`
  - `expression`
  - `field` (measures only)
  - `canonical`
  - `match_type`

This lets agents see the semantic definition in the search result itself before
calling `get_context`.

## When to Add Metric Models Anyway

Metric models are still recommended for:

- Cross‑model joins or complex logic.
- Executive or compliance KPIs that must be enforced centrally.

---

## See Also

- [Tools Reference](../api/tools.md) - Search tool documentation
- [Configuration Reference](../configuration/reference.md) - Environment variables
- [Search Defaults](../configuration/search-defaults.md) - Default search behavior
- [Personas](../personas/overview.md) - Persona-specific search behavior
