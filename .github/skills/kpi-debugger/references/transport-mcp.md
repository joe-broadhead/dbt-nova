# KPI Debugger Transport: MCP

Use this reference when the client exposes `mcp__nova__*` tools directly.

## Session contract

- Use MCP first for KPI resolution, contract checks, recipe discovery, and bounded reproduction.
- Use `health` when readiness is uncertain.
- If `health` reports `ready_for_traffic=false` or a tool returns
  `INDEX_BUILDING`, wait for readiness before continuing.
- Use `show_metadata` when the manifest/project context matters.
- Use recipes only when a recurring reconciliation or debug workflow already exists and its required parameters are clear.
- Treat warehouse failures as execution blockers, not diagnosis proof.
- Keep SQL bounded with explicit time predicates, filter predicates, `row_limit`, `byte_limit`, small `max_chunks`, and finite timeouts.
- Do not run broad `count(*)`, full-table scans, or manifest warmups from this skill.

## Practical order

1. `health` and, if needed, `show_metadata` to confirm readiness and manifest context.
2. `search_indicator` or `indicator_inventory` to resolve the canonical KPI.
3. `get_entity` or `get_context` for relation, grain, dimensions, and indicator definitions.
4. `search_columns` and `get_columns` to validate time fields, filters, dimensions, and numerator/denominator columns.
5. `execute_sql` for bounded reproduction. Relation preflight is useful, but a preflight failure is only a blocker/warning until confirmed.
6. `search_recipes {}` or a specific recipe search, then `get_recipe` before any `run_recipe`.
7. `compare_grains`, `find_entity_overlap`, or `diff_entities` when comparing alternate sources.
8. `get_lineage` or `get_column_lineage` only after the discrepancy is localized.
9. `get_test_coverage`, `get_metadata_score`, or `get_impact` when trust, reliability, or blast radius matters.

## Recipe rules

- Prefer `get_recipe` before `run_recipe`; inspect required parameters and query names.
- Run only the relevant query indexes or names when a recipe has many queries.
- Pass the same date/filter contract used in the canonical reproduction.
- Use `include_sql: false` unless query-level debugging needs raw SQL.
- If required parameters are missing or ambiguous, report the blocker instead of guessing.
