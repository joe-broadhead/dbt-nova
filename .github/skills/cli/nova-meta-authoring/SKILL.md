---
name: cli-nova-meta-authoring
description: "Builds and reviews high-signal `meta.nova` blocks through the dbt-nova CLI. Use when authoring or reviewing Nova metadata from the terminal, especially with `audit nova-meta`, targeted validation modes, and post-change search/contract checks."
license: MIT
allowed-tools: "Bash Read Write"
metadata:
  owner: "dbt-nova"
  persona: "authoring"
  transport: "cli"
  version: "0.0.3"
---

# CLI Nova Meta Authoring

## Mission

Author high-signal `meta.nova` with fast local validation loops and then confirm the authored semantics through the real CLI search and entity contract surface.

## Session contract

- Use `audit nova-meta` as the first validation step.
- Start with the narrowest possible target.
- Validate against the real schema contract:
  - `schemas/nova/v0.json`
- After YAML changes are compiled into the manifest, confirm behavior through `tool call` commands, not only schema validation.

## Required workflow

1. Classify the change
- Decide whether you are editing:
  - entity-level metadata
  - measures
  - metrics
  - helper-model search hints
  - column-level metadata

2. Validate the smallest possible scope first
- Single file:
  - `dbt-nova audit nova-meta --project-dir /path/to/project --path models/marts/orders.yml --json`
- Single resource:
  - `dbt-nova audit nova-meta --project-dir /path/to/project --resource-kind model --resource-name fct_orders --json`
- Single column:
  - `dbt-nova audit nova-meta --project-dir /path/to/project --resource-kind model --resource-name fct_orders --column order_date --json`

3. Treat schema and semantic failures as blockers
- `audit nova-meta` enforces the `v0` schema and local semantic checks.
- Do not continue to search validation until the narrow target is clean.

4. Rebuild and reload before checking search behavior
- After dbt compile/build updates the manifest:
  - `dbt-nova manifest reload --manifest-path /path/to/manifest.json --json`
  - `dbt-nova health check --manifest-path /path/to/manifest.json --json`

5. Check authored semantics through the real search surface
- Use:
  - `tool call search_indicator` for indicators
  - `tool call search` for broader entity discovery
  - `tool call get_entity` with `detail: "standard"` for the compact semantic contract
  - `tool call get_columns` to confirm referenced fields
  - `tool call get_metadata_score` to confirm documentation and metadata quality

6. Widen scope only after the narrow target is clean
- Re-run on the containing folder or full project before shipping.

## Command patterns

### Validate one file

```bash
dbt-nova audit nova-meta \
  --project-dir /path/to/dbt/project \
  --path models/marts/orders.yml \
  --json
```

### Validate one model

```bash
dbt-nova audit nova-meta \
  --project-dir /path/to/dbt/project \
  --resource-kind model \
  --resource-name fct_orders \
  --json
```

### Check whether a metric resolves correctly

```bash
dbt-nova tool call search_indicator \
  --manifest-path /path/to/manifest.json \
  --storage-instance-id nova-meta-session \
  --params-file indicator.json \
  --json
```

Example `indicator.json`:

```json
{"query":"gmv","persona":"analyst","limit":5}
```

### Confirm the compact contract

```bash
dbt-nova tool call get_entity \
  --manifest-path /path/to/manifest.json \
  --storage-instance-id nova-meta-session \
  --params-file entity.json \
  --json
```

Example `entity.json`:

```json
{"id_or_name":"base__sales_enriched_sql","detail":"standard"}
```

## Guardrails

- Start with the narrowest validator target that matches the edit.
- Use explicit `--path` when you intentionally need to validate inside ignored directories such as `.venv` or `target`.
- Use `search_indicator` to confirm authored measures and metrics actually resolve the way analysts will search for them.
- Use `get_entity detail=standard` to confirm the contract exposed to agents, not just the YAML you wrote.
- Re-run project-wide validation after a series of local fixes.

## References

- `docs/getting-started/cli.md`
- `docs/features/nova-meta-overview.md`
- `docs/features/nova-meta-models.md`
- `docs/features/nova-meta-metrics.md`
