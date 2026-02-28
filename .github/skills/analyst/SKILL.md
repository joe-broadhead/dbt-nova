---
name: analyst
description: "Analyzes dbt metrics and KPIs for business intelligence. Use when asking business questions, calculating metrics, comparing YoY performance, validating measures, discovering KPIs, generating reports, or querying the data warehouse. Supports metric lookup, grain validation, dimension filtering, and standardized report outputs."
license: MIT
allowed-tools: "mcp__nova__search mcp__nova__search_recipes mcp__nova__get_recipe mcp__nova__run_recipe mcp__nova__get_entity mcp__nova__get_columns mcp__nova__get_sql mcp__nova__get_lineage mcp__nova__get_column_lineage mcp__nova__get_context mcp__nova__get_test_coverage mcp__nova__get_metadata_score mcp__nova__find_by_path mcp__nova__execute_sql mcp__nova__health Read"
metadata:
  owner: "dbt-nova"
  persona: "analyst"
  version: "0.0.2"
---

# Analyst Skill (dbt-nova)

## Mission

Turn business questions into correct, reproducible SQL answers with explicit evidence:
- which metric definition was used
- which relation was queried
- which time and geo columns were selected
- how filter values were validated

## Execution contract (required)

1) Parse the question into required parts
- Extract: metric(s), measure(s), time window, geography/segment filters, requested breakdown, comparison mode (YoY/WoW/etc).
- If any required part is missing, ask one clarification question before querying.

2) Discover candidates
- Run `search` with `persona: "analyst"` and `detail: "standard"`.
- Search both business terms and metric shorthand/synonyms (for example: `cr`, `conversion rate`, `sessions`, 'gmv').
- Keep top candidates only; do not fan out to many entities.

3) Check for a reusable recipe (required for recurring asks)
- For recurring workflows (weekly report, MBR, channel pack), run `search_recipes` first.
- If matched, inspect `get_recipe` and execute via `run_recipe`.
- Use ad-hoc SQL only for uncovered gaps after running the recipe.

4) Select execution entity
- Use `get_entity` on candidates and choose one execution relation.
- Prefer entities with:
  - clear metric/measure definition
  - explicit grain (`meta.nova.grain`)
  - available time + geo dimensions
  - acceptable test coverage
- Record selection rationale in the final answer.

5) Resolve metric/time/geo fields
- Use `get_columns` + `get_entity` to identify:
  - metric expression or numerator/denominator components
  - time column
  - geo column
- Never assume a geo value mapping (for example UK -> GB) without validating actual warehouse values.

6) Validate filter values before final aggregation
- Run a lightweight `execute_sql` distinct/check query for time+geo fields.
- Confirm the exact filter values to be used in final SQL.

7) Run final SQL
- Use measure expressions verbatim when defined in metadata.
- For rate metrics, compute from validated numerator/denominator unless a canonical rate expression is defined.
- Default weekly standard: Sunday-Saturday.
- Default YoY alignment: 364-day day-of-week alignment.

8) Report with evidence
- Use `assets/report-template.md`.
- Always include:
  - selected entity (`unique_id`, `relation_name`)
  - selected time column and geo column
  - validated filter value(s)
  - metric definition source (measure expression, metric expression, or derived formula)

## Examples

### Example: "Give me ecommerce sessions and CR last week for the UK"
1. `search` for sessions and conversion-rate candidates (`persona: analyst`)
2. `get_entity` to pick the execution relation and metric definitions
3. `get_columns` to identify time + country columns
4. `execute_sql` distinct check to resolve actual UK value(s)
5. `execute_sql` final query for sessions + CR with aligned last-week window
6. Return result table + evidence block

## Tool usage (quick map)

- `search` (persona: analyst): discovery
- `search_recipes` / `get_recipe` / `run_recipe`: deterministic recurring workflows
- `get_entity` / `get_columns`: definitions + grain
- `get_sql`: validate SQL logic (raw or compiled)
- `get_lineage` / `get_column_lineage`: trust + provenance
- `get_test_coverage`: data quality signals
- `get_metadata_score`: documentation/trust scoring
- `execute_sql`: run queries when needed
- `health`: confirm readiness after manifest reloads

## SQL execution guardrails (required)

- Assume provider defaults to `databricks` unless `DBT_NOVA_SQL_PROVIDER` is set to `bigquery` or `duckdb`.
- For unfamiliar environments, run `execute_sql` preflight first (`preflight_only: true` plus `preflight_catalog`/`preflight_schema`/`preflight_relation` when relevant).
- Read `data.provider` from the preflight response to identify the active SQL provider before writing provider-specific SQL.
- Treat object checks as pass only when `ok: true`; object preflight checks require non-empty probe results.
- Set bounded query controls on exploratory queries (`row_limit`, `byte_limit`, `max_chunks`); server-side config may clamp these values.
- Use `parameters` for injected user values. Use `parameter_types` only when needed, and never with DuckDB.
- `run_recipe` executes through the same SQL provider and limit guards as `execute_sql`.

## Output standard (required)

- Include current, prior (YoY), delta (abs), delta (%) for counts.
- For rates, include delta in percentage points.
- State assumptions and grain explicitly.

## Validation checklist (copy and complete)

[ ] Recipe lookup performed (`search_recipes`) for recurring workflow requests
[ ] Grain confirmed (rows == distinct primary key)
[ ] Execution entity selected and justified
[ ] Time column selected
[ ] Geo column selected
[ ] Geo filter values validated with SQL
[ ] SQL preflight run when environment/provider access was uncertain
[ ] Time window specified
[ ] Measure expressions verified
[ ] YoY alignment correct (364 days)
[ ] Results sanity-checked

## Payload and detail guidance (required)

- Use `detail: "standard"` for `search`, `get_lineage`, and `find_by_path` to keep payloads high-signal.
- Use `detail: "full"` only when you need full column metadata or long descriptions.
- Prefer `get_context` with `include_docs: false` unless you explicitly need linked documentation.

Example:
```json
{"name":"get_context","arguments":{"id_or_name":"model.package.model_name","lineage_depth":1,"include_columns":true,"include_tests":true,"include_upstream":true,"include_downstream":false,"include_docs":false}}
```

For file discovery, always specify model scope:
```json
{"name":"find_by_path","arguments":{"path_pattern":"models/**/ecommerce/**","resource_types":["model"],"limit":10,"detail":"standard"}}
```

## References

- `references/analysis-workflow.md`
- `references/tool-recipes.md`
