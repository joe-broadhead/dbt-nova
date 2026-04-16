---
name: cli-kpi-debugger
description: "Investigates KPI discrepancies through the dbt-nova CLI. Use when you have terminal access to `dbt-nova` and need to reproduce a KPI, compare it to alternate sources or time windows, inspect lineage, and document suspected root causes with explicit evidence."
license: MIT
allowed-tools: "Bash Read Write"
metadata:
  owner: "dbt-nova"
  persona: "kpi-debugger"
  transport: "cli"
  version: "0.0.3"
---

# CLI KPI Debugger Skill (dbt-nova)

## Transport contract

- Always pass `--manifest-path`.
- Reuse one `--storage-instance-id` for the investigation session.
- Always use `--json`.
- Prefer `--params-file` for structured tool payloads.
- Run `health check` before substantive work.
- If not ready, run `manifest reload` and retry readiness before trusting discovery or SQL.

## CLI surface

- `tool call search_indicator`: canonical KPI resolution
- `tool call search`: supporting discovery when the KPI name is ambiguous
- `tool call get_entity` with `detail: "standard"`: compact semantic contract
- `tool call get_columns` / `get_sql`: execution and definition checks
- `tool call get_lineage` / `get_column_lineage`: provenance and root-cause checks
- `tool call execute_sql`: bounded KPI reproduction and comparison queries
- `tool call run_recipe`: recurring reconciliations or reference workflows when available

## Load these shared references before substantive work

- `../../shared/kpi-debugger/references/workflow.md`
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
- `docs/getting-started/cli.md`
- `docs/api/tools.md`
