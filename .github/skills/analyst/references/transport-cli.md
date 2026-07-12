# Analyst Transport: CLI

Use this reference when the client does not expose Nova MCP tools directly and you must work through the `dbt-nova` CLI in the terminal.

## Session contract

- Always pass `--manifest-path`.
- Reuse one `--storage-instance-id` for the whole analysis session.
- Always use `--json`.
- Prefer `--params-file` over long inline JSON when payloads get large.
- Run `dbt-nova health check` before substantive work.
- If health is not ready or `ready_for_traffic` is false, resolve the readiness
  problem before trusting search or SQL.
- `execute_sql` and `run_recipe` require valid warehouse environment variables.
- Do not run `manifest warm` during analyst work unless the user explicitly
  approves semantic cache warmup. For large or memory-constrained manifests,
  keep vector, sparse, and reranker search disabled in the environment.

## Transport mapping

Use the same analytical flow as the main skill; only the transport changes.

Common mappings:
- KPI resolution: `dbt-nova tool call search_indicator`
- inventory discovery: `dbt-nova tool call indicator_inventory`
- recipe discovery and execution: `dbt-nova tool call search_recipes`, `get_recipe`, `run_recipe`
- entity inspection: `dbt-nova tool call get_entity`
- field verification: `dbt-nova tool call get_columns`, `search_columns`, `column_inventory`
- trust checks: `dbt-nova tool call get_context`, `get_lineage`, `get_test_coverage`, `get_metadata_score`
- SQL execution: `dbt-nova tool call execute_sql`

For KPI, metric, measure, rate, funnel, or conversion questions, run
`search_indicator` before broad `search`, `get_context`, `get_sql`, or
`execute_sql`. Raw model search is fallback only after the CLI response shows
no relevant governed indicator or no credible semantic parent. Keep the
`search_indicator` command and rejection reason as final-answer evidence when
fallback is used. Inspect `indicator_source`, `execution_surface`, `queryable`,
`direct_sql_queryable`, and `queryable_via` before execution.

## CLI guardrails

- Use `tool call` for tool-surface parity first.
- Keep one stable manifest and storage instance for the whole session.
- Prefer `--params-file` or `--params-json` to avoid shell-escaping mistakes.
- Prefer `--params-file` for `execute_sql` when SQL contains quotes or newlines.
- Treat warehouse auth failures as execution blockers, not discovery failures.
- Treat `INDEX_BUILDING` as a readiness state. Poll health until
  `ready_for_traffic=true` instead of retrying analytical tools immediately.

## Execution

- Run a lightweight validation query before final aggregation when the question includes geography, channel, or segment filters.
- Use bounded result sizes on exploratory SQL.
- Use CLI JSON output as the evidence source for the final answer.

## Useful CLI examples

Health:

```bash
dbt-nova health check \
  --manifest-path /path/to/manifest.json \
  --storage-instance-id analyst-session \
  --json
```

KPI resolution:

```bash
dbt-nova tool call search_indicator \
  --manifest-path /path/to/manifest.json \
  --storage-instance-id analyst-session \
  --params-json '{"query":"ecommerce conversion rate checkout digital sessions","indicator_types":["metric"],"resource_types":["model"],"persona":"analyst","detail":"compact","group_mode":"top","limit":3,"include_support_signals":true}' \
  --json
```

For rate, conversion, funnel, or ratio questions, request metric indicators
first and copy any returned metric `expression` exactly into downstream SQL.
If the compact indicator row is relation-backed (`execution_surface: "relation"`,
`queryable: true`, `direct_sql_queryable: true`,
`queryable_via: "relation_name"`) and includes `relation_name`, `grain`, and
`expression`, do not run schema-inspection SQL before execution. If the row has
`queryable_via: "metricflow"`, use the externally configured MetricFlow/dbt
Semantic Layer path instead of Nova SQL. If it is metadata-only or not
queryable, report the execution blocker instead of writing inferred SQL.

Compact entity inspection:

```bash
dbt-nova tool call get_entity \
  --manifest-path /path/to/manifest.json \
  --storage-instance-id analyst-session \
  --params-json '{"id_or_name":"model.package.model_name","detail":"compact"}' \
  --json
```

Filter validation:

```bash
dbt-nova tool call execute_sql \
  --manifest-path /path/to/manifest.json \
  --storage-instance-id analyst-session \
  --params-file /tmp/execute-sql-params.json \
  --json
```

For multiline SQL, write JSON such as
`{"statement":"select ...","row_limit":50}` to the params file and pass that
file with `--params-file`.

When you need exact CLI flag or payload shapes, open:
- `docs/getting-started/cli.md`
- `docs/api/tools.md`
- `docs/features/recipes.md`
