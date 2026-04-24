# Analyst Transport: CLI

Use this reference when the client does not expose Nova MCP tools directly and you must work through the `dbt-nova` CLI in the terminal.

## Session contract

- Always pass `--manifest-path`.
- Reuse one `--storage-instance-id` for the whole analysis session.
- Always use `--json`.
- Prefer `--params-file` over long inline JSON when payloads get large.
- Run `dbt-nova health check` before substantive work.
- If health is not ready, resolve the readiness problem before trusting search or SQL.
- `execute_sql` and `run_recipe` require valid warehouse environment variables.

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

## CLI guardrails

- Use `tool call` for tool-surface parity first.
- Keep one stable manifest and storage instance for the whole session.
- Prefer `--params-file` or `--params-json` to avoid shell-escaping mistakes.
- Treat warehouse auth failures as execution blockers, not discovery failures.

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
  --params-json '{"query":"conversion rate","indicator_types":["metric"],"resource_types":["model"],"persona":"analyst","limit":10}' \
  --json
```

Compact entity inspection:

```bash
dbt-nova tool call get_entity \
  --manifest-path /path/to/manifest.json \
  --storage-instance-id analyst-session \
  --params-json '{"id_or_name":"model.package.model_name","detail":"standard"}' \
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

When you need exact CLI flag or payload shapes, open:
- `docs/getting-started/cli.md`
- `docs/api/tools.md`
- `docs/features/recipes.md`
