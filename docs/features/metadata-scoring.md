# Metadata Quality Scoring

The `get_metadata_score` tool evaluates dbt entities and returns a 0–100
metadata quality score, plus a letter grade, per category breakdowns, and
optional improvement recommendations.

It is designed for:
- **Analysts**: discoverability and semantic richness
- **Engineers**: test coverage and structural quality
- **Governance**: sensitivity / PII / compliance metadata

## Scopes

`scope` controls the level of scoring:

- `entity` — score a single entity (`id_or_name` required)
- `column` — score each column on an entity (`id_or_name` required)
- `project` — score multiple entities across the project

## Request Parameters

```json
{
  "id_or_name": "model.jaffle_shop.orders",
  "resource_type": "model",
  "persona": "analyst",
  "scope": "entity",
  "include_breakdown": true,
  "include_recommendations": true,
  "resource_types": ["model", "source"],
  "limit": 1000,
  "offset": 0
}
```

Notes:
- `persona` is optional (`analyst`, `engineer`, `governance`, default).
- `resource_types` selects entities for `scope=project`.
- `limit` and `offset` page the returned project entity sample, not the aggregate project score.
- Default project `limit` is **1000**.

## Response Structure

```json
{
  "success": true,
  "data": {
    "unique_id": "model.jaffle_shop.orders",
    "scope": "entity",
    "persona": "analyst",
    "overall_score": 72,
    "grade": "C",
    "scoring_contract": { "schema_version": "metadata_score_contract.v2" },
    "categories": {
      "documentation": { "score": 85, "weight": 0.20, "weighted": 17.0 },
      "semantic": { "score": 65, "weight": 0.45, "weighted": 29.25 },
      "governance": { "score": 40, "weight": 0.15, "weighted": 6.0 },
      "quality": { "score": 98, "weight": 0.20, "weighted": 19.6 }
    },
    "diagnostics": [ /* machine-readable scoring evidence */ ],
    "breakdown": { /* per-check detail */ },
    "recommendations": [ /* suggestions */ ]
  }
}
```

For `scope=project`, the tool returns an aggregate overall score across all
matching entities plus a deterministic paged sample of entity scores. It marks
the response as `truncated` when more sample rows are available. It also returns
`quality_summary.test_coverage`, aggregated across all matching entities.

All scopes include `scoring_contract.schema_version:
"metadata_score_contract.v2"` so agents can explain scores without reading
source code. The contract includes grade bands, description tiers, array-count
tiers, canonical grain shape, declared-grain evidence, resource-type
expectations, and the primary-key integrity evidence rule. Set
`DBT_NOVA_METADATA_SCORE_CONTRACT_VERSION=v1` only when a downstream gate needs
the legacy contract label during migration.

Nova also scores derived semantic metadata from dbt Semantic Layer / MetricFlow
artifacts. Manifest `metrics` and `semantic_models` contribute Nova metric,
measure, and grain signals even when the project has not duplicated them under
`meta.nova`. Explicit `meta.nova` fields override or extend the derived values.

If `catalog.json` is configured or auto-discovered, column quality scoring sees
catalog-backed `data_type` values. Type mismatches, catalog-only columns, and
declared columns absent from catalog remain visible in `catalog_drift` fields on
`get_columns`/`get_context` payloads for governance follow-up.

Entity and column responses include `diagnostics` when Nova can explain partial
or missing credit. Diagnostics are deterministic JSON rows with `code`,
`category`, `field`, observed values, expected thresholds, and a short message.
Common diagnostic codes:

- `description_tier_progress`: shows observed character count, the 50-character
  good-enough threshold, and the 100-character full-credit threshold
- `array_tier_progress`: shows current count, next useful count, full-credit
  count, score, and max points for tiered arrays
- `invalid_grain_shape`: identifies `meta.nova.grain`, `meta.nova.metric.grain`,
  or `meta.nova.metrics[].grain` values that are strings, empty objects, or
  otherwise not canonical grain objects
- `primary_key_integrity_missing_tests`: names the primary key column and the
  missing `unique` or `not_null` dbt manifest test evidence; Nova does not infer
  uniqueness from compiled SQL or warehouse introspection

Project scope responses include `summary` for agent triage:

- `scope`, `entities`, `entities_total`, `truncated`, and `page` so agents know
  whether the summary covers all matching entities or only the returned page
- `score_buckets` and `grade_buckets`
- `worst_entities`
- `category_weak_spots`
- `top_recommendation_fields` with estimated point impact where available
- `drill_down_hints` with exact `get_metadata_score` calls for detailed follow-up

## Scoring Model

Each category produces a **0–100** score, then the overall score is a
weighted sum:

```
overall = documentation * w_doc
        + semantic      * w_sem
        + governance    * w_gov
        + quality       * w_quality
```

### Category Weights (by persona)

| Persona | Documentation | Semantic | Governance | Quality |
|---------|----------------|----------|------------|---------|
| default | 0.30 | 0.25 | 0.25 | 0.20 |
| analyst | 0.20 | **0.45** | 0.15 | 0.20 |
| engineer | 0.20 | 0.15 | 0.15 | **0.50** |
| governance | 0.15 | 0.15 | **0.55** | 0.15 |

These defaults are configured in `src/config/metadata_score.rs` under
`metadata_score.persona_weights` and mirrored in `docs/config_defaults.json`.

## Category Details

### Documentation (0–100)

- Entity description (tiered by length)
- Column descriptions (average tiered quality)
- Doc blocks present (binary)
- Owner defined (binary)

### Semantic (0–100)

Based on `meta.nova` fields:
- `synonyms`, `domains`, `use_cases` (tiered by count)
- `role`, `semantic_type` (binary - checked at the entity level, i.e. `meta.nova.role`)
- `canonical`, `tier`, `grain` (binary)
- `measures` (expression + synonyms)
- `metric` / `metrics` (expression + synonyms)
- Column semantic coverage (% columns with role/semantic_type)

For `resource_type: source`, analytical indicator fields (`meta.nova.measures`
and `meta.nova.metrics`) are not scored and do not produce recommendations.
Sources should remain landing-table metadata surfaces rather than analytical
metric definitions.

Note: `example_values` improves discovery but is **not** scored today.

### Governance (0–100)

- `meta.nova.governance.sensitivity` (binary)
- `meta.nova.governance.pii` (binary)
- `meta.nova.governance.compliance` (tiered by count)
- `owner` (binary)
- `access` (binary)

### Quality (0–100)

- Test coverage (weighted by column role)
  - Critical coverage: identifier, measure, time (higher weight)
  - Dimension coverage: lighter weight for analytic slicing
  - Baseline credit if any tests exist (avoids “all‑or‑nothing”)
- Declared grain present. Full-credit evidence can be column
  `meta.primary_key`, `meta.nova.grain.primary_key`, or aggregate
  `meta.nova.grain.time_field` + `dimensions` with a matching
  `unique`/`unique_combination_of_columns` dbt test over exactly those columns.
- Grain integrity. Identifier PKs need `unique` + `not_null` test evidence;
  aggregate grain receives integrity credit from the matching uniqueness test.
- Constraints (tiered count of not_null / unique / foreign_key)

`get_metadata_score` also surfaces a lightweight quality summary under
`categories.quality.summary.test_coverage` with the coverage percentages and
tested counts for critical and dimension columns.

## Tiered Scoring Rules

### Description length

| Length | Score |
|--------|------|
| 0 | 0% |
| 1–19 | 20% |
| 20–49 | 50% |
| 50–99 | 80% |
| 100+ | 100% |

### Array size (synonyms, domains, compliance, etc.)

| Count | Score |
|-------|------|
| 0 | 0% |
| 1 | 40% |
| 2 | 70% |
| 3+ | 100% |

## Recommendations

If `include_recommendations=true`, each missing or weak signal emits a
recommendation with:

- `category` — documentation / semantic / governance / quality
- `priority` — high / medium / low (based on impact)
- `impact` — max possible points for the missing signal
- `field` — suggested location (e.g., `meta.nova.synonyms`)

## Column vs Entity Scoring

When `scope=column`, each column is scored independently using:
- Column-level description quality
- Column-level nova semantic fields (if present)
- Column-level governance fields (if present)
- Column-level tests, constraints, and data types

The overall column score is still weighted by persona category weights.

## Project Scoring Behavior

`scope=project`:
- sorts selected `resource_types` and entity IDs deterministically
- scores all matching entities for the aggregate project score
- uses `limit` + `offset` only for the returned entity sample
- returns an overall average and paged per‑entity sample results
- returns compact summary buckets and drill-down hints for agent triage
- sets `truncated: true` if `offset + count < total_available`

## Examples

Entity score:
```json
{"name":"get_metadata_score","arguments":{"id_or_name":"model.jaffle_shop.orders","scope":"entity"}}
```

Column score:
```json
{"name":"get_metadata_score","arguments":{"id_or_name":"model.jaffle_shop.orders","scope":"column"}}
```

Project score (models only):
```json
{"name":"get_metadata_score","arguments":{"scope":"project","resource_types":["model"],"limit":500}}
```

## Notes & Limitations

- Non‑column resources (e.g., docs, macros) are **not penalized** for missing
  column metadata.
- If a project does not define tests, quality scores will naturally be lower.
- This tool **does not** write metadata; it only scores based on current
  manifest content.
