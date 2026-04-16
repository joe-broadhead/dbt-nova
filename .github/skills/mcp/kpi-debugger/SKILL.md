---
name: mcp-kpi-debugger
description: "Investigates KPI discrepancies through Nova MCP tools. Use when reproducing a KPI, comparing it to alternate sources or periods, tracing lineage, and documenting suspected root causes with explicit evidence."
license: MIT
allowed-tools: "mcp__nova__search mcp__nova__search_indicator mcp__nova__indicator_inventory mcp__nova__search_columns mcp__nova__search_recipes mcp__nova__get_recipe mcp__nova__run_recipe mcp__nova__get_entity mcp__nova__get_columns mcp__nova__get_sql mcp__nova__get_lineage mcp__nova__get_column_lineage mcp__nova__get_test_coverage mcp__nova__get_metadata_score mcp__nova__execute_sql mcp__nova__reload_manifest mcp__nova__health Read"
metadata:
  owner: "dbt-nova"
  persona: "kpi-debugger"
  version: "0.0.3"
---

# MCP KPI Debugger Skill (dbt-nova)

## Transport contract

- Run `health` before substantive work.
- If status is not `ready`, run `reload_manifest` and wait for readiness.
- `run_recipe` uses the same warehouse execution path as `execute_sql`.
- Warehouse auth failures are execution blockers, not investigation failures.

## MCP surface

- `search_indicator`: canonical KPI resolution
- `indicator_inventory`: compare nearby KPI definitions before choosing one
- `search`: supporting discovery when the KPI name is ambiguous
- `search_columns`: resolve filter or segment fields during reproduction
- `search_recipes` / `get_recipe` / `run_recipe`: recurring reconciliations or reference workflows
- `get_entity` with `detail: "standard"`: compact semantic contract
- `get_columns` / `get_sql`: execution and definition checks
- `get_lineage` / `get_column_lineage`: provenance and root-cause checks
- `get_test_coverage` / `get_metadata_score`: optional trust signals after reproduction
- `execute_sql`: bounded KPI reproduction and comparison queries

## Load order

- Read `../../shared/kpi-debugger/references/workflow.md` first.
- Load the deeper investigation references only when needed:
  - `../../shared/kpi-debugger/references/metric-discrepancy-playbook.md`
  - `../../shared/kpi-debugger/references/root-cause-catalog.md`
  - `../../shared/kpi-debugger/references/variance-investigation-flow.md`

## Shared asset

- `../../shared/kpi-debugger/assets/investigation-template.md`

## References

- `../../shared/kpi-debugger/references/workflow.md`
- `../../shared/kpi-debugger/references/metric-discrepancy-playbook.md`
- `../../shared/kpi-debugger/references/root-cause-catalog.md`
- `../../shared/kpi-debugger/references/variance-investigation-flow.md`
- `../../shared/kpi-debugger/assets/investigation-template.md`
