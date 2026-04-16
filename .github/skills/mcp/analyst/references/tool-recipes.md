# Analyst Tool Recipes

## Table of contents

- Discovery
  - Resolve indicators directly (start here)
  - Inventory indicators
  - Search by Nova fields
  - Search columns for filters
- Inspection
  - Inspect entity
  - Inspect columns
  - Inspect SQL
- Trust and lineage
  - Upstream lineage
  - Column lineage
  - Test coverage
  - Metadata score
- Execution
  - Validate filter values
  - Execute final SQL
  - Health check

## Discovery

### Resolve indicators directly (start here)
**When:** Beginning any KPI analysis.
**Why:** `search_indicator` is the primary resolver for measures and metrics.

```json
{"name":"search_indicator","arguments":{"query":"conversion rate","indicator_types":["metric"],"resource_types":["model"],"persona":"analyst","limit":10}}
```

Key fields: `indicator_name`, `parent_unique_id`, `grain`, `parent_groups`, `match_type`.

### Inventory indicators
**When:** The ask is broad or repeated-term selection matters.
**Why:** Use a deterministic catalog instead of ranked search when comparing candidate KPIs.

```json
{"name":"indicator_inventory","arguments":{"indicator_types":["metric","measure"],"resource_types":["model"],"canonical_only":true,"limit":100,"offset":0}}
```

Key fields: `indicator_name`, `indicator_type`, `canonical`, `parent_unique_id`, `domains`, `grain`.

### Search by Nova fields
**When:** You already know the semantic target.
**Why:** Pinpoint models with specific measures/metrics/domains.

```text
nova_measures:sessions
nova_metric:conversion_rate
nova_domains:ecommerce AND nova_use_cases:weekly_report
```

Key fields: meta.nova.* fields present in search highlights.

### Search columns for filters
**When:** The filter field is unclear after choosing an entity.
**Why:** Find the best matching field for geography, channel, segment, or similar constraints.

```json
{"name":"search_columns","arguments":{"query":"market","resource_types":["model"],"limit":10,"offset":0}}
```

Key fields: `column_name`, `match_type`, `matched_value`, `parent_unique_id`, `semantic_type`.

## Inspection

### Inspect entity
**When:** You need a model definition and metadata.
**Why:** Validate grain, measures, and ownership.

```json
{"name":"get_entity","arguments":{"id_or_name":"model.package.model_name","detail":"standard"}}
```

Key fields: `nova_summary`, `relation_name`, `domains`, `synonyms`.

### Inspect columns
**When:** Confirm dimensions, PKs, or filter fields.
**Why:** Avoid invalid filters and wrong grain.

```json
{"name":"get_columns","arguments":{"id_or_name":"model.package.model_name"}}
```

Key fields: data_type, meta.primary_key, meta.nova.

### Inspect SQL
**When:** Validate computation logic.
**Why:** Ensure measure expressions are correct.

```json
{"name":"get_sql","arguments":{"id_or_name":"model.package.model_name","compiled":false}}
```

Key fields: SQL expressions for metrics, joins, filters.
Default to `compiled: false` unless the manifest definitely contains compiled SQL.

## Trust and lineage

### Upstream lineage
**When:** Impact or provenance matters.
**Why:** Confirm sources and upstream dependencies.

```json
{"name":"get_lineage","arguments":{"id_or_name":"model.package.model_name","direction":"upstream","depth":2,"resource_types":["source","model"],"detail":"standard"}}
```

Key fields: upstream nodes, source tables.

### Column lineage
**When:** A specific column drives a metric.
**Why:** Validate its origin and transformations.

```json
{"name":"get_column_lineage","arguments":{"id_or_name":"model.package.model_name","column_name":"session_date","direction":"upstream","depth":2,"confidence":"medium"}}
```

Key fields: matches and confidence levels.

### Test coverage
**When:** Results need higher trust.
**Why:** Confirm tests exist on key columns.

```json
{"name":"get_test_coverage","arguments":{"id_or_name":"model.package.model_name","include_full":false}}
```

Key fields: coverage_pct, missing_pk_tests.

### Metadata score
**When:** You need documentation/trust signal.
**Why:** Explain limitations in outputs.

```json
{"name":"get_metadata_score","arguments":{"id_or_name":"model.package.model_name","scope":"entity","persona":"analyst"}}
```

Key fields: score, grade, missing_fields.

### Context summary (lean by default)
**When:** You need fast triage without large doc payloads.
**Why:** Keep analysis flow high-signal.

```json
{"name":"get_context","arguments":{"id_or_name":"model.package.model_name","lineage_depth":1,"include_columns":true,"include_tests":true,"include_upstream":true,"include_downstream":false,"include_docs":false}}
```

Key fields: grain_summary, nova_summary, tests.summary, upstream.entities.

## Execution

### SQL preflight (provider + object access)
**When:** Starting in a new environment or after connection/config changes.
**Why:** Fail fast on provider/auth/schema issues before running expensive queries.

```json
{"name":"execute_sql","arguments":{"preflight_only":true,"preflight_relation":"analytics.orders"}}
```

Key fields: `provider`, `ready`, and `checks[*].ok`.
Provider detection: use `data.provider` to branch SQL syntax/rules (Databricks vs BigQuery vs DuckDB).
Interpretation: object checks only pass when the probe returns at least one row.

### Validate filter values
**When:** The question includes geography/segment filters.
**Why:** Prevent wrong mappings (for example UK/GB/United Kingdom confusion).

```json
{"name":"execute_sql","arguments":{"statement":"select <geo_col>, count(*) as rows from <relation> where <time_col> between <start> and <end> group by 1 order by rows desc limit 50"}}
```

Key fields: candidate value strings for exact SQL filter.

### Execute SQL
**When:** You must compute metrics.
**Why:** Produce the final numbers for reporting.

```json
{"name":"execute_sql","arguments":{"statement":"SELECT ..."}}
```

Key fields: result rows and column names.

### Execute parameterized SQL (when injecting user values)
**When:** The query contains dynamic filters (dates, country, channel).
**Why:** Keep statements deterministic and avoid string interpolation errors.

```json
{"name":"execute_sql","arguments":{"statement":"select * from analytics.orders where order_date between :start_date and :end_date and country_code = :country_code","parameters":{"start_date":"2026-02-01","end_date":"2026-02-07","country_code":"GB"},"row_limit":5000}}
```

Key fields: `parameters` values used at execution time.
Note: `parameter_types` is optional for Databricks/BigQuery and not supported by DuckDB.

### Execute final SQL (sessions + CR pattern)
**When:** User asks for a volume metric and conversion rate together.
**Why:** Keep one scoped dataset and compute both metrics consistently.

```json
{"name":"execute_sql","arguments":{"statement":"with scoped as (...) select sessions, conversion_rate from ..."}}
```

Key fields: sessions, conversion_rate, and any requested breakdown columns.

### Health check
**When:** After reloads or if queries fail unexpectedly.
**Why:** Ensure manifest is ready.

```json
{"name":"health","arguments":{}}
```

Key fields: status, refresh details.

### Find by path (model scoped)
**When:** You know the folder structure and want models only.
**Why:** Avoid test-only matches.

```json
{"name":"find_by_path","arguments":{"path_pattern":"models/**/ecommerce/**","resource_types":["model"],"limit":10,"detail":"standard"}}
```

## End-to-end recipe: "sessions and CR last week for UK"

1. `search_indicator` separately for each requested indicator.
2. Choose one execution entity from the top shared `parent_groups` result.
3. `get_entity detail=standard` on that parent.
4. `get_columns` to pick `<time_col>` and `<geo_col>`.
5. `execute_sql` filter-value validation query to find UK value(s).
6. `execute_sql` final aggregate query with validated value(s).
7. Return result table and include: selected relation, selected columns, validated UK mapping.
