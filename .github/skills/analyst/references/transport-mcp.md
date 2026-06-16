# Analyst Transport: MCP

Use this reference when the client exposes `mcp__nova__*` tools directly.

## Session contract

- Use `show_metadata` for fast project identity and manifest scope checks.
- Use `health` only when readiness is uncertain or a prior tool suggests startup, refresh, or cache issues.
- If `health.status` is not `ready` on a shared hosted endpoint, stop and report the readiness state. Do not mutate server state from an analyst workflow.
- `run_recipe` uses the same warehouse execution path as `execute_sql`.
- A warehouse auth or provider failure is an execution blocker, not a discovery failure.

## Discovery order

1. `search_recipes` for recurring deliverables.
   - try a targeted query first
   - if that returns zero but the request still looks recurring, run `search_recipes {}`
   - if discovery is still inconclusive but the folder family is obvious, corroborate with `find_by_path`
   - if a recipe run already answers the business ask, do not drift back into broad `search` or generic parent discovery
2. `search_indicator` for KPI resolution
   - use domain-specific query terms
   - default to `limit: 3`, `detail: compact`, `group_mode: top`
   - keep `include_support_signals: true` when the question includes filter
     values such as country, channel, segment, device, or market labels
   - set `include_support_signals: false` only when the top rows already
     provide enough evidence and no filter-value mapping is needed
3. `indicator_inventory` when comparing KPI families
4. `search` for supporting entity discovery when the ask is not yet KPI-shaped

## Entity selection

- `get_entity detail=compact` is the default single-entity contract check.
- `batch_get_entities` is the fastest compact comparison tool for 2-3 shortlisted parents.
- `compare_grains` and `diff_entities` are tie-breakers when multiple parents remain plausible.

Use the compact summary to confirm:
- grain
- relevant measures or metrics
- relation name
- domains
- primary key columns

## Field verification

- `get_columns` is the default field-verification tool after choosing the entity.
- `search_columns` is best when you know the business term but not the exact field name.
- `column_inventory` is best for semantic family lookup across entities.
- `get_sql` is only for metric-logic or join verification when metadata is not enough.

## Trust escalation

- `get_context` when you need columns, tests, lineage, and docs together
- `get_lineage` / `get_column_lineage` when provenance matters
- `get_test_coverage` when reliability matters
- `get_metadata_score` when documenting caveats or choosing between similar entities
- Avoid `get_context`, `get_sql`, lineage tools, test coverage, and
  `detail=full` for simple KPI answers unless compact discovery is ambiguous or
  the user asks for provenance.

## Execution

- Run `execute_sql preflight_only=true` when the environment or provider is uncertain.
- Use bounded limits on exploratory queries.
- Use `parameters` for dynamic user values instead of string interpolation.
- Validate each non-trivial filter before final aggregation.
- Keep validation queries close to the target slice. Prefer the target window itself or a short recent lookback over open-ended historical scans.

## Recipe execution

- Inspect with `get_recipe include_queries=true include_sql=false` before `run_recipe`.
- If a recipe has an inventory or diagnostic query with no required parameters, run that first.
- Do not guess `recipe_id` values when discovery tools are available.

## Useful MCP examples

Project identity:

```json
{"name":"show_metadata","arguments":{}}
```

Recurring workflow discovery:

```json
{"name":"search_recipes","arguments":{"query":"proshop uplift","limit":5,"include_queries":true}}
```

KPI resolution:

```json
{"name":"search_indicator","arguments":{"query":"ecommerce conversion rate checkout digital sessions","indicator_types":["metric"],"resource_types":["model"],"persona":"analyst","detail":"compact","group_mode":"top","limit":3,"include_support_signals":true}}
```

For rate, conversion, funnel, or ratio questions, request metric indicators
first and copy any returned metric `expression` exactly into downstream SQL.
If the compact indicator row includes `relation_name`, `grain`, and
`expression`, do not run schema-inspection SQL before execution.

Compact entity inspection:

```json
{"name":"get_entity","arguments":{"id_or_name":"model.package.model_name","detail":"compact"}}
```

Filter validation:

```json
{"name":"execute_sql","arguments":{"statement":"select country_code, count(*) as rows from catalog.schema.table where event_ts >= :start_ts and event_ts < :end_ts group by 1 order by rows desc limit 50","parameters":{"start_ts":"2026-04-13","end_ts":"2026-04-20"},"row_limit":50}}
```
