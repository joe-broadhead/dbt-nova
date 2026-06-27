# BI Engineer Transport: MCP

Use this reference when the client exposes `mcp__nova__*` tools directly.

## Session contract

- Use MCP first for indicator resolution, contract checks, and filter validation.
- Use `health` when readiness is uncertain and `show_metadata` when manifest context matters.
- If `health` reports `ready_for_traffic=false` or a tool returns
  `INDEX_BUILDING`, wait for readiness before continuing.
- Use recipe tools when a recurring reporting workflow already exists; otherwise design from the entity contract.
- Treat warehouse failures as execution blockers, not design proof.
- Keep SQL bounded with explicit time/filter predicates, `row_limit`, `byte_limit`, small `max_chunks`, and finite timeouts.
- Do not run manifest reloads, full-table scans, or broad warmups from a BI design session.

## Practical order

1. `health` and optionally `show_metadata` to confirm the project and manifest.
2. `search_indicator` or `indicator_inventory` for KPI resolution.
3. `get_entity` or `get_context` for relation, grain, dimensions, indicators, and caveats.
4. `get_columns` first, then `search_columns` or `column_inventory` for unresolved filters and breakdowns.
5. `search_recipes {}` or targeted recipe search, then `get_recipe` before any `run_recipe`.
6. `compare_grains`, `diff_entities`, or `find_entity_overlap` before mixing entities.
7. `execute_sql` only for bounded validation of values, dataset shape, or filter examples.
8. `get_test_coverage`, `get_metadata_score`, `get_lineage`, or `get_impact` when trust or rollout risk matters.

## Recipe rules

- Match the recipe to the artifact, not just to a metric name.
- Inspect required parameters and query names with `get_recipe`.
- Run only the relevant query indexes or names.
- Pass the same period and entity contract that the dashboard spec will declare.
- Use `include_sql: false` unless the user asks for query-level detail.
