# Governance Transport: CLI

Use this reference when you are working through the local `dbt-nova` CLI.

## Session contract

- Always pass `--manifest-path`.
- Reuse one `--storage-instance-id` for the whole audit session.
- Always use `--json`.
- Freeze scope before scoring and keep it stable for reruns.
- Use `audit metadata-score` as the primary gate.
- Use `audit nova-meta` when schema YAML or Nova metadata is in scope.

## CLI mapping

Use the same governance flow as the main skill; only the transport changes.

Common mappings:
- primary gate: `audit metadata-score`
- local Nova validation: `audit nova-meta`
- blocker detail: `tool call get_metadata_score`, `get_test_coverage`, `get_entity`
- scope inventory: `tool call list_entities`, `find_by_path`
