---
name: mcp-engineer
description: "Builds and modifies dbt models through Nova MCP tools with production-quality gates. Use when changing model SQL, grain, columns, or semantic contracts and you need impact analysis, SQL inspection, metadata/test gates, and manifest readiness checks."
license: MIT
allowed-tools: "mcp__nova__search mcp__nova__find_by_path mcp__nova__get_entity mcp__nova__get_columns mcp__nova__get_sql mcp__nova__get_lineage mcp__nova__get_column_lineage mcp__nova__get_context mcp__nova__get_impact mcp__nova__get_test_coverage mcp__nova__get_metadata_score mcp__nova__get_undocumented mcp__nova__reload_manifest mcp__nova__health mcp__nova__diff_entities mcp__nova__validate_dag Read"
metadata:
  owner: "dbt-nova"
  persona: "engineer"
  version: "0.0.3"
---

# MCP Engineer Skill (dbt-nova)

## Transport contract

- Run `health` before substantive work.
- If status is not `ready`, run `reload_manifest` and wait for readiness.
- MCP does not expose `audit nova-meta`; use the CLI separately when schema/YAML validation is required.

## MCP surface

- `search` / `find_by_path`: target discovery
- `get_entity` / `get_columns` / `get_sql`: contract and SQL inspection
- `get_context`: fast triage bundle
- `get_lineage` / `get_impact` / `get_column_lineage`: blast radius
- `get_test_coverage` / `get_metadata_score` / `get_undocumented`: quality gates
- `diff_entities` / `validate_dag`: change verification
- `reload_manifest` / `health`: lifecycle and readiness

## Load these shared references before substantive work

- `../../shared/engineer/references/workflow.md`
- `../../shared/common/references/manifest-refresh.md`
- `../../shared/engineer/assets/ship-checklist.md`

## References

- `../../shared/engineer/references/workflow.md`
- `../../shared/common/references/manifest-refresh.md`
- `../../shared/engineer/assets/ship-checklist.md`
- `references/tool-recipes.md`
