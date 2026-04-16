---
name: mcp-meta-authoring
description: "Builds and reviews high-signal `meta.nova` through Nova MCP tools. Use when choosing canonical datasets, measures, metrics, grain, search hints, and semantic disambiguation, then validating that those choices surface correctly through search and entity contracts."
license: MIT
allowed-tools: "mcp__nova__search mcp__nova__search_indicator mcp__nova__indicator_inventory mcp__nova__search_columns mcp__nova__column_inventory mcp__nova__get_entity mcp__nova__get_context mcp__nova__get_columns mcp__nova__get_sql mcp__nova__get_lineage mcp__nova__get_metadata_score mcp__nova__find_by_path mcp__nova__compare_grains mcp__nova__find_entity_overlap mcp__nova__reload_manifest mcp__nova__health Read"
metadata:
  owner: "dbt-nova"
  persona: "authoring"
  version: "0.0.3"
---

# MCP Metadata Authoring

## Transport contract

- MCP currently does not expose the CLI-only `audit nova-meta` validator.
- Use MCP to validate search behavior and compact contracts after metadata changes.
- Use the CLI validator separately when you need schema and local semantic validation against `schemas/nova/v0.json`.

## MCP surface

- `indicator_inventory`: inspect repeated measures and metrics before adding another definition
- `search_columns` / `column_inventory`: inspect existing column semantics before annotating columns
- `find_entity_overlap` / `compare_grains`: confirm canonical placement when repeated concepts span multiple entities
- `search_indicator`: measure / metric resolution checks
- `search`: broader entity discovery checks
- `get_entity` with `detail: "standard"`: compact semantic contract
- `get_columns` / `get_metadata_score`: field and quality verification
- `reload_manifest` / `health`: post-build refresh and readiness

## Load order

- Read `../../shared/meta-authoring/references/workflow.md` first.
- Load the deeper authoring references only when the workflow reaches them:
  - `../../shared/meta-authoring/references/decision-rules.md`
  - `../../shared/meta-authoring/references/patterns.md`
  - `../../shared/meta-authoring/references/review-checklist.md`
- Load `references/tool-recipes.md` only when you need exact call shapes.

## References

- `../../shared/meta-authoring/references/workflow.md`
- `../../shared/meta-authoring/references/decision-rules.md`
- `../../shared/meta-authoring/references/patterns.md`
- `../../shared/meta-authoring/references/review-checklist.md`
- `references/tool-recipes.md`
- docs site: `https://joe-broadhead.github.io/dbt-nova/`
- overview: `https://joe-broadhead.github.io/dbt-nova/features/nova-meta-overview/`
- models: `https://joe-broadhead.github.io/dbt-nova/features/nova-meta-models/`
- metrics and measures: `https://joe-broadhead.github.io/dbt-nova/features/nova-meta-metrics/`
- search ranking: `https://joe-broadhead.github.io/dbt-nova/features/search-ranking/`
- persona summaries: `https://joe-broadhead.github.io/dbt-nova/features/persona-summaries/`
