# Tools Reference

This reference lists all MCP tools exposed by dbt‑nova, grouped by category.
All tools return the standard envelope described in [Response Format](response-format.md).

> Tip: use `persona` in `search` to tune ranking (`analyst`, `engineer`, `governance`).

## Discovery

### `search`
Full‑text and hybrid search across models, sources, tests, macros, etc.

Required:
- `query`

Common:
- `resource_types` (array)
- `persona` (string)
- `detail` (`standard` | `full`)
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

- `standard`: persona‑optimized summary (default)
- `full`: complete entity payload (same as `get_entity` with `detail: "full"`)

When `detail: "standard"` and the query matches Nova measures or metrics, search
results include a compact `semantic_preview` with the matched measure/metric
name, description, expression, canonical flag, and match type. For analyst KPI
resolution, prefer `search_indicator` before broad `search`.

When `explain: true`, each result row includes an `explain` block with retrieval
and scoring factors, and the top-level response includes an `explain` payload
with query tokens, retrievers used, and the active ranking config snapshot.

### `search_indicator`
Search Nova measures and metrics directly, then return the parent execution
entity and grain context.

Required:
- `query`

Common:
- `indicator_types` (`["metric"]`, `["measure"]`, or omitted for both)
- `resource_types` (filters parent entity types)
- `persona` (string, defaults to `analyst`)
- `explain` (bool, includes deterministic ranking/debug breakdowns)
- `limit`, `offset`, `min_score`

```json
{"name":"search_indicator","arguments":{"query":"average order value","indicator_types":["metric"],"resource_types":["model"],"persona":"analyst","limit":5,"offset":0}}
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

### `indicator_inventory`
List Nova measures and metrics deterministically, with parent execution context.
Use this when you need a flat semantic catalog instead of ranked search results.

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
Audit project-level overlap, duplicate indicators, canonical conflicts, and grain drift.

Common:
- `resource_types`
- `limit` (max rows per report section)
- `min_score` (applies to overlap section)

```json
{"name":"modelling_consistency_report","arguments":{"resource_types":["model"],"limit":20}}
```

### `get_entity`
Fetch a single entity by `unique_id` or name.

Required:
- `id_or_name`

Optional:
- `resource_type`
- `detail` (`standard` | `full`, default: `standard`)

Notes:
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
- `detail`, `limit`, `offset`

```json
{"name":"list_entities","arguments":{"resource_type":"model","tags":["pii"],"detail":"standard","limit":100}}
```

### `batch_get_entities`
Retrieve multiple entities at once.

Required:
- `unique_ids`

Optional:
- `detail`

```json
{"name":"batch_get_entities","arguments":{"unique_ids":["model.a","model.b"],"detail":"full"}}
```

### `find_by_path`
Find entities matching a file path glob.

Required:
- `path_pattern`

Optional:
- `resource_types`, `detail`, `limit`, `offset`

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

Project scope responses include `quality_summary.test_coverage`, aggregated
across the returned entities and use deterministic ordering for pagination.

```json
{"name":"get_metadata_score","arguments":{"id_or_name":"model.jaffle_shop.orders","persona":"analyst","scope":"entity"}}
```

See `docs/features/metadata-scoring.md` for scoring rules, personas, and examples.

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
- DuckDB provider is available via `DBT_NOVA_SQL_PROVIDER=duckdb`; named parameters are supported, but `parameter_types` is not.
- DuckDB reuses pooled read-only connections per `(duckdb_path,file_search_path)` key (`DBT_NOVA_DUCKDB_POOL_MAX_SIZE`).
- Object-level preflight checks (`preflight_catalog`, `preflight_schema`, `preflight_relation`) require a non-empty probe result across providers; missing/inaccessible targets return `ok=false`.
- BigQuery credentials can come from OAuth token env vars, `GOOGLE_APPLICATION_CREDENTIALS`, or gcloud ADC (same auth family used by GCS SDK mode).
- Snowflake credentials can use key-pair JWT, OAuth bearer tokens, or Snowflake programmatic access tokens.

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
Reload the manifest from a new source and rebuild indexes in the background.

Optional:
- `manifest_uri` or `manifest_path`
- `refresh_secs`, `storage_instance_id`

If no arguments are provided, Nova reloads the current manifest source.

```json
{"name":"reload_manifest","arguments":{"manifest_uri":"dbfs:///path/to/manifest.json"}}
```

---

## See Also

- [Quick Reference](quick-reference.md) - One-page tool cheatsheet
- [Response Format](response-format.md) - Understanding API responses
- [Error Codes](error-codes.md) - Handling errors
- [Search Ranking](../features/search-ranking.md) - How search results are ranked
- [Configuration Reference](../configuration/reference.md) - Environment variables
