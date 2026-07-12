# Tools Reference

This reference lists the canonical 53-tool MCP catalog, grouped by category.
Runtime exposure is profile-filtered by `DBT_NOVA_TOOL_PROFILE`; the default
`agent` profile is lean, while `DBT_NOVA_TOOL_PROFILE=all` exposes the full
catalog for backwards-compatible local/operator use.
All tools return the standard envelope described in [Response Format](response-format.md).
For CLI equivalents and known parity gaps, see
[MCP/CLI Parity](mcp-cli-parity.md).

## Stability and Profiles

The MCP catalog uses two public contract tiers for the v0.0.x line:

- `Stable`: documented fields follow additive-compatibility rules. Removing or
  renaming documented fields requires an explicit breaking-change note.
- `StableGated`: the same field contract, plus explicit opt-in safety controls
  for execution, writes, mutation, or SQL-capable behavior.

Runtime exposure starts from `DBT_NOVA_TOOL_PROFILE` (`agent` by default), then
applies strict allowlist/denylist filters. The `all` profile means the complete
catalog and does not bypass safety gates.

| Tool | Stability | Profiles | Safety gate |
| --- | --- | --- | --- |
| [`search`](#search) | Stable | agent, analyst, engineer, governance, all | - |
| [`search_indicator`](#search_indicator) | Stable | agent, analyst, engineer, governance, all | - |
| [`indicator_inventory`](#indicator_inventory) | Stable | agent, analyst, engineer, governance, all | - |
| [`search_columns`](#search_columns) | Stable | agent, analyst, engineer, governance, all | - |
| [`column_inventory`](#column_inventory) | Stable | agent, analyst, engineer, governance, all | - |
| [`compare_grains`](#compare_grains) | Stable | agent, analyst, engineer, governance, all | - |
| [`find_entity_overlap`](#find_entity_overlap) | Stable | agent, engineer, governance, all | - |
| [`modelling_consistency_report`](#modelling_consistency_report) | Stable | agent, engineer, governance, all | - |
| [`get_entity`](#get_entity) | Stable | agent, analyst, engineer, governance, all | - |
| [`list_entities`](#list_entities) | Stable | agent, analyst, engineer, governance, all | - |
| [`get_lineage`](#get_lineage) | Stable | agent, analyst, engineer, governance, all | - |
| [`get_sql`](#get_sql) | Stable | agent, analyst, engineer, all | - |
| [`get_columns`](#get_columns) | Stable | agent, analyst, engineer, governance, all | - |
| [`diff_entities`](#diff_entities) | Stable | engineer, all | - |
| [`get_impact`](#get_impact) | Stable | agent, analyst, engineer, governance, all | - |
| [`validate_dag`](#validate_dag) | Stable | agent, engineer, governance, all | - |
| [`validate_nova_meta`](#validate_nova_meta) | Stable | engineer, governance, all | - |
| [`validate_eval_suite`](#validate_eval_suite) | Stable | ops, all | - |
| [`get_eval_gate`](#get_eval_gate) | Stable | ops, all | - |
| [`get_eval_history`](#get_eval_history) | Stable | ops, all | - |
| [`compare_eval_runs`](#compare_eval_runs) | Stable | ops, all | - |
| [`run_eval`](#run_eval) | StableGated | ops, all | `DBT_NOVA_MCP_ENABLE_EVAL_RUN` |
| [`init_eval_suite`](#init_eval_suite) | StableGated | ops, all | `DBT_NOVA_MCP_ENABLE_EVAL_WRITES` |
| [`run_agent_eval`](#run_agent_eval) | StableGated | ops, all | `DBT_NOVA_MCP_ENABLE_AGENT_EVAL`; custom commands also require `DBT_NOVA_MCP_ENABLE_CUSTOM_AGENT_PROVIDER` |
| [`inspect_tool_trace`](#inspect_tool_trace) | Stable | ops, all | - |
| [`summarize_tool_trace`](#summarize_tool_trace) | StableGated | ops, all | `DBT_NOVA_MCP_ENABLE_TRACE_WRITES` for Markdown report writes |
| [`redact_tool_trace`](#redact_tool_trace) | StableGated | ops, all | `DBT_NOVA_MCP_ENABLE_TRACE_WRITES` |
| [`replay_tool_trace`](#replay_tool_trace) | Stable | ops, all | - |
| [`show_metadata`](#show_metadata) | Stable | agent, analyst, engineer, governance, ops, all | - |
| [`health`](#health) | Stable | agent, analyst, engineer, governance, ops, all | - |
| [`reload_manifest`](#reload_manifest) | StableGated | ops, all | `DBT_NOVA_MCP_ENABLE_MANIFEST_RELOAD` for source, refresh, or storage changes |
| [`warm_manifest`](#warm_manifest) | StableGated | ops, all | `DBT_NOVA_MCP_ENABLE_MANIFEST_WARM` |
| [`show_config`](#show_config) | Stable | ops, all | - |
| [`validate_config`](#validate_config) | Stable | ops, all | - |
| [`inspect_storage`](#inspect_storage) | Stable | ops, all | - |
| [`prune_storage`](#prune_storage) | StableGated | ops, all | `DBT_NOVA_MCP_ENABLE_STORAGE_ADMIN` |
| [`cleanup_storage`](#cleanup_storage) | StableGated | ops, all | `DBT_NOVA_MCP_ENABLE_STORAGE_ADMIN` |
| [`list_tags`](#list_tags) | Stable | agent, analyst, engineer, governance, all | - |
| [`list_packages`](#list_packages) | Stable | agent, analyst, engineer, governance, all | - |
| [`list_databases`](#list_databases) | Stable | agent, analyst, engineer, governance, all | - |
| [`get_column_lineage`](#get_column_lineage) | Stable | agent, analyst, engineer, governance, all | - |
| [`get_test_coverage`](#get_test_coverage) | Stable | agent, analyst, engineer, governance, all | - |
| [`get_metadata_score`](#get_metadata_score) | Stable | agent, analyst, engineer, governance, all | - |
| [`get_metadata_audit`](#get_metadata_audit) | Stable | agent, engineer, governance, all | - |
| [`get_agent_readiness`](#get_agent_readiness) | Stable | agent, engineer, governance, all | - |
| [`batch_get_entities`](#batch_get_entities) | Stable | agent, analyst, engineer, governance, all | - |
| [`find_by_path`](#find_by_path) | Stable | agent, analyst, engineer, governance, all | - |
| [`search_recipes`](#search_recipes) | Stable | agent, analyst, engineer, all | - |
| [`get_recipe`](#get_recipe) | Stable | agent, analyst, engineer, all | - |
| [`run_recipe`](#run_recipe) | StableGated | engineer, all | tool profile/denylist plus SQL provider controls |
| [`get_undocumented`](#get_undocumented) | Stable | agent, engineer, governance, all | - |
| [`get_context`](#get_context) | Stable | agent, analyst, engineer, governance, all | - |
| [`execute_sql`](#execute_sql) | StableGated | analyst, engineer, all | tool profile/denylist plus SQL provider controls |

> Tip: use `persona` in `search` to tune ranking (`analyst`, `engineer`, `governance`).

## Discovery

### `search`
Full‑text and hybrid search across models, sources, tests, macros, etc.

Required:
- `query`

Common:
- `resource_types` (array)
- `persona` (string)
- `detail` (`compact` | `standard` | `full`)
- `include_highlights` (bool)
- `include_sql` (bool, only for `detail=full`)
- `explain` (bool, includes deterministic ranking/debug breakdowns)
- `limit`, `offset`, `min_score`, `fuzzy`

```json
{"name":"search","arguments":{"query":"customer lifetime value","persona":"analyst","resource_types":["model"],"detail":"standard","include_highlights":true,"limit":10,"offset":0,"fuzzy":false}}
```

**Example Response:**
```json
{
  "success": true,
  "count": 3,
  "persona": "analyst",
  "suggestions": ["customer_lifetime_value", "customer_metrics"],
  "data": [
    {
      "unique_id": "model.analytics.customers",
      "name": "customers",
      "resource_type": "model",
      "relation_name": "analytics.public.customers",
      "description": "Customer dimension with lifecycle attributes...",
      "columns_total": 18,
      "primary_key_columns": ["customer_id"],
      "extended_meta_summary": {
        "fields": [
          {
            "alias": "owner",
            "path": "meta.owner",
            "search_field": "meta.owner",
            "mode": "keyword",
            "values": ["analytics_reporting"]
          }
        ]
      },
      "persona_payload": {
        "focus": "business_discovery",
        "business_definition": "Customer dimension with lifecycle attributes...",
        "key_dimensions": ["customer_id", "country", "segment"],
        "time_field": "created_at",
        "candidate_metrics": ["customer_lifetime_value"],
        "selection_signals": {
          "has_metric_definition": true,
          "has_measure_definition": false,
          "has_grain": true,
          "has_time_field": true,
          "dimension_overlap": 1,
          "confidence_band": "high"
        },
        "selection_rationale": "Selection signals: includes metric definitions, declares semantic grain, has an explicit time field, 1 query-aligned dimension(s)."
      },
      "provenance": {
        "tier": "semantic_layer",
        "owner": "analytics",
        "readiness": {
          "metadata_score": 91,
          "metadata_grade": "A",
          "doc_coverage_pct": 94.44,
          "has_owner": true,
          "has_nova_meta": true,
          "tests_total": 6
        },
        "freshness": {
          "status": "fresh",
          "source": "manifest_generated_at",
          "timestamp": "2026-06-16T08:00:00Z",
          "age_days": 0,
          "stale_after_days": 30
        }
      },
      "score": 12.5,
      "highlights": {
        "description": ["<em>customer</em> lifetime value calculation"]
      }
    }
  ],
  "analysis_hints": [
    "Top candidates `customers` and `customer_metrics` are close (6.5% score gap). Use `get_entity` or `get_context` to verify metric definition, grain, and date/country dimensions before final SQL."
  ]
}
```

**detail** controls result payload:

- `compact`: identity, relation, grain, primary-key, and compact Nova indicator names
- `standard`: persona‑optimized summary
- `full`: complete entity payload (same as `get_entity` with `detail: "full"`)

When omitted, `detail` uses the active result profile: `standard` for CLI/tool
calls by default and `compact` for MCP by default. Explicit `detail` values
override the profile.

When `search.extended_meta.fields` contains fields with `summary: true`,
`standard` and `full` search rows include `extended_meta_summary`. The summary is
bounded by `extended_meta.max_values_per_field` and
`extended_meta.max_bytes_per_value`, contains only configured summary fields,
and uses aliases such as `meta.owner` for fielded search. A field sets
`truncated: true` when summary caps drop values or trim bytes, with
`dropped_values` and `byte_truncated_values` counts when applicable. Full rows
still include the complete raw dbt `meta` and `columns.*.meta` payload for
inspection.

Search result rows include an additive `provenance` object for `compact`,
`standard`, and `full` detail. `provenance.tier` is `semantic_layer` when
canonical Nova measures or metrics are present, `curated` when owner/docs/tests
or other meaningful metadata are present, and `raw` when metadata is sparse.
`owner` is extracted from dbt metadata using legacy `meta` first and dbt 1.11+
`config.meta` as fallback. `readiness` contains compact metadata score,
documentation, owner, Nova metadata, and test-count signals. `freshness.status`
is `fresh`, `stale`, or `unknown`; Nova uses source freshness timestamps when
available, then manifest `metadata.generated_at`, and otherwise returns
`unknown`.

When `detail: "standard"` and the query matches Nova measures or metrics, search
results include a compact `semantic_preview` with the matched measure/metric
name, description, expression, canonical flag, and match type. For analyst KPI
resolution, prefer `search_indicator` before broad `search`.

When `explain: true`, each result row includes an `explain` block with retrieval
and scoring factors, and the top-level response includes an `explain` payload
with query tokens, retrievers used, and the active ranking config snapshot.

### `search_indicator`
Search Nova measures and metrics directly, then return the parent execution
entity and grain context. Native dbt Semantic Layer / MetricFlow `metrics` and
`semantic_models` are bridged into this indicator surface even when no
hand-authored `meta.nova` block exists.

Each indicator row includes response-only execution metadata:
`indicator_source` (`nova_meta`, `dbt_metric`, or `dbt_semantic_model`),
`execution_surface` (`relation`, `semantic_layer`, or `metadata_only`),
`queryable`, `direct_sql_queryable`, `queryable_via` (`relation_name`,
`metricflow`, or `none`), and an optional `execution_note`.

Use those fields as the execution gate. Relation-backed indicators can be
queried through the returned `relation_name` after the grain and fields fit the
question. Semantic Layer-backed indicators require the configured dbt Semantic
Layer / MetricFlow execution path and are not directly queryable through Nova
SQL. Metadata-only indicators are context only; they are not safe SQL surfaces
and should not trigger inferred joins.

Required:
- `query`

Common:
- `indicator_types` (`["metric"]`, `["measure"]`, or omitted for both)
- `resource_types` (filters parent entity types)
- `persona` (string, defaults to `analyst`)
- `detail` (`compact` | `standard` | `full`; defaults from the active result profile)
- `group_mode` (`none` | `top` | `all`; defaults to `top` for omitted MCP compact-profile calls, otherwise `all`)
- `max_parent_groups` (integer cap for `parent_groups`)
- `include_support_signals` (bool, default: `true`)
- `explain` (bool, includes deterministic ranking/debug breakdowns)
- `limit`, `offset`, `min_score`

```json
{"name":"search_indicator","arguments":{"query":"average order value","indicator_types":["metric"],"resource_types":["model"],"persona":"analyst","detail":"compact","group_mode":"top","limit":5,"offset":0}}
```

Example response shape:

```json
{
  "success": true,
  "count": 1,
  "persona": "analyst",
  "suggestions": [],
  "data": [
    {
      "indicator_name": "average_order_value",
      "indicator_type": "metric",
      "canonical": true,
      "match_type": "name",
      "score": 10.5,
      "expression": "sum(gmv_amount) / nullif(count(distinct order_id), 0)",
      "parent_unique_id": "model.package.orders_semantic_templates",
      "parent_name": "orders_semantic_templates",
      "parent_resource_type": "model",
      "relation_name": "analytics.dbt_test.orders_semantic_templates",
      "indicator_source": "nova_meta",
      "execution_surface": "relation",
      "queryable": true,
      "direct_sql_queryable": true,
      "queryable_via": "relation_name",
      "domains": ["commerce"],
      "grain": {
        "time_field": "order_date",
        "dimensions": ["country_code", "sales_channel"]
      }
    }
  ]
}
```

When `explain: true`, indicator rows include a per-row `explain` block with the
base semantic match score, parent coherence, RRF contribution, reranker bonus,
and final score. The top-level response also includes an `explain` payload with
query tokens, retrievers used, and the active indicator-ranking config snapshot.

For agent workflows, prefer `detail: "compact"`, `group_mode: "top"`, and a
small `limit` such as `3`. `group_mode: "all"` preserves richer parent-group
diagnostics for debugging, while `group_mode: "none"` removes `parent_groups`
entirely.

### `indicator_inventory`
List Nova measures and metrics deterministically, with parent execution context.
Use this when you need a flat semantic catalog instead of ranked search results.
MetricFlow metrics and semantic-model measures are included as derived Nova
indicators unless explicit `meta.nova` metadata overrides or extends them.
Execution metadata uses the same response-only fields and execution-surface gate
as `search_indicator`.

Common:
- `indicator_types` (`["metric"]`, `["measure"]`, or omitted for both)
- `resource_types` (filters parent entity types)
- `canonical_only` (boolean, defaults to `false`)
- `limit`, `offset`

```json
{"name":"indicator_inventory","arguments":{"indicator_types":["measure"],"resource_types":["model"],"canonical_only":true,"limit":100,"offset":0}}
```

Example response shape:

```json
{
  "success": true,
  "count": 1,
  "data": [
    {
      "indicator_name": "gmv",
      "indicator_type": "measure",
      "canonical": true,
      "synonyms": ["gross merchandise value"],
      "expression": "sum(gmv_amount)",
      "field": "gmv_amount",
      "measure_type": "sum",
      "parent_unique_id": "model.package.fact_orders_canonical",
      "parent_name": "fact_orders_canonical",
      "parent_resource_type": "model",
      "relation_name": "analytics.dbt_test.fact_orders_canonical",
      "indicator_source": "nova_meta",
      "execution_surface": "relation",
      "queryable": true,
      "direct_sql_queryable": true,
      "queryable_via": "relation_name",
      "domains": ["commerce"],
      "grain": {
        "time_field": "order_date",
        "dimensions": ["country_code", "sales_channel"]
      }
    }
  ]
}
```

### `search_columns`
Search columns directly by name, synonym, description, role, semantic type, or example values.

Required:
- `query`

Common:
- `resource_types` (filters parent entity types)
- `roles`
- `semantic_types`
- `limit`, `offset`, `min_score`

```json
{"name":"search_columns","arguments":{"query":"alpha","resource_types":["model"],"limit":10,"offset":0}}
```

Example response shape:

```json
{
  "success": true,
  "count": 1,
  "data": [
    {
      "column_name": "country_code",
      "match_type": "example_value",
      "score": 5.25,
      "matched_value": "alpha",
      "annotated": true,
      "role": "dimension",
      "semantic_type": "country_code",
      "synonyms": ["market"],
      "example_values": ["alpha", "beta"],
      "parent_unique_id": "model.package.fact_orders_canonical",
      "parent_name": "fact_orders_canonical",
      "parent_resource_type": "model",
      "domains": ["commerce"]
    }
  ]
}
```

### `column_inventory`
List columns deterministically across models or sources, with parent context and semantic hints.

Common:
- `resource_types` (filters parent entity types)
- `roles`
- `semantic_types`
- `annotated_only` (boolean, defaults to `false`)
- `limit`, `offset`

```json
{"name":"column_inventory","arguments":{"resource_types":["model"],"roles":["dimension"],"annotated_only":true,"limit":100,"offset":0}}
```

Example response shape:

```json
{
  "success": true,
  "count": 1,
  "data": [
    {
      "column_name": "country_code",
      "annotated": true,
      "role": "dimension",
      "semantic_type": "country_code",
      "synonyms": ["market"],
      "example_values": ["alpha", "beta"],
      "parent_unique_id": "model.package.fact_orders_canonical",
      "parent_name": "fact_orders_canonical",
      "parent_resource_type": "model",
      "domains": ["commerce"]
    }
  ]
}
```

### `compare_grains`
Compare effective grain between two entities, including entity-level and metric-level grain variants.

Required:
- `entity1`
- `entity2`

Optional:
- `entity1_resource_type`
- `entity2_resource_type`

```json
{"name":"compare_grains","arguments":{"entity1":"model.package.fact_orders_canonical","entity2":"model.package.orders_semantic_templates"}}
```

### `find_entity_overlap`
Detect overlapping entities using shared semantic evidence such as domains, synonyms, indicators, semantic types, and grain hints.

Common:
- `id_or_name` and optional `resource_type` to focus overlap on one entity
- `resource_types`
- `limit`, `offset`, `min_score`

```json
{"name":"find_entity_overlap","arguments":{"resource_types":["model"],"limit":25,"offset":0}}
```

### `modelling_consistency_report`
Audit project-level overlap, duplicate indicators, canonical conflicts, grain
drift, and deterministic agent-modelling risks.

Common:
- `resource_types`
- `limit` (max rows per report section)
- `min_score` (applies to overlap section)

```json
{"name":"modelling_consistency_report","arguments":{"resource_types":["model"],"limit":20}}
```

Responses include a compact `summary` with section counts, top duplicate
indicator groups, top canonical conflicts, overlap evidence category counts,
bounded overlap examples, multi-grain entity highlights, `agent_modelling`
finding counts, and drill-down hints. The existing detail arrays remain paged
by `limit` and `offset`.

The report includes `agent_modelling_schema_version` set to
`"agent_modelling.v1"`, `agent_modelling_finding_count`, and
`agent_modelling_findings`. Findings currently cover duplicate/canonical
indicator ambiguity, non-queryable indicator parents, metric output/grain field
issues, missing metric time fields, multi-grain entities, and semantic-model
primary/time grain gaps, catalog drift on indicator fields, catalog-only
measure-like columns, unresolved MetricFlow measure references,
cross-grain/multi-fact risks, missing canonical primary keys, and
analyst-surface layering, semantic-label, column-semantic, and governance risks.
Deterministic manifest, metadata, and catalog checks are enabled by default;
use `DBT_NOVA_AGENT_MODELLING_AUDIT_ENABLED=false` to suppress
`agent_modelling_findings`, `DBT_NOVA_AGENT_MODELLING_MAX_FINDINGS` to bound
retained findings, and keep SQL-shape checks opt-in with
`DBT_NOVA_AGENT_MODELLING_ENABLE_SQL_SHAPE_CHECKS=true`.

See [Agent Modelling Audits](../features/agent-modelling-audits.md) for the
finding contract, execution-surface policy, readiness integration, and CI
examples.

### `get_entity`
Fetch a single entity by `unique_id` or name.

Required:
- `id_or_name`

Optional:
- `resource_type`
- `detail` (`compact` | `standard` | `full`; defaults from the active result profile)

Notes:
- `unique_id` is accepted as a compatibility alias for `id_or_name`.
- `detail: "compact"` includes identity, relation, grain, primary-key columns,
  domains, canonical status, and capped metric/measure names.
- `detail: "standard"` includes `nova_summary` (metrics/measures/grain/domains) and `primary_key_columns` when available.

```json
{"name":"get_entity","arguments":{"id_or_name":"model.jaffle_shop.orders","detail":"standard"}}
```

### `list_entities`
List entities by type with filters.

Required:
- `resource_type`

Optional:
- `package`, `tags`, `database_schema`
- `detail` (`compact` | `standard` | `full`), `limit`, `offset`

```json
{"name":"list_entities","arguments":{"resource_type":"model","tags":["pii"],"detail":"standard","limit":100}}
```

### `batch_get_entities`
Retrieve multiple entities at once.

Required:
- `unique_ids`

Optional:
- `detail` (`compact` | `standard` | `full`)

```json
{"name":"batch_get_entities","arguments":{"unique_ids":["model.a","model.b"],"detail":"full"}}
```

### `find_by_path`
Find entities matching a file path glob.

Required:
- `path_pattern`

Optional:
- `resource_types`, `detail` (`compact` | `standard` | `full`), `limit`, `offset`

Broad path globs are bounded by pagination. When `truncated=true`,
`total_available` is the number of matches observed before the bounded scan
stopped, not an exhaustive manifest-wide count.

```json
{"name":"find_by_path","arguments":{"path_pattern":"models/staging/**","resource_types":["model"],"detail":"standard"}}
```

## Context

### `get_context`
One-shot context bundle. Returns lineage, columns, tests, docs, and summary stats for an entity.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `id_or_name` | string | Yes | Entity identifier (unique_id or name) |
| `resource_type` | string | No | Filter by type when using name |
| `include_columns` | bool | No | Include column details (default: true) |
| `include_upstream` | bool | No | Include upstream lineage (default: true) |
| `include_downstream` | bool | No | Include downstream lineage (default: true) |
| `include_tests` | bool | No | Include test coverage (default: true) |
| `include_docs` | bool | No | Include documentation (default: true) |
| `include_sql` | bool | No | Include raw/compiled SQL in entity context (default: false) |
| `context_mode` | string | No | Output shaping (`standard` \| `engineer`, default: `standard`) |
| `lineage_depth` | int | No | Depth for lineage traversal (default: 1) |

Notes:
- The `entity` object includes `nova_summary` and `grain_summary` when available.
- The `entity` object and upstream/downstream lineage entities include the same
  `provenance` object used by search results.
- The `tests` object includes `columns_tested`, `columns_total`, and a limited `columns_without_tests` list.
- The `tests.gaps` array includes `missing_pk_test` and `untested_column` entries (limited).
- `analysis_hints` explains empty lineage or missing dependency metadata when detected.

```json
{"name":"get_context","arguments":{"id_or_name":"model.jaffle_shop.orders","include_columns":true,"include_upstream":true,"include_downstream":true,"include_tests":true,"include_docs":false,"include_sql":false,"lineage_depth":2,"context_mode":"engineer"}}
```

## Lineage

### `get_lineage`
Traverse entity lineage.

Required:
- `id_or_name`
- `direction` (`upstream` | `downstream`)

Optional:
- `depth`, `resource_types`, `detail`

Notes:
- Response includes `lineage_status` and `lineage_hints` to explain empty lineage results.
- Entity rows include the same additive `provenance` object used by search
  results for `compact`, `standard`, and `full` detail.
- Requested `depth` is capped by `DBT_NOVA_MAX_LINEAGE_DEPTH` (config `lineage_max_depth`).

```json
{"name":"get_lineage","arguments":{"id_or_name":"model.jaffle_shop.orders","direction":"upstream","depth":2,"resource_types":["model","source"]}}
```

### `get_impact`
Blast‑radius estimate for an entity.

Required:
- `id_or_name`

```json
{"name":"get_impact","arguments":{"id_or_name":"model.jaffle_shop.orders"}}
```

### `get_column_lineage`
Trace a column upstream or downstream.

Required:
- `id_or_name`
- `column_name`
- `direction` (`upstream` | `downstream`)

Optional:
- `resource_type` (disambiguates `id_or_name` when name matches multiple entities)
- `depth`, `confidence` (`high`|`medium`|`low`)

Notes:
- Response includes `lineage_status` and `lineage_hints` to explain empty lineage results.
- Requested `depth` is capped by `DBT_NOVA_COLUMN_LINEAGE_MAX_DEPTH` (config `column_lineage.max_depth`).

```json
{"name":"get_column_lineage","arguments":{"id_or_name":"model.jaffle_shop.orders","column_name":"customer_id","direction":"upstream","confidence":"high"}}
```

## Code & Schema

### `get_columns`
Inspect column names, types, and metadata.

Required:
- `id_or_name`

```json
{"name":"get_columns","arguments":{"id_or_name":"model.jaffle_shop.orders"}}
```

Notes:
- Response includes `primary_key_columns` when columns are marked with `meta.primary_key: true`.
- When `catalog.json` is configured or auto-discovered, column rows include
  warehouse `data_type`, `catalog_data_type`, optional `catalog_stats`, and
  `catalog_drift` fields for type mismatches, catalog-only columns, or declared
  columns missing from the catalog.

### `get_sql`
Return raw or compiled SQL.

Required:
- `id_or_name`

Optional:
- `compiled` (bool)

```json
{"name":"get_sql","arguments":{"id_or_name":"model.jaffle_shop.orders","compiled":true}}
```

Response fields:
- `contains_templating` (bool): true if the SQL still contains Jinja/refs.
- `compiled_available` (bool): only present when `compiled=true`; false means compiled SQL is not fully rendered.

### `diff_entities`
Compare two entities side‑by‑side.

Required:
- `entity1`
- `entity2`

Optional:
- `compare_fields`
- `entity1_resource_type` (disambiguates `entity1` when passed as a name)
- `entity2_resource_type` (disambiguates `entity2` when passed as a name)

```json
{"name":"diff_entities","arguments":{"entity1":"model.pkg.orders_v1","entity2":"model.pkg.orders_v2","compare_fields":["columns","sql","config"]}}
```

## Analysis

### `get_test_coverage`
Schema/data test coverage for a model/source.

Required:
- `id_or_name`

Optional:
- `resource_type` (disambiguates `id_or_name` when name matches multiple entities)
- `include_full`
- `columns_limit` (max entries in `columns_without_tests`)

```json
{"name":"get_test_coverage","arguments":{"id_or_name":"model.jaffle_shop.orders","include_full":true,"columns_limit":50}}
```

Response fields:
- `columns_without_tests` (possibly truncated)
- `columns_without_tests_truncated` (bool)
- `columns_without_tests_total` (number)

### `search_recipes`
Discover reusable analysis recipes from manifest `analysis` nodes under the
recipe prefix.

Required:
- none

Optional:
- `query` (text)
- `topic` (directory prefix matcher)
- `include_queries` (bool)
- `limit`, `offset`

`query` matches recipe IDs (path) and SQL query names.

```json
{"name":"search_recipes","arguments":{"topic":"marketing","include_queries":true,"limit":10,"offset":0}}
```

Recipe search results include:
- `required_parameters`
- `optional_parameters`
- `parameter_defaults`
- `query_parameters` (per-query contract map)

For workflow design guidance, see [Analysis Recipes](../features/recipes.md).

### `get_recipe`
Load a specific recipe and optional SQL payload.

Required:
- `recipe_id` (recipe ID from manifest analysis path)

Optional:
- `include_sql` (bool)
- `include_queries` (bool)
- `parameters` (optional placeholder values for SQL rendering and missing checks)
- `placeholder_types` (optional placeholder coercion hints)
- `parameter_types` (legacy compatibility alias; prefer `placeholder_types`)

```json
{"name":"get_recipe","arguments":{"recipe_id":"marketing/weekly_country_kpi_report","include_queries":true,"include_sql":true}}
```

`queries` entries include:
- `source` (`manifest_analysis`)
- `analysis_id`
- `parameters` (per-query parameter specs)

Recipe payload includes:
- `required_parameters`
- `optional_parameters`
- `parameter_defaults`
- `query_parameters`
- `missing_parameters`
- `unused_parameters`
- `type_mismatches`

### `run_recipe`
Execute a recipe's SQL queries in deterministic order.

Required:
- `recipe_id` (recipe ID from manifest analysis path)

Optional:
- `query_names` (list of SQL file names)
- `query_indexes` (1-based indexes by execution order)
- `stop_on_failure` (bool, default: `true`)
- `include_sql` (bool)
- SQL execution controls: `row_limit`, `byte_limit`, `wait_timeout_s`, `poll_interval_ms`, `max_poll_seconds`, `parameters`, `placeholder_types`, `sql_parameter_types`, `fetch_all_chunks`, `max_chunks`
- `parameter_types` (legacy compatibility fallback alias; prefer `placeholder_types` and `sql_parameter_types`)

```json
{"name":"run_recipe","arguments":{"recipe_id":"marketing/weekly_country_kpi_report","query_indexes":[1,2],"stop_on_failure":true}}
```

Queries are resolved from manifest analysis SQL (`compiled_code` preferred,
`raw_code` fallback).

Run result includes per-query status and optional SQL text.

Run preflight validates selected queries before execution and returns structured
validation details on failure:
- `missing_parameters`
- `unused_parameters`
- `type_mismatches`
- `by_query`

For workflow design guidance, see [Analysis Recipes](../features/recipes.md).

### `get_metadata_score`
Metadata quality score for entities, columns, or project scope.

Optional:
- `id_or_name`
- `resource_type`
- `persona` (`analyst`, `engineer`, `governance`)
- `scope` (`entity`, `column`, `project`)
- `include_breakdown`
- `include_recommendations`
- `resource_types` (project scope)
- `limit` (project scope)
- `offset` (project scope pagination)

Responses include `scoring_contract` metadata describing grade bands,
description tiers, array-count tiers, canonical grain shape, and primary-key
integrity evidence rules. Entity and column scope responses include
`diagnostics` rows for partial or missing credit. Project scope responses include
`quality_summary.test_coverage`, aggregated across the returned entities, plus
`summary` scope/paging metadata, buckets, weak spots, repeated recommendation
fields, and drill-down hints. Project rows use deterministic ordering for
pagination.

```json
{"name":"get_metadata_score","arguments":{"id_or_name":"model.jaffle_shop.orders","persona":"analyst","scope":"entity"}}
```

See `docs/features/metadata-scoring.md` for scoring rules, personas, and examples.

### `get_metadata_audit`
Higher-level metadata audit report and gate status for project, changed-file, or
explicit-entity selections.

Use `get_metadata_score` for a single score lookup or raw project scoring data.
Use `get_metadata_audit` when you need the CLI audit workflow: selection modes,
thresholds, required/advisory gate status, project summary, entity rows, and
report-ready JSON.

Optional:
- `selection_mode` (`project`, `changed`, `entities`; default `project`)
- `changed_files_json`: JSON array of changed file paths for `changed`
- `entity_ids_json`: JSON array of ids or names for `entities`
- `resource_types_json`: JSON array, defaulting to `["model"]`
- `personas_json`: JSON array, defaulting to `["engineer"]`
- `thresholds_json`: JSON required/advisory threshold configuration
- `include_breakdown`, `include_recommendations`
- `fail_on_no_targets`

Required threshold failures are returned in `data.gate_status` and
`data.summary`; they do not become MCP transport errors.
`data.summary` also includes score/grade buckets, worst entities by persona,
category weak spots, repeated recommendation fields with estimated impact, and
drill-down hints.

```json
{"name":"get_metadata_audit","arguments":{"selection_mode":"changed","changed_files_json":"[\"models/marts/orders.sql\"]"}}
```

See `docs/features/metadata-audit.md` for report fields and threshold examples.

### `get_agent_readiness`
Manifest-level readiness report for agent workflows.

Optional:
- `personas_json`: JSON array of personas, defaulting to
  `["engineer","analyst","governance"]`
- `thresholds_json`: JSON readiness threshold configuration
- `eval_gate_json`: raw `eval gate` report JSON or the full CLI JSON envelope

The tool returns the same `agent_readiness.v1` report contract as
`dbt-nova audit agent-readiness --json`, without writing report files or applying
CLI exit semantics. Reports include advisory `suggested_meta_patches` for
reviewable dbt metadata remediation and draft `golden_question_seeds` for eval
authoring. Suggested patches never edit files, and generated seeds should be
reviewed before becoming CI gates. Reports also include the shared metadata
`scoring_contract` and compact `summary` triage fields for score buckets, weak
spots, repeated fields, agent-modelling counts/top codes, and drill-down hints.
Deterministic modelling blockers are returned as readiness blockers; high and
medium modelling findings and advisory count threshold misses are returned as
improvements, while required count threshold misses are blockers.

See [Agent Modelling Audits](../features/agent-modelling-audits.md) for the
modelling finding contract and execution-surface rules.

Large reports use the standard MCP response-budget behavior; check
`_nova_result_meta.truncated` when response budgeting is enabled.

```json
{"name":"get_agent_readiness","arguments":{"personas_json":"[\"engineer\",\"analyst\"]"}}
```

See `docs/features/agent-readiness.md` for report fields and threshold examples.

### `validate_nova_meta`
Validate project YAML `meta.nova` blocks against the public Nova schema and
local semantic rules.

Use `validate_nova_meta` when an MCP-connected agent is authoring or reviewing
dbt YAML and needs the same data returned by `dbt-nova audit nova-meta --json`.
Validation findings are returned in `data.findings`; validation errors do not
become MCP transport errors.

Optional:
- `project_dir`: dbt project directory, defaulting to the MCP server working
  directory. Relative values are resolved under the server working directory.
- `paths`: relative YAML file or directory paths under `project_dir`. Omit for a
  project-wide scan. The singular alias `path` is also accepted.
- `resource_kind` (`model`, `source`, `table`, `metric`)
- `resource_name`
- `column` (requires `resource_name`)

For path safety, `project_dir` must resolve under the server working directory,
and each supplied path must be relative and remain under `project_dir` after
symlink resolution. Project-wide scans skip symlinked files and directories.

```json
{"name":"validate_nova_meta","arguments":{"project_dir":".","paths":["models/marts/orders.yml"],"resource_kind":"model","resource_name":"fct_orders"}}
```

The response `data` object includes `schema_version`, `project_dir`,
`scanned_files`, `target_count`, `error_count`, `warning_count`, `findings`,
`selector`, and `path_policy`.

See `docs/features/nova-meta-overview.md` for schema and semantic validation
rules.

### `validate_eval_suite`
Validate a local YAML or JSON eval suite without loading a manifest or running a
provider.

Required:
- `suite`: suite path under the MCP server working directory

The response `data` includes `valid`, `path`, `suite_name`, `version`,
`bridge_case_count`, `agent_case_count`, and `safety_policy`.

```json
{"name":"validate_eval_suite","arguments":{"suite":"evals/analyst-smoke.yml"}}
```

### `get_eval_gate`
Read eval telemetry and return the same gate report data as
`dbt-nova eval gate <SUITE> --json`.

Required:
- `suite`: suite name used in telemetry

```json
{"name":"get_eval_gate","arguments":{"suite":"analyst-smoke"}}
```

### `get_eval_history`
Read filtered eval telemetry rows for a suite.

Required:
- `suite`: suite name used in telemetry
- `since`: UTC lower bound in `YYYY-MM-DD` format

The response `data` includes `suite_name`, the normalized `since` boundary,
`row_count`, `rows`, and `safety_policy`.

Eval agent provider logs are redacted by default when written as artifacts or
returned as assertion evidence. The `safety_policy.raw_provider_logs_enabled_env`
field names the explicit unsafe local opt-in for writing raw provider logs.

```json
{"name":"get_eval_history","arguments":{"suite":"analyst-smoke","since":"2026-06-01"}}
```

### `compare_eval_runs`
Compare two local eval result directories or `results.json` files and return
the same before/after data used by `dbt-nova eval compare`.

Required:
- `before`: baseline result directory or `results.json` path under the MCP
  server working directory
- `after`: candidate result directory or `results.json` path under the MCP
  server working directory

The response `data` uses `eval_comparison.v1` and includes `before`, `after`,
`delta`, and `markdown`. The Markdown is suitable for PR descriptions. The delta
includes pass-rate movement, assertion count changes, newly passing/failing
cases, still-failing cases, added/removed cases, and status changes. For agent
evals, Nova reads trace artifact paths from each `results.json` and includes
tool-call counts, duration, response bytes, and token counters when the traces
exist and contain those fields. Missing trace artifacts are returned as warnings
instead of tool errors.

```json
{"name":"compare_eval_runs","arguments":{"before":"out/evals/before","after":"out/evals/after/results.json"}}
```

### `run_eval`
Run deterministic bridge eval assertions against the currently loaded MCP
manifest.

Required:
- `suite`: suite path under the MCP server working directory

Optional:
- `output_dir`: artifact directory under the server working directory
- `telemetry`, `telemetry_retention`
- `case_ids`
- `fail_under`

This local execution capability is disabled unless
`DBT_NOVA_MCP_ENABLE_EVAL_RUN=1` is set. Unlike the CLI, MCP `run_eval` uses the
manifest already loaded by the server.

The response `data` is the same eval report contract written to CLI
`results.json` and includes `eval_card` (`eval_card.v1`) with suite purpose,
scope, case counts, pass rate, gate evidence, telemetry status, and known gaps.
CLI runs also write `card.md` and prepend the same card to `report.md`.

```json
{"name":"run_eval","arguments":{"suite":"evals/analyst-smoke.yml","telemetry":true,"fail_under":1.0}}
```

### `init_eval_suite`
Write a starter eval suite file under the server working directory.

Required:
- `out`: output path under the MCP server working directory

Optional:
- `persona`
- `force`

This file-write capability is disabled unless
`DBT_NOVA_MCP_ENABLE_EVAL_WRITES=1` is set.

```json
{"name":"init_eval_suite","arguments":{"persona":"analyst","out":"evals/analyst-smoke.yml"}}
```

### `run_agent_eval`
Run provider-backed agent evals and score observed Nova tool-use traces.

Required:
- `suite`: suite path under the MCP server working directory

Optional:
- `provider`, `provider_model`
- `provider_command`, `provider_args_json`
- `manifest_path`, `manifest_uri`, `storage_instance_id`
- `output_dir`, `telemetry`, `telemetry_retention`
- `case_ids`, `timeout_secs`, `fail_under`
- `cleanup_storage_on_start`, `read_only`

This provider execution capability is disabled unless
`DBT_NOVA_MCP_ENABLE_AGENT_EVAL=1` is set. Custom provider commands and
arguments also require `DBT_NOVA_MCP_ENABLE_CUSTOM_AGENT_PROVIDER=1`.

The response `data.eval_card` includes provider metadata when available,
including provider preset, command preset, and provider model.

```json
{"name":"run_agent_eval","arguments":{"suite":"evals/analyst-smoke.yml","provider":"opencode","case_ids":["metric_lookup_flow"]}}
```

## Trace Review

### `inspect_tool_trace`
Inspect a local Nova tool-call trace JSONL file and return valid rows, malformed
row warnings, tool order, counts, response byte budgets, truncation, errors, and
semantic-first signals.

Required:
- `path`: trace JSONL path under the MCP server working directory

```json
{"name":"inspect_tool_trace","arguments":{"path":".nova/eval-runs/agent/tool-calls/metric_lookup_flow.jsonl"}}
```

### `summarize_tool_trace`
Summarize a local trace JSONL file for PRs, eval artifacts, and review handoffs.
When `report_md_path` is provided, Nova writes the Markdown summary under the
server working directory.

Required:
- `path`: trace JSONL path under the MCP server working directory

Optional:
- `report_md_path`

Markdown report writes are disabled unless
`DBT_NOVA_MCP_ENABLE_TRACE_WRITES=1` is set.

```json
{"name":"summarize_tool_trace","arguments":{"path":".nova/eval-runs/agent/tool-calls/metric_lookup_flow.redacted.jsonl","report_md_path":".nova/eval-runs/agent/tool-calls/metric_lookup_flow.redacted.md"}}
```

### `redact_tool_trace`
Redact a local trace JSONL file for safe sharing. The output preserves tool name,
call index, status, response bytes, truncation flags, selected IDs, top IDs, and
sanitized scalar parameter summaries.

Required:
- `path`: input trace JSONL path under the MCP server working directory
- `out`: redacted JSONL output path under the MCP server working directory

Redaction writes are disabled unless `DBT_NOVA_MCP_ENABLE_TRACE_WRITES=1` is set.

```json
{"name":"redact_tool_trace","arguments":{"path":".nova/eval-runs/agent/tool-calls/metric_lookup_flow.jsonl","out":".nova/eval-runs/agent/tool-calls/metric_lookup_flow.redacted.jsonl"}}
```

### `replay_tool_trace`
Replay supported deterministic Nova tool calls from a local trace JSONL file
against the currently loaded MCP manifest. Unsupported, unsafe,
under-specified, and `execute_sql` rows are skipped with explicit reasons.
Replay compares response-shape evidence such as success, result count,
truncation, selected IDs, and top IDs; it does not return full response diffs.

Required:
- `path`: trace JSONL path under the MCP server working directory

Supported replay tools are `search`, `search_indicator`, `search_columns`,
`get_entity`, `get_context`, and `get_lineage`.

```json
{"name":"replay_tool_trace","arguments":{"path":".nova/eval-runs/agent/tool-calls/metric_lookup_flow.redacted.jsonl"}}
```

### `get_undocumented`
Find entities missing descriptions (optionally columns).

Required:
- `resource_type`

Optional:
- `id_or_name`, `include_columns`, `include_full`, `limit`, `offset`
- `package`, `path_prefix`

```json
{"name":"get_undocumented","arguments":{"resource_type":"model","include_columns":true,"limit":100,"package":"nova_test"}}
```

Response notes:
- `count` equals `entities_returned + columns_returned`.
- `data.summary` includes `entities_returned`, `columns_returned`, and `items_returned`.

## Validation

### `validate_dag`
Check for cycles/orphans.

Optional:
- `detail` (`full` | `summary`)

```json
{"name":"validate_dag","arguments":{"detail":"summary"}}
```

## Metadata Inventory

### `show_metadata`
Project overview with entity counts.

```json
{"name":"show_metadata","arguments":{}}
```

### `list_tags`
All tags with counts.

```json
{"name":"list_tags","arguments":{}}
```

### `list_packages`
All packages with counts.

```json
{"name":"list_packages","arguments":{}}
```

### `list_databases`
All database.schema combinations with counts.

```json
{"name":"list_databases","arguments":{}}
```

## Warehouse

### `execute_sql`
Run SQL against the configured warehouse provider (default: Databricks).

Required:
- `statement` (optional when `preflight_only=true`)

Optional:
- `sql` - Compatibility alias for `statement`
- `row_limit` - Maximum rows to return
- `byte_limit` - Maximum bytes to return
- `wait_timeout_s` - Timeout for query completion
- `poll_interval_ms` - Polling interval for async queries
- `max_poll_seconds` - Max total polling duration for async queries
- `warehouse_id` - Override the default Databricks or Snowflake warehouse
- `parameters` - Named SQL parameters (e.g., `{"date": "2024-01-01"}`)
- `parameter_types` - SQL type hints for parameters (e.g., `{"date": "DATE"}`)
- `fetch_all_chunks` - Fetch all result pages (default: true)
- `max_chunks` - Limit the number of result pages fetched
- `preflight_only` - Run provider diagnostics without executing the main statement
- `preflight_catalog` - Optional catalog check during preflight
- `preflight_schema` - Optional schema check during preflight
- `preflight_relation` - Optional relation access check during preflight

Notes:
- `row_limit`, `byte_limit`, `max_chunks`, and `max_poll_seconds` are clamped by server config.
- `poll_interval_ms` is raised to a configured minimum when too low.
- Databricks supports named parameters and preflight checks.
- BigQuery provider is available via `DBT_NOVA_SQL_PROVIDER=bigquery` and supports named scalar parameters with optional `parameter_types`.
- Snowflake provider is available via `DBT_NOVA_SQL_PROVIDER=snowflake`; named parameters are rewritten to SQL API positional binds and null values require explicit `parameter_types`.
- DuckDB provider is available via `DBT_NOVA_SQL_PROVIDER=duckdb`; named parameters are supported, but `parameter_types` is not. Ad-hoc DuckDB file-scan functions in `execute_sql` text are rejected. Connection-level external access for trusted file-backed database objects is disabled unless `DBT_NOVA_DUCKDB_ALLOW_EXTERNAL_ACCESS=true` is paired with `DBT_NOVA_DUCKDB_FILE_SEARCH_PATH`.
- DuckDB reuses pooled read-only connections per `(duckdb_path,file_search_path,external_access)` key (`DBT_NOVA_DUCKDB_POOL_MAX_SIZE`).
- Object-level preflight checks (`preflight_catalog`, `preflight_schema`, `preflight_relation`) require a non-empty probe result across providers; missing/inaccessible targets return `ok=false`.
- BigQuery credentials can come from OAuth token env vars, `GOOGLE_APPLICATION_CREDENTIALS`, or gcloud ADC (same auth family used by GCS SDK mode).
- Snowflake credentials can use key-pair JWT, a supplied OAuth bearer token, a Snowflake programmatic access token, or local interactive external browser SSO (`DBT_NOVA_SNOWFLAKE_AUTH=externalbrowser`). Snowflake SQL API workload identity federation is not implemented yet.

```json
{"name":"execute_sql","arguments":{"statement":"select * from orders where order_date > :date","parameters":{"date":"2024-01-01"},"row_limit":100}}
```

Preflight example:
```json
{"name":"execute_sql","arguments":{"preflight_only":true,"preflight_relation":"analytics.orders"}}
```

## Operations

### `health`
Readiness and status (`loading`/`ready`/`refreshing`/`failed`) plus retriever info when ready/refreshing.

Includes `manifest_health` diagnostics for lineage metadata quality, including
models with `ref(...)` calls but no manifest dependencies.

Also includes `artifact_consumer` status when prebuilt artifact consumer mode is
configured (`enabled`, `fetch_policy`, validation/materialization flags, and
last evaluation/materialization timestamps).

`status=failed` indicates manifest initialization is not yet available. Keep the source
valid and allow the configured refresh interval to recover automatically.

```json
{"name":"health","arguments":{}}
```

### `reload_manifest`
Reload the manifest and rebuild indexes in the background.

Optional:
- `manifest_uri` or `manifest_path`
- `refresh_secs`, `storage_instance_id`

If no arguments are provided, Nova reloads the current manifest source. In MCP
server mode, changing `manifest_uri`, `manifest_path`, `refresh_secs`, or
`storage_instance_id` is disabled unless the operator sets
`DBT_NOVA_MCP_ENABLE_MANIFEST_RELOAD=1`.

```json
{"name":"reload_manifest","arguments":{"manifest_uri":"dbfs:///path/to/manifest.json"}}
```

MCP `reload_manifest` starts a background reload for the running server and
returns `status: "refreshing"` when accepted. CLI `dbt-nova manifest reload`
and CLI `dbt-nova tool call reload_manifest` are one-shot reloads that load the
target manifest before returning.

### `warm_manifest`
Warm semantic caches for the current manifest source.

Optional:
- `vector`, `sparse`, `reranker`
- `force`: require freshly rebuilt manifest-scoped cache files

When no component flag is supplied, `warm_manifest` requests vector and sparse
warmup, matching `dbt-nova manifest warm`. The tool uses the manifest source and
storage instance already configured for the running server or CLI `tool call`
load; it does not accept a new manifest path or URI.

This cache-write capability is disabled unless
`DBT_NOVA_MCP_ENABLE_MANIFEST_WARM=1` is set. Read-only storage is rejected.

```json
{"name":"warm_manifest","arguments":{"vector":true,"sparse":true}}
```

### `show_config`
Inspect operator configuration.

Optional:
- `defaults`: return built-in defaults instead of the active runtime config

`show_config` returns the active dbt-nova runtime configuration used by the
server or CLI `tool call` process. Credential values such as warehouse tokens
and private keys are read directly by providers from environment variables and
are not persisted in this config payload.

```json
{"name":"show_config","arguments":{"defaults":true}}
```

### `validate_config`
Validate operator configuration.

`validate_config` checks the active runtime configuration and returns the same
structured validation payload as `dbt-nova config validate --json`, including
the resolved `storage_instance_id` and `embedding_cache_dir`.

```json
{"name":"validate_config","arguments":{}}
```

### `inspect_storage`
Inspect Nova storage instances without mutating storage.

Optional:
- `storage_instance_id`: treat this instance as the configured instance for the
  response

The payload matches `dbt-nova storage inspect --json`: storage root, instances
directory, configured instance id, count, and per-instance metadata including
size, lock status, current manifest version, and version count.

```json
{"name":"inspect_storage","arguments":{}}
```

### `prune_storage`
Prune stale Nova storage instances.

Optional:
- `max_keep`: number of stale instances to retain
- `max_bytes`: total storage bytes to retain
- `storage_instance_id`: instance id to protect from pruning

This destructive operator tool is disabled unless
`DBT_NOVA_MCP_ENABLE_STORAGE_ADMIN=1` is set. When enabled, it returns the same
storage prune payload as the CLI plus a `safety_policy` object.

```json
{"name":"prune_storage","arguments":{"max_keep":1}}
```

### `cleanup_storage`
Remove the configured Nova storage instance when it is not in use.

Optional:
- `storage_instance_id`: instance id to remove

This destructive operator tool is disabled unless
`DBT_NOVA_MCP_ENABLE_STORAGE_ADMIN=1` is set. In-use storage directories are
left in place, matching CLI cleanup behavior, and the response includes a
`safety_policy` object.

```json
{"name":"cleanup_storage","arguments":{"storage_instance_id":"manifest-abc123"}}
```

---

## See Also

- [Quick Reference](quick-reference.md) - One-page tool cheatsheet
- [Response Format](response-format.md) - Understanding API responses
- [Error Codes](error-codes.md) - Handling errors
- [Search Ranking](../features/search-ranking.md) - How search results are ranked
- [Configuration Reference](../configuration/reference.md) - Environment variables
