# Model Architect Transport: MCP

Use this reference when the client exposes `mcp__nova__*` tools directly.

## Session contract

- Use MCP for candidate discovery, overlap comparison, and impact analysis.
- Use `health` when readiness is uncertain.
- Prefer targeted MCP calls before broad reports because project-wide consistency checks can be slower and noisier.
- MCP does not replace deterministic exported inventories; use CLI helper scripts if you need durable artifacts.

## Practical order

1. `show_metadata` or `health` for manifest identity and readiness when needed.
2. `search`, `find_by_path`, or `list_entities` for baseline scope.
3. `search_indicator`, `indicator_inventory`, `search_columns`, and `column_inventory` for repeated-concept discovery.
4. `find_entity_overlap` for a focused candidate cluster.
5. `get_entity`, `get_context`, `get_sql`, and `get_columns` for contract inspection.
6. `compare_grains` and `diff_entities` for canonical-candidate decisions.
7. `get_lineage`, `get_impact`, and `get_column_lineage` for blast-radius review.
8. `get_metadata_score` when metadata quality should influence the canonical choice.
9. `modelling_consistency_report` when you need project-wide duplicate indicators, canonical conflicts, or grain drift.
