# Model Architect Transport: CLI

Use this reference when you are working through the local `dbt-nova` CLI.

## Session contract

- Always pass `--manifest-path`.
- Reuse one `--storage-instance-id` for the architecture session.
- Always use `--json`.
- Prefer helper scripts when you need deterministic exported artifacts.
- Use local CLI output for source-controlled evidence; use MCP for the currently deployed endpoint.

## CLI mapping

Common mappings:
- baseline discovery: `tool call search`, `find_by_path`, `list_entities`
- repeated-concept discovery: `search_indicator`, `indicator_inventory`, `search_columns`, `column_inventory`
- overlap and consistency: `find_entity_overlap`,
  `modelling_consistency_report` with agent-modelling findings
- contract checks: `get_entity`, `get_context`, `get_sql`, `get_columns`
- side-by-side comparisons: `compare_grains`, `diff_entities`
- impact checks: `get_lineage`, `get_impact`, `get_column_lineage`
- deterministic exports: `python3 scripts/export_entity_inventory.py`, `export_column_inventory.py`, `build_overlap_report.py`
