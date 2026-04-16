---
name: cli-bi-engineer
description: "Designs dashboard-ready analytical products through the dbt-nova CLI. Use when you have terminal access to `dbt-nova` and need to turn canonical indicators and execution models into dashboard specs, metric cards, dataset contracts, and viz QA outputs."
license: MIT
allowed-tools: "Bash Read Write"
metadata:
  owner: "dbt-nova"
  persona: "bi-engineer"
  transport: "cli"
  version: "0.0.3"
---

# CLI BI Engineer Skill (dbt-nova)

## Transport contract

- Always pass `--manifest-path`.
- Reuse one `--storage-instance-id` for the full design session.
- Always use `--json`.
- Prefer `--params-file` for structured tool payloads.
- Run `health check` before substantive work.
- If not ready, run `manifest reload` and retry readiness before trusting search or SQL.

## CLI surface

- `tool call search_indicator`: canonical KPI resolution
- `tool call search_recipes`, `get_recipe`, `run_recipe`: recurring reporting scaffolds
- `tool call get_entity` with `detail: "standard"`: compact semantic contract
- `tool call get_columns`: supported dimensions and filter fields
- `tool call get_sql`: execution logic inspection when needed
- `tool call execute_sql`: bounded validation of filters and dataset shape

## Load these shared references before substantive work

- `../../shared/bi-engineer/references/workflow.md`
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
- `docs/getting-started/cli.md`
- `docs/api/tools.md`
