# Meta Authoring Transport: CLI

Use the local `dbt-nova` CLI when editing files or validating a local manifest. The CLI is the validation path; MCP is the deployed search/contract path.

## Session Contract

- Run the narrowest available `audit nova-meta` target first.
- Validate the real schema contract before widening scope.
- Compile/build the project manifest after YAML changes.
- Use local `tool call` commands against the rebuilt manifest for pre-deploy verification when MCP still points at an older deployed manifest.

## CLI Mapping

- schema and semantic validation: `audit nova-meta`
- repeated-concept discovery: `tool call search_indicator`, `indicator_inventory`, `search_columns`, `column_inventory`
- contract checks: `tool call get_entity`, `get_context`, `get_columns`
- canonical placement checks: `tool call compare_grains`, `find_entity_overlap`, `modelling_consistency_report`
- post-build verification: `tool call search_indicator`, `search`, `get_metadata_score`
