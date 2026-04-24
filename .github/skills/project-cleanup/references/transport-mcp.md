# Project Cleanup Transport: MCP

Use this reference when the client exposes `mcp__nova__*` tools directly.

## Session contract

- Use MCP for overlap discovery, comparison, and cleanup prioritization.
- Use `health` when readiness is uncertain.
- Prefer targeted overlap calls before broad project reports.
- MCP does not replace deterministic exported inventories; use CLI helper scripts when you need durable artifacts.

## Practical order

1. `show_metadata` or `health` for manifest identity and readiness when needed.
2. `search`, `find_by_path`, or `list_entities` for baseline scope.
3. `find_entity_overlap` for focused overlap clusters.
4. `modelling_consistency_report` when duplicate indicators, canonical conflicts, or project-wide grain drift matter.
5. `search_indicator`, `indicator_inventory`, `search_columns`, and `column_inventory` for repeated semantics and column families.
6. `get_entity`, `get_context`, and `get_columns` for contract inspection.
7. `compare_grains` and `diff_entities` for cluster comparison.
8. `get_lineage`, `get_impact`, and `get_column_lineage` for downstream risk.
9. `get_metadata_score` when metadata inconsistency affects prioritization.
