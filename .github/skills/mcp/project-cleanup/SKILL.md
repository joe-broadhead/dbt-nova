---
name: mcp-project-cleanup
description: "Finds and plans cleanup work in dbt projects through Nova MCP tools. Use when detecting overlap, inconsistent naming, repeated semantics, and unclear canonical models, then turning that into a cleanup plan."
license: MIT
allowed-tools: "mcp__nova__search mcp__nova__find_by_path mcp__nova__list_entities mcp__nova__indicator_inventory mcp__nova__column_inventory mcp__nova__search_columns mcp__nova__get_entity mcp__nova__get_columns mcp__nova__get_context mcp__nova__diff_entities mcp__nova__compare_grains mcp__nova__find_entity_overlap mcp__nova__modelling_consistency_report mcp__nova__get_lineage mcp__nova__get_impact mcp__nova__get_column_lineage mcp__nova__get_metadata_score mcp__nova__reload_manifest mcp__nova__health Read"
metadata:
  owner: "dbt-nova"
  persona: "project-cleanup"
  version: "0.0.3"
---

# MCP Project Cleanup Skill (dbt-nova)

## Transport contract

- Run `health` before substantive work.
- If status is not `ready`, run `reload_manifest` and wait for readiness.
- Use MCP for overlap discovery, comparison, and cleanup planning; use the CLI separately if you need repo-local validation commands.

## MCP surface

- `modelling_consistency_report`: broad cleanup baseline
- `find_entity_overlap`, `search`, `find_by_path`, `list_entities`: scope inventory and overlap clustering
- `indicator_inventory`, `column_inventory`, `search_columns`: repeated semantics and repeated columns
- `get_entity`, `get_columns`, `get_context`: contract and repeated-concept inspection
- `compare_grains`, `diff_entities`: side-by-side overlap comparison
- `get_lineage`, `get_impact`, `get_column_lineage`: structural dependency review
- `get_metadata_score`: metadata signal during cleanup prioritization

## Load order

- Read `../../shared/project-cleanup/references/workflow.md` first.
- Load the cleanup references only when the workflow reaches them:
  - `../../shared/project-cleanup/references/overlap-triage.md`
  - `../../shared/project-cleanup/references/column-normalization-rules.md`
  - `../../shared/project-cleanup/references/semantic-duplication-patterns.md`
  - `../../shared/project-cleanup/references/cleanup-prioritization.md`
- Load `references/tool-recipes.md` only when you need exact call shapes.

## Shared asset

- `../../shared/project-cleanup/assets/overlap-audit-template.md`

## References

- `../../shared/project-cleanup/references/workflow.md`
- `../../shared/project-cleanup/references/overlap-triage.md`
- `../../shared/project-cleanup/references/column-normalization-rules.md`
- `../../shared/project-cleanup/references/semantic-duplication-patterns.md`
- `../../shared/project-cleanup/references/cleanup-prioritization.md`
- `../../shared/project-cleanup/assets/overlap-audit-template.md`
- `references/tool-recipes.md`
