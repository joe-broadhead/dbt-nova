---
name: cli-engineer
description: "Builds and modifies dbt models through the dbt-nova CLI. Use when you have terminal access to `dbt-nova` but not direct MCP tool bindings. Supports target discovery, impact analysis, SQL inspection, metadata/test gates, DAG validation, Nova-meta validation, and manifest lifecycle commands."
license: MIT
allowed-tools: "Bash Read Write"
metadata:
  owner: "dbt-nova"
  persona: "engineer"
  transport: "cli"
  version: "0.0.3"
---

# CLI Engineer Skill (dbt-nova)

## Transport contract

- Always pass `--manifest-path`.
- Reuse one `--storage-instance-id` for the whole change session.
- Always use `--json`.
- Prefer `--params-file` for structured tool payloads.
- Reload the manifest after dbt compile/build before trusting fresh search results.
- Use `audit nova-meta` whenever schema YAML or `meta.nova` changes.

## CLI surface

- Use `tool call search`, `find_by_path`, and `list_entities` for target discovery.
- Use `tool call search_columns`, `column_inventory`, and `indicator_inventory` when the change touches existing semantic contracts or repeated columns.
- Use `tool call get_entity`, `get_columns`, `get_sql`, and `get_context` for input validation and triage.
- Use `tool call get_lineage`, `get_impact`, `get_column_lineage`, and `compare_grains` for blast-radius and grain analysis.
- Use `tool call get_test_coverage`, `get_metadata_score`, `get_undocumented`, and `validate_dag` for quality gates.
- Use `manifest reload` and `health check` after dbt compile/build updates the manifest.

## Load order

- Read `../../shared/engineer/references/workflow.md` first.
- Load `../../shared/common/references/manifest-refresh.md` when you reach reload/readiness work.
- Load `../../shared/engineer/assets/ship-checklist.md` only when preparing the final ship summary.

## References

- `../../shared/engineer/references/workflow.md`
- `../../shared/common/references/manifest-refresh.md`
- `../../shared/engineer/assets/ship-checklist.md`
- `docs/getting-started/cli.md`
- `docs/features/metadata-audit.md`
- `docs/features/metadata-scoring.md`
