# Meta Authoring Transport: MCP

Use MCP to gather evidence from the currently deployed manifest. MCP is excellent for discovery and verification, but it does not validate uncompiled local YAML edits.

## Session Contract

- Start with `show_metadata` when manifest identity or freshness matters.
- Use MCP before editing to avoid duplicate canonical metadata.
- Use MCP after deployment or manifest refresh to verify search and scoring behavior.
- Do not call or rely on hosted manifest reload from this skill. Refresh belongs to the project build/deploy workflow.

## Practical Order

1. `show_metadata` for project and manifest timestamp.
2. `search_indicator` and `indicator_inventory` for measures and metrics.
3. `search_columns` and `column_inventory` for column semantics.
4. `search`, `get_entity`, `get_context`, and `get_columns` for current resource contracts.
5. `compare_grains`, `find_entity_overlap`, or `modelling_consistency_report` for canonical placement questions.
6. `get_metadata_score` and `get_undocumented` for review and remediation evidence.

## Evidence Standard

Capture only the evidence needed to justify the metadata decision:
- current canonical owner
- competing duplicate definitions
- grain and referenced fields
- missing governance or semantic fields
- expected search term and whether the preferred result surfaces
