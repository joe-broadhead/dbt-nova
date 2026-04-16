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

## Mission

Ship production-safe dbt changes with explicit blast-radius, quality, and readiness checks using the `dbt-nova` CLI.

## Session contract

- Always pass `--manifest-path`.
- Reuse one `--storage-instance-id` for the whole change session.
- Always use `--json`.
- Prefer `--params-file` for structured tool payloads.
- Reload the manifest after dbt compile/build before trusting fresh search results.

## Required workflow

1. Preflight
- Run:
  - `dbt-nova health check --manifest-path /path/to/manifest.json --json`
- If not ready, reload first:
  - `dbt-nova manifest reload --manifest-path /path/to/manifest.json --json`

2. Discover the implementation target
- Prefer:
  - `tool call search` with `persona: "engineer"`
  - `tool call find_by_path` when you already know the file area
  - `tool call list_entities` for scoped inventory
- Prefer reuse or extension before adding new models.

3. Validate upstream inputs
- Use:
  - `tool call get_entity`
  - `tool call get_columns`
  - `tool call get_sql`
  - `tool call get_context` with `context_mode: "engineer"` for fast triage
- Default `get_sql` mode:
  - `compiled=false`
- Use `compiled=true` only when you need rendered SQL and the manifest actually contains it.

4. Run blast-radius analysis
- Use:
  - `tool call get_lineage`
  - `tool call get_impact`
  - `tool call get_column_lineage` for critical fields
- Filter lineage to models only when tests would add noise.

5. Run quality gates
- Use:
  - `tool call get_test_coverage`
  - `tool call get_metadata_score`
  - `tool call get_undocumented`
  - `tool call validate_dag`
- Use `validate_dag` with `detail: "summary"` unless you are actively debugging graph defects.

6. Validate Nova metadata when YAML changes
- Run `audit nova-meta` whenever schema YAML or `meta.nova` changes.
- Start with the narrowest target:
  - one file
  - one resource
  - one column
- Widen to project scope only after the narrow target is clean.

7. Refresh the manifest after implementation
- Re-run:
  - `dbt-nova manifest reload --manifest-path /path/to/manifest.json --json`
  - `dbt-nova health check --manifest-path /path/to/manifest.json --json`

8. Report the ship checklist
- Include:
  - target model and grain
  - changed columns or logic
  - downstream impact
  - test gaps
  - metadata score
  - manifest readiness

## Command patterns

### Search for the implementation target

```bash
dbt-nova tool call search \
  --manifest-path /path/to/manifest.json \
  --storage-instance-id engineer-session \
  --params-file search.json \
  --json
```

Example `search.json`:

```json
{"query":"customer lifetime value","persona":"engineer","resource_types":["model"],"limit":10}
```

### Fast triage bundle

```bash
dbt-nova tool call get_context \
  --manifest-path /path/to/manifest.json \
  --storage-instance-id engineer-session \
  --params-file context.json \
  --json
```

Example `context.json`:

```json
{
  "id_or_name":"fct_orders",
  "context_mode":"engineer",
  "include_columns":true,
  "include_upstream":true,
  "include_downstream":false,
  "include_tests":true,
  "include_docs":false,
  "lineage_depth":1
}
```

### Validate Nova metadata on one file

```bash
dbt-nova audit nova-meta \
  --project-dir /path/to/dbt/project \
  --path models/marts/orders.yml \
  --json
```

## Guardrails

- Keep the manifest path and storage instance stable through the session.
- Use raw SQL inspection first; compiled SQL is optional manifest data.
- Treat `ready_for_traffic=false` as a blocker for trusting semantic search results.
- Do not skip blast-radius analysis for grain changes or renamed columns.
- Treat test coverage and metadata score as release gates, not decoration.
- Use `execute_sql` only when you need live data validation and warehouse auth is available.

## References

- `docs/getting-started/cli.md`
- `docs/features/metadata-audit.md`
- `docs/features/metadata-scoring.md`
