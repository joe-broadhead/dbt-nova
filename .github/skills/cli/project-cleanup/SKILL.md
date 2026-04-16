---
name: cli-project-cleanup
description: "Finds and plans cleanup work in dbt projects through the dbt-nova CLI. Use when you have terminal access to `dbt-nova` and need to detect overlap, inconsistent naming, repeated semantics, and unclear canonical models, then turn that into a cleanup plan."
license: MIT
allowed-tools: "Bash Read Write"
metadata:
  owner: "dbt-nova"
  persona: "project-cleanup"
  transport: "cli"
  version: "0.0.3"
---

# CLI Project Cleanup Skill (dbt-nova)

## Transport contract

- Always pass `--manifest-path`.
- Reuse one `--storage-instance-id` for the cleanup session.
- Always use `--json`.
- Prefer `--params-file` for structured payloads.
- Run `health check` before substantive work.
- If not ready, run `manifest reload` and retry readiness before trusting discovery outputs.

## CLI surface

- `tool call modelling_consistency_report`: broad cleanup baseline
- `tool call find_entity_overlap`, `search`, `find_by_path`, `list_entities`: scope inventory and overlap clustering
- `tool call indicator_inventory`, `column_inventory`, `search_columns`: repeated semantics and repeated columns
- `tool call get_entity`, `get_columns`, `get_context`: contract and repeated-concept inspection
- `tool call compare_grains`, `diff_entities`: side-by-side overlap comparison
- `tool call get_lineage`, `get_impact`, `get_column_lineage`: structural dependency review
- `tool call get_metadata_score`: metadata signal during cleanup prioritization
- Prefer `scripts/export_entity_inventory.py`, `scripts/export_column_inventory.py`, and `scripts/build_overlap_report.py` when you need deterministic exported artifacts.

## Load order

- Read `../../shared/project-cleanup/references/workflow.md` first.
- Load the cleanup references only when the workflow reaches them:
  - `../../shared/project-cleanup/references/overlap-triage.md`
  - `../../shared/project-cleanup/references/column-normalization-rules.md`
  - `../../shared/project-cleanup/references/semantic-duplication-patterns.md`
  - `../../shared/project-cleanup/references/cleanup-prioritization.md`

## Shared asset

- `../../shared/project-cleanup/assets/overlap-audit-template.md`

## References

- `../../shared/project-cleanup/references/workflow.md`
- `../../shared/project-cleanup/references/overlap-triage.md`
- `../../shared/project-cleanup/references/column-normalization-rules.md`
- `../../shared/project-cleanup/references/semantic-duplication-patterns.md`
- `../../shared/project-cleanup/references/cleanup-prioritization.md`
- `../../shared/project-cleanup/assets/overlap-audit-template.md`
- `docs/getting-started/cli.md`
- `docs/api/tools.md`
