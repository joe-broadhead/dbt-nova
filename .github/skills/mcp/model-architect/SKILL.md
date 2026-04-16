---
name: mcp-model-architect
description: "Improves dbt project structure through Nova MCP tools. Use when choosing canonical execution models, comparing grains, assessing overlap, and planning semantic or modelling refactors."
license: MIT
allowed-tools: "mcp__nova__search mcp__nova__find_by_path mcp__nova__list_entities mcp__nova__get_entity mcp__nova__get_columns mcp__nova__get_context mcp__nova__diff_entities mcp__nova__get_lineage mcp__nova__get_impact mcp__nova__get_column_lineage mcp__nova__get_metadata_score mcp__nova__reload_manifest mcp__nova__health Read"
metadata:
  owner: "dbt-nova"
  persona: "model-architect"
  version: "0.0.3"
---

# MCP Model Architect Skill (dbt-nova)

## Transport contract

- Run `health` before substantive work.
- If status is not `ready`, run `reload_manifest` and wait for readiness.
- Use MCP for project-shape discovery, comparison, and planning; use the CLI separately if you need repo-local validation commands.

## MCP surface

- `search`, `find_by_path`, `list_entities`: candidate discovery
- `get_entity`, `get_columns`, `get_context`: contract and shape inspection
- `diff_entities`: side-by-side comparison
- `get_lineage`, `get_impact`, `get_column_lineage`: dependency and blast-radius review
- `get_metadata_score`: quality signal during canonical selection

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
