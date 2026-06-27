# KPI Debugger Transport: CLI

Use this reference when you are working through the local `dbt-nova` CLI.

## Session contract

- Always pass `--manifest-path`.
- Reuse one `--storage-instance-id` for the full investigation session.
- Always use `--json`.
- Prefer `--params-file` for structured payloads.
- Run `health check` before substantive work.
- If health is not ready or `ready_for_traffic` is false, wait before using
  discovery or execution evidence.
- Keep warehouse execution bounded with explicit dates, filters, row/byte limits, and finite timeouts.
- Do not use CLI workarounds that warm a full manifest or scan a full table unless the user explicitly approves it.
- Treat `INDEX_BUILDING` as startup readiness evidence, not discrepancy
  evidence.

## CLI mapping

Use the same debugging flow as the main skill; only the transport changes.

Common mappings:
- KPI resolution: `tool call search_indicator`, `indicator_inventory`
- recipe discovery and execution: `tool call search_recipes`, `get_recipe`, `run_recipe`
- contract checks: `get_entity`, `get_context`, `get_columns`, `get_sql`
- filter discovery: `search_columns`
- bounded reproduction: `execute_sql`
- alternate-source checks: `compare_grains`, `find_entity_overlap`, `diff_entities`
- root-cause checks: `get_lineage`, `get_column_lineage`, `get_impact`
- trust checks: `get_test_coverage`, `get_metadata_score`

If CLI env vars, warehouse access, or manifest paths are missing, state the blocker and continue with MCP when available.
