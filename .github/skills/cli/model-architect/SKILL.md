---
name: cli-model-architect
description: "Improves dbt project structure through the dbt-nova CLI. Use when you have terminal access to `dbt-nova` and need to choose canonical models, compare grains, assess overlap, and plan semantic or modelling refactors."
license: MIT
allowed-tools: "Bash Read Write"
metadata:
  owner: "dbt-nova"
  persona: "model-architect"
  transport: "cli"
  version: "0.0.3"
---

# CLI Model Architect Skill (dbt-nova)

## Transport contract

- Always pass `--manifest-path`.
- Reuse one `--storage-instance-id` for the architecture session.
- Always use `--json`.
- Prefer `--params-file` for structured payloads.
- Run `health check` before substantive work.
- If not ready, run `manifest reload` and retry readiness before trusting discovery outputs.

## CLI surface

- `tool call search`, `find_by_path`, `list_entities`: candidate discovery
- `tool call get_entity`, `get_columns`, `get_context`: contract and shape inspection
- `tool call diff_entities`: side-by-side comparison
- `tool call get_lineage`, `get_impact`, `get_column_lineage`: dependency and blast-radius review
- `tool call get_metadata_score`: quality signal during canonical selection

## Load these shared references before substantive work

- `../../shared/model-architect/references/workflow.md`
- `../../shared/model-architect/references/canonical-model-selection.md`
- `../../shared/model-architect/references/grain-decision-tree.md`
- `../../shared/model-architect/references/modelling-antipatterns.md`
- `../../shared/model-architect/references/helper-vs-canonical-rules.md`
- `../../shared/model-architect/references/semantic-layer-boundaries.md`

## Shared asset

- `../../shared/model-architect/assets/refactor-plan-template.md`

## References

- `../../shared/model-architect/references/workflow.md`
- `../../shared/model-architect/references/canonical-model-selection.md`
- `../../shared/model-architect/references/grain-decision-tree.md`
- `../../shared/model-architect/references/modelling-antipatterns.md`
- `../../shared/model-architect/references/helper-vs-canonical-rules.md`
- `../../shared/model-architect/references/semantic-layer-boundaries.md`
- `../../shared/model-architect/assets/refactor-plan-template.md`
- `docs/getting-started/cli.md`
- `docs/api/tools.md`
