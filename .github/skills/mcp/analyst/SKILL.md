---
name: mcp-analyst
description: "Answers business questions through Nova MCP tools. Use when resolving KPIs, validating canonical indicators, choosing the right execution entity, running deterministic recipes, or executing bounded warehouse SQL with explicit evidence."
license: MIT
allowed-tools: "mcp__nova__search mcp__nova__search_indicator mcp__nova__search_recipes mcp__nova__get_recipe mcp__nova__run_recipe mcp__nova__get_entity mcp__nova__get_columns mcp__nova__get_sql mcp__nova__get_lineage mcp__nova__get_column_lineage mcp__nova__get_context mcp__nova__get_test_coverage mcp__nova__get_metadata_score mcp__nova__find_by_path mcp__nova__execute_sql mcp__nova__health mcp__nova__reload_manifest Read"
metadata:
  owner: "dbt-nova"
  persona: "analyst"
  version: "0.0.3"
---

# MCP Analyst Skill (dbt-nova)

## Transport contract

- Run `health` before substantive work.
- If status is not `ready`, run `reload_manifest` and wait for readiness.
- `run_recipe` uses the same warehouse execution path as `execute_sql`.
- A warehouse auth or provider failure is an execution blocker, not a discovery failure.

## MCP surface

- `search_indicator`: primary KPI resolver
- `search_recipes` / `get_recipe` / `run_recipe`: recurring workflows
- `search`: supporting discovery and entity confirmation
- `get_entity` / `get_columns`: compact contract plus field verification
- `get_sql`: model SQL inspection
- `execute_sql`: bounded validation and final execution
- `get_context`, `get_lineage`, `get_column_lineage`, `get_test_coverage`, and `get_metadata_score`: optional trust and provenance checks

## Load these shared references before substantive work

- `../../shared/analyst/references/workflow.md`
- `../../shared/analyst/assets/report-template.md`

## References

- `../../shared/analyst/references/workflow.md`
- `../../shared/analyst/assets/report-template.md`
- `references/tool-recipes.md`
