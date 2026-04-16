---
name: mcp-bi-engineer
description: "Designs dashboard-ready analytical products through Nova MCP tools. Use when turning canonical indicators and execution entities into dashboard specs, metric cards, dataset contracts, and visualization QA outputs."
license: MIT
allowed-tools: "mcp__nova__search mcp__nova__search_indicator mcp__nova__indicator_inventory mcp__nova__search_columns mcp__nova__column_inventory mcp__nova__search_recipes mcp__nova__get_recipe mcp__nova__run_recipe mcp__nova__get_entity mcp__nova__get_columns mcp__nova__get_sql mcp__nova__execute_sql mcp__nova__get_test_coverage mcp__nova__get_metadata_score mcp__nova__reload_manifest mcp__nova__health Read"
metadata:
  owner: "dbt-nova"
  persona: "bi-engineer"
  version: "0.0.3"
---

# MCP BI Engineer Skill (dbt-nova)

## Transport contract

- Run `health` before substantive work.
- If status is not `ready`, run `reload_manifest` and wait for readiness.
- `run_recipe` uses the same warehouse execution path as `execute_sql`.
- Warehouse auth failures are execution blockers, not discovery failures.

## MCP surface

- `search_indicator`: canonical KPI resolution
- `indicator_inventory`: indicator-set inventory for dashboard sections
- `search_recipes` / `get_recipe` / `run_recipe`: recurring reporting scaffolds
- `get_entity` with `detail: "standard"`: compact semantic contract
- `search_columns` / `column_inventory` / `get_columns`: supported dimensions and filter fields
- `get_sql`: execution logic inspection when needed
- `execute_sql`: bounded validation of filters and dataset shape
- `get_test_coverage` / `get_metadata_score`: optional trust signals for dashboard handoff

## Load order

- Read `../../shared/bi-engineer/references/workflow.md` first.
- Load the specific design references only when the workflow reaches them:
  - `../../shared/bi-engineer/references/dashboard-design-workflow.md`
  - `../../shared/bi-engineer/references/chart-selection-matrix.md`
  - `../../shared/bi-engineer/references/grain-and-aggregation-rules.md`
  - `../../shared/bi-engineer/references/filter-design-contracts.md`
  - `../../shared/bi-engineer/references/metric-card-patterns.md`

## Shared assets

- `../../shared/bi-engineer/assets/dashboard-spec-template.md`
- `../../shared/bi-engineer/assets/metric-card-template.md`
- `../../shared/bi-engineer/assets/dataset-contract-template.md`
- `../../shared/bi-engineer/assets/viz-qa-checklist.md`

## References

- `../../shared/bi-engineer/references/workflow.md`
- `../../shared/bi-engineer/references/dashboard-design-workflow.md`
- `../../shared/bi-engineer/references/chart-selection-matrix.md`
- `../../shared/bi-engineer/references/grain-and-aggregation-rules.md`
- `../../shared/bi-engineer/references/filter-design-contracts.md`
- `../../shared/bi-engineer/references/metric-card-patterns.md`
- `../../shared/bi-engineer/assets/dashboard-spec-template.md`
- `../../shared/bi-engineer/assets/metric-card-template.md`
- `../../shared/bi-engineer/assets/dataset-contract-template.md`
- `../../shared/bi-engineer/assets/viz-qa-checklist.md`
