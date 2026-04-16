---
name: cli-nova-meta-authoring
description: "Builds and reviews high-signal `meta.nova` blocks through the dbt-nova CLI. Use when authoring or reviewing Nova metadata from the terminal, especially with `audit nova-meta`, targeted validation modes, and post-change search/contract checks."
license: MIT
allowed-tools: "Bash Read Write"
metadata:
  owner: "dbt-nova"
  persona: "authoring"
  transport: "cli"
  version: "0.0.3"
---

# CLI Nova Meta Authoring

## Transport contract

- Use `audit nova-meta` as the first validation step.
- Start with the narrowest possible target.
- Validate against the real schema contract:
  - `schemas/nova/v0.json`
- After YAML changes are compiled into the manifest, confirm behavior through `tool call` commands, not only schema validation.

## CLI surface

- `audit nova-meta`: schema and local semantic validation
- `tool call search_indicator`: measure / metric resolution checks
- `tool call search`: broader entity discovery checks
- `tool call get_entity` with `detail: "standard"`: compact semantic contract
- `tool call get_columns` / `get_metadata_score`: field and quality verification
- `manifest reload` / `health check`: post-build refresh and readiness

## Load these shared references before substantive work

- `../../shared/nova-meta-authoring/references/workflow.md`
- `../../shared/nova-meta-authoring/references/decision-rules.md`
- `../../shared/nova-meta-authoring/references/patterns.md`
- `../../shared/nova-meta-authoring/references/review-checklist.md`

## References

- `../../shared/nova-meta-authoring/references/workflow.md`
- `../../shared/nova-meta-authoring/references/decision-rules.md`
- `../../shared/nova-meta-authoring/references/patterns.md`
- `../../shared/nova-meta-authoring/references/review-checklist.md`
- `docs/getting-started/cli.md`
- `docs/features/nova-meta-overview.md`
- `docs/features/nova-meta-models.md`
- `docs/features/nova-meta-metrics.md`
