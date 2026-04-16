---
name: cli-governance
description: "Runs deterministic governance audits through the dbt-nova CLI. Use when you need reproducible metadata scoring, documentation/test gap detection, Nova metadata validation, blocker classification, and rerunnable audit evidence from the terminal."
license: MIT
allowed-tools: "Bash Read Write"
metadata:
  owner: "dbt-nova"
  persona: "governance"
  transport: "cli"
  version: "0.0.3"
---

# CLI Governance Skill (dbt-nova)

## Transport contract

- Always pass `--manifest-path`.
- Reuse one `--storage-instance-id` for the whole audit session.
- Always use `--json`.
- Freeze scope before scoring and keep it stable for reruns.
- Run `health check` before substantive work.
- If not ready, run `manifest reload` and retry readiness before scoring.
- Use `audit metadata-score` as the primary gate.
- Use `audit nova-meta` when schema YAML or Nova metadata is in scope.

## CLI surface

- `audit metadata-score`: primary governance gate
- `audit nova-meta`: schema and local semantic validation for Nova metadata
- `tool call get_metadata_score`, `get_undocumented`, `get_test_coverage`, and `get_entity`: blocker detail
- `tool call search` with `persona: "governance"`: triage support only

## Load these shared references before substantive work

- `../../shared/governance/references/workflow.md`
- `../../shared/governance/references/metadata-rubric.md`
- `../../shared/common/references/manifest-refresh.md`
- `../../shared/governance/assets/governance-audit-template.md`

## References

- `../../shared/governance/references/workflow.md`
- `../../shared/governance/references/metadata-rubric.md`
- `../../shared/common/references/manifest-refresh.md`
- `../../shared/governance/assets/governance-audit-template.md`
- `docs/getting-started/cli.md`
- `docs/features/metadata-audit.md`
- `docs/features/metadata-scoring.md`
