---
name: cli-meta-authoring
description: "Builds and reviews high-signal `meta.nova` blocks through the dbt-nova CLI. Use when authoring or reviewing metadata from the terminal, especially with `audit nova-meta`, targeted validation modes, and post-change search/contract checks."
license: MIT
allowed-tools: "Bash Read Write"
metadata:
  owner: "dbt-nova"
  persona: "authoring"
  transport: "cli"
  version: "0.0.3"
---

# CLI Metadata Authoring

## Transport contract

- Use `audit nova-meta` as the first validation step.
- Start with the narrowest possible target.
- Validate against the real schema contract:
  - `schemas/nova/v0.json`
- After YAML changes are compiled into the manifest, confirm behavior through `tool call` commands, not only schema validation.

## CLI surface

- `audit nova-meta`: schema and local semantic validation
- `tool call indicator_inventory`: inspect repeated measures and metrics before adding another definition
- `tool call search_columns` / `column_inventory`: inspect existing column semantics before annotating columns
- `tool call find_entity_overlap` / `compare_grains`: confirm canonical placement when repeated concepts span multiple entities
- `tool call search_indicator`: measure / metric resolution checks
- `tool call search`: broader entity discovery checks
- `tool call get_entity` with `detail: "standard"`: compact semantic contract
- `tool call get_columns` / `get_metadata_score`: field and quality verification
- `manifest reload` / `health check`: post-build refresh and readiness

## Load order

- Read `../../shared/meta-authoring/references/workflow.md` first.
- Load the deeper authoring references only when the workflow reaches them:
  - `../../shared/meta-authoring/references/decision-rules.md`
  - `../../shared/meta-authoring/references/patterns.md`
  - `../../shared/meta-authoring/references/review-checklist.md`

## References

- `../../shared/meta-authoring/references/workflow.md`
- `../../shared/meta-authoring/references/decision-rules.md`
- `../../shared/meta-authoring/references/patterns.md`
- `../../shared/meta-authoring/references/review-checklist.md`
- `docs/getting-started/cli.md`
- `docs/features/nova-meta-overview.md`
- `docs/features/nova-meta-models.md`
- `docs/features/nova-meta-metrics.md`
