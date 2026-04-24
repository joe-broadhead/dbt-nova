# BI Engineer Transport: CLI

Use this reference when you are working through the local `dbt-nova` CLI.

## Session contract

- Always pass `--manifest-path`.
- Reuse one `--storage-instance-id` for the full design session.
- Always use `--json`.
- Prefer `--params-file` for structured payloads.
- Run `health check` before substantive work.
- Keep warehouse validation bounded with explicit time/filter predicates, row/byte limits, and finite timeouts.
- Do not trigger full manifest warmups or broad warehouse scans unless the user explicitly approves them.

## CLI mapping

Use the same design flow as the main skill; only the transport changes.

Common mappings:
- KPI resolution: `tool call search_indicator`, `indicator_inventory`
- recipe discovery and execution: `tool call search_recipes`, `get_recipe`, `run_recipe`
- contract checks: `get_entity`, `get_context`, `get_columns`, `get_sql`
- filter and breakdown discovery: `search_columns`, `column_inventory`
- entity comparison: `compare_grains`, `diff_entities`, `find_entity_overlap`
- validation and trust: `execute_sql`, `get_test_coverage`, `get_metadata_score`, `get_lineage`, `get_impact`

If CLI env vars, manifest paths, or warehouse credentials are missing, state the blocker and continue with MCP when available.
