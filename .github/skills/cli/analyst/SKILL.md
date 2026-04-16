---
name: cli-analyst
description: "Answers business questions through the dbt-nova CLI. Use when you have terminal access to `dbt-nova` but not direct MCP bindings. Optimized for recipe-first recurring workflows, canonical indicator discovery, compact semantic contract checks, bounded SQL execution, and evidence-first reporting."
license: MIT
allowed-tools: "Bash Read Write"
metadata:
  owner: "dbt-nova"
  persona: "analyst"
  transport: "cli"
  version: "0.0.3"
---

# CLI Analyst Skill (dbt-nova)

## Transport contract

- Always pass `--manifest-path`.
- Reuse one `--storage-instance-id` for the whole analysis session.
- Always use `--json`.
- Prefer `--params-file` over long inline JSON.
- Run `health check` before substantive work.
- If not ready, run `manifest reload` and retry readiness before trusting search or SQL.
- `execute_sql` and `run_recipe` require valid warehouse environment variables.

## CLI surface

- Use `tool call search_indicator` as the primary KPI resolver.
- Use `tool call search_recipes`, `get_recipe`, and `run_recipe` for recurring workflows.
- Use `tool call search` as supporting discovery, not the only KPI resolver.
- Use `tool call get_entity` with `detail: "standard"` as the compact semantic contract.
- Use `tool call get_columns`, `get_sql`, and `execute_sql` only after the execution entity is chosen.

## Load these shared references before substantive work

- `../../shared/analyst/references/workflow.md`
- `../../shared/analyst/assets/evidence-block.md`
- `../../shared/analyst/assets/report-template.md`

## References

- `../../shared/analyst/references/workflow.md`
- `../../shared/analyst/assets/evidence-block.md`
- `../../shared/analyst/assets/report-template.md`
- `docs/getting-started/cli.md`
- `docs/api/tools.md`
- `docs/features/recipes.md`
