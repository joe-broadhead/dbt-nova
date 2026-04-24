# Engineer Transport: CLI

Use this reference when you are working through the local `dbt-nova` CLI in the terminal.

## Session contract

- Always pass `--manifest-path`.
- Reuse one `--storage-instance-id` for the whole change session.
- Always use `--json`.
- Prefer `--params-file` when payloads become awkward inline.
- Run `audit nova-meta` when schema YAML or `meta.nova` changes.
- After local compile/build updates the manifest, refresh the Nova view before trusting new discovery results.

## CLI mapping

Use the same engineering flow as the main skill; only the transport changes.

Common mappings:
- discovery: `tool call search`, `find_by_path`, `list_entities`
- repeated fields or semantics: `search_columns`, `column_inventory`, `indicator_inventory`
- contract inspection: `get_entity`, `get_columns`, `get_sql`, `get_context`
- blast radius: `get_impact`, `get_lineage`, `get_column_lineage`, `compare_grains`, `diff_entities`
- quality gates: `get_test_coverage`, `get_metadata_score`, `validate_dag`
- lifecycle: `manifest reload`, `health check`

## CLI guardrails

- Keep one stable manifest and storage instance for the session.
- Prefer `tool call` parity with MCP before inventing custom shell parsing.
- Treat auth failures as execution blockers, not discovery failures.
- Use CLI JSON output as evidence for ship summaries.
