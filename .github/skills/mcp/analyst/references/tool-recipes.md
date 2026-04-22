# Analyst Tool Recipes

## Table of contents

- Session and scope
  - Confirm project identity
  - Check readiness only when needed
- Discovery
  - Resolve indicators directly (start here)
  - Inventory indicators
  - Search by business/domain terms
  - Search recipes
  - Path-based fallback discovery
- Entity selection
  - Inspect shortlisted parent entities
  - Inspect one chosen entity
  - Compare candidate grains
  - Diff two candidate entities
- Field verification
  - Inspect columns on the chosen entity
  - Search columns for a likely filter field
  - Find semantic field families across entities
  - Inspect SQL
  - One-shot context bundle
  - Trust and provenance checks
- Execution
  - SQL preflight
  - Validate filter values
  - Execute parameterized SQL
  - Run a recipe
  - End-to-end pattern

## Session and scope

### Confirm project identity
**When:** Freshness, package scope, or manifest size matters.
**Why:** `show_metadata` is a fast, low-noise check compared with `health`.

```json
{"name":"show_metadata","arguments":{}}
```

Key fields: `metadata.project_name`, `metadata.dbt_version`, `entity_counts`, `total_entities`.

### Check readiness only when needed
**When:** The endpoint was just reconnected, tools fail unexpectedly, or you suspect a startup/refresh issue.
**Why:** `health` is operationally rich, but heavier than `show_metadata`.

```json
{"name":"health","arguments":{}}
```

Key fields: `status`, `ready_for_traffic`, `manifest.hash`, `search.*.ready`, `artifact_consumer.*`.

Rule: if `status != "ready"` on a shared hosted endpoint, stop and report the readiness state. Do not try to mutate server state from an analyst workflow.

## Discovery

### Resolve indicators directly (start here)
**When:** Beginning any KPI analysis.
**Why:** `search_indicator` is the primary business-term to execution-parent resolver.

```json
{"name":"search_indicator","arguments":{"query":"conversion rate","indicator_types":["metric"],"resource_types":["model"],"persona":"analyst","limit":10}}
```

Key fields: `indicator_name`, `indicator_type`, `parent_unique_id`, `parent_name`, `grain`, `match_type`, `parent_groups`.

Interpretation rule: reason from `parent_groups` first, then shortlist 1-3 parent entities for inspection.

### Inventory indicators
**When:** The ask is broad, catalog-style, or you need to compare similar KPIs without ranked search.
**Why:** `indicator_inventory` is deterministic and avoids overfitting to one search phrase.

```json
{"name":"indicator_inventory","arguments":{"indicator_types":["metric","measure"],"resource_types":["model"],"canonical_only":true,"limit":100,"offset":0}}
```

Key fields: `indicator_name`, `indicator_type`, `canonical`, `parent_unique_id`, `domains`, `grain`.

### Search by business or domain terms
**When:** The ask is not cleanly KPI-shaped yet or you need supporting entity discovery.
**Why:** `search` helps locate likely execution models and domain-specific candidates.

```json
{"name":"search","arguments":{"query":"sales enriched ecommerce country performance","persona":"analyst","detail":"standard","limit":10}}
```

Rule: use `search` to support entity discovery, not to replace `search_indicator` for KPI resolution.

### Search recipes
**When:** The request looks like a recurring workflow, reference pack, report deck, or reconciliation.
**Why:** Recipes are the most deterministic path when available.

Targeted discovery first:

```json
{"name":"search_recipes","arguments":{"query":"proshop uplift","limit":5,"include_queries":true}}
```

If targeted search returns zero but the request still looks recurring, run:

```json
{"name":"search_recipes","arguments":{}}
```

Key fields: `id`, `topic`, `required_parameters`, `optional_parameters`, `query_names`, `path`.

### Path-based fallback discovery
**When:** You know a folder family or want to verify that recipe-backed analyses exist in the manifest.
**Why:** `find_by_path` is a deterministic fallback when search ranking is not enough.

```json
{"name":"find_by_path","arguments":{"path_pattern":"analyses/recipes/**","resource_types":["analysis"],"detail":"standard","limit":20}}
```

Useful scoped companions:
- `list_entities` for package-specific model or analysis discovery
- `list_packages` for package inventory
- `list_tags` for tag inventory
- `list_databases` for physical relation scope

## Entity selection

### Inspect shortlisted parent entities
**When:** `search_indicator` returns several plausible parent models.
**Why:** `batch_get_entities` is the fastest compact comparison tool for 2-3 candidates.

```json
{"name":"batch_get_entities","arguments":{"unique_ids":["model.package.parent_a","model.package.parent_b"],"detail":"standard"}}
```

Key fields: `entities[*].nova_summary`, `relation_name`, `primary_key_columns`, `package_name`, `original_file_path`.

Selection rule: prefer the entity with the clearest grain, relevant measures/metrics, and the fewest assumptions for the requested filters.

### Inspect one chosen entity
**When:** You need the compact contract for the most likely parent.
**Why:** `get_entity detail=standard` is the default single-entity contract check.

```json
{"name":"get_entity","arguments":{"id_or_name":"model.package.parent_a","detail":"standard"}}
```

Key fields: `nova_summary`, `relation_name`, `domains`, `synonyms`, `primary_key_columns`.

### Compare candidate grains
**When:** Two parents expose similar indicators but roll up differently.
**Why:** `compare_grains` is the fastest tie-breaker for effective grain.

```json
{"name":"compare_grains","arguments":{"entity1":"model.package.parent_a","entity2":"model.package.parent_b"}}
```

Key fields: `shared_dimensions`, `entity1_only_dimensions`, `entity2_grain_variants`, `exact_match`.

### Diff two candidate entities
**When:** You need a column-level comparison of similar parents.
**Why:** `diff_entities` helps explain why one parent is safer for the question.

```json
{"name":"diff_entities","arguments":{"entity1":"model.package.parent_a","entity2":"model.package.parent_b","compare_fields":["columns"]}}
```

## Field verification

### Inspect columns on the chosen entity
**When:** After selecting the execution relation.
**Why:** `get_columns` is the main way to confirm time fields, filter fields, identifiers, and semantic annotations.

```json
{"name":"get_columns","arguments":{"id_or_name":"model.package.model_name"}}
```

Key fields: `columns[*].name`, `columns[*].data_type`, `columns[*].meta.primary_key`, `columns[*].meta.nova`.

### Search columns for a likely filter field
**When:** The chosen entity is correct but the exact geography, channel, or segment column is still unclear.
**Why:** `search_columns` gives ranked column candidates by business term.

```json
{"name":"search_columns","arguments":{"query":"country code","resource_types":["model"],"limit":10,"offset":0}}
```

Key fields: `column_name`, `match_type`, `matched_value`, `parent_unique_id`, `semantic_type`.

### Find semantic field families across entities
**When:** You know the semantic type you need, but not the exact model yet.
**Why:** `column_inventory` gives deterministic semantic lookup across entities.

```json
{"name":"column_inventory","arguments":{"resource_types":["model"],"roles":["dimension"],"semantic_types":["country_code"],"annotated_only":true,"limit":20,"offset":0}}
```

Key fields: `column_name`, `semantic_type`, `role`, `example_values`, `parent_unique_id`, `domains`.

Best use: cross-entity field family lookup before the final parent choice, not final field confirmation after the choice.

### Inspect SQL
**When:** You need to verify joins, metric logic, or a derived formula.
**Why:** `get_sql` is the direct inspection path when metadata is not enough.

```json
{"name":"get_sql","arguments":{"id_or_name":"model.package.model_name","compiled":false}}
```

Rule: default to raw SQL unless the manifest definitely contains compiled SQL and you need it.

### One-shot context bundle
**When:** You need columns, tests, lineage, and docs together for one entity.
**Why:** `get_context` is the fastest bundled inspection path.

```json
{"name":"get_context","arguments":{"id_or_name":"model.package.model_name","lineage_depth":1,"include_columns":true,"include_tests":true,"include_upstream":true,"include_downstream":false,"include_docs":false}}
```

### Trust and provenance checks
Use these only when they materially affect the answer.

Upstream lineage:

```json
{"name":"get_lineage","arguments":{"id_or_name":"model.package.model_name","direction":"upstream","depth":2,"resource_types":["source","model"],"detail":"standard"}}
```

Column lineage:

```json
{"name":"get_column_lineage","arguments":{"id_or_name":"model.package.model_name","column_name":"session_date","direction":"upstream","depth":2,"confidence":"medium"}}
```

Test coverage:

```json
{"name":"get_test_coverage","arguments":{"id_or_name":"model.package.model_name","include_full":false}}
```

Metadata score:

```json
{"name":"get_metadata_score","arguments":{"id_or_name":"model.package.model_name","scope":"entity","persona":"analyst"}}
```

## Execution

### SQL preflight
**When:** Starting in a new environment or after warehouse/provider issues.
**Why:** Fail fast on auth, provider, or relation-access problems.

```json
{"name":"execute_sql","arguments":{"preflight_only":true,"preflight_relation":"catalog.schema.table_name"}}
```

Key fields: `provider`, `ready`, `checks[*].ok`.

Rule: object checks only pass when the probe returns a non-empty result.

### Validate filter values
**When:** The question includes geography, channel, seller type, or segment filters.
**Why:** Prevent wrong mappings such as `UK` vs `GB`.

```json
{"name":"execute_sql","arguments":{"statement":"select economical_businessunit_country_code, count(*) as rows from datalake_insight_analytics.omnicommerce.base__sales_enriched where gmv_recorded_at between :start_ts and :end_ts group by 1 order by rows desc limit 50","parameters":{"start_ts":"2026-04-06","end_ts":"2026-04-12"},"row_limit":50}}
```

Key fields: exact warehouse values to use in the final filter.

Rule: validate each non-trivial filter before final aggregation.

### Execute parameterized SQL
**When:** Computing the final KPI answer.
**Why:** Keep filters deterministic and avoid string interpolation errors.

```json
{"name":"execute_sql","arguments":{"statement":"select sum(gmv_amount_euros) as gmv from datalake_insight_analytics.omnicommerce.base__sales_enriched where economical_businessunit_country_code = :country_code and gmv_recorded_at >= :start_ts and gmv_recorded_at < :end_ts","parameters":{"country_code":"ES","start_ts":"2026-04-13","end_ts":"2026-04-20"},"row_limit":1000}}
```

### Run a recipe
**When:** Recipe discovery or prior context gives you a valid `recipe_id`.
**Why:** Recipes are the most deterministic execution path for recurring workflows.

```json
{"name":"get_recipe","arguments":{"recipe_id":"reference/proshop_uplift_daily","include_queries":true,"include_sql":false}}
```

Inventory-first execution when available:

```json
{"name":"run_recipe","arguments":{"recipe_id":"reference/proshop_uplift_daily","query_indexes":[1],"row_limit":5,"wait_timeout_s":120}}
```

Rule: when a recipe has an inventory or diagnostic step with no required parameters, run that first before heavier parameterized steps.

## End-to-end pattern: "sessions and CR last week for the UK"

1. Decompose the ask into:
   - indicators: `sessions`, `conversion rate`
   - time window: `last week`
   - filter: `UK`
   - breakdown: none
   - comparison mode: none unless implied
2. `search_recipes` only if the ask sounds like a recurring report.
3. `search_indicator` separately for `sessions` and `conversion rate`.
4. Choose one common parent from the top shared `parent_groups`.
5. `get_entity detail=standard` or `batch_get_entities` on the shortlisted parent ids.
6. `compare_grains` if two parents still look plausible.
7. `get_columns` to confirm the time field, the country field, and any numerator/denominator fields if CR is derived.
8. `execute_sql` validation query to confirm the exact UK warehouse value, such as `GB`.
9. `execute_sql` final aggregate query with validated dates and filter values.
10. Return the result with explicit evidence: indicators selected, relation used, grain, time field, filter field, and validated warehouse value.
