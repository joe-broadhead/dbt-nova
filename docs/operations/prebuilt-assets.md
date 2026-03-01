# Prebuilt Asset Workflow

Use this workflow when you want to build Nova storage assets once in CI and
reuse them across jobs/repos in read-only mode.

## What this solves

- Removes repeated index builds on consumer jobs.
- Makes consumer startup deterministic.
- Enforces a strict contract (`storage_instance_id` + manifest content hash).

## v1 boundaries

- Producer workflow + GitHub Artifacts are supported.
- Optional models cache artifact is supported via `include_models_cache`.
- Consumers are read-only (`DBT_NOVA_STORAGE_READ_ONLY=true`) and do **not**
  fall back to rebuilding.
- Optional S3/GCS/DBFS publish targets are out of scope for this version.

## Producer (build once)

Create a workflow in the downstream repo that calls Nova's reusable producer.

```yaml
name: Build Nova Assets

on:
  workflow_dispatch:

jobs:
  build_nova_assets:
    uses: joe-broadhead/dbt-nova/.github/workflows/nova-build-assets.yml@v0.0.2
    with:
      manifest_path: target/manifest.json
      storage_instance_id: analytics-prod
      artifact_name_prefix: analytics-prod
      retention_days: 14
      include_models_cache: false
```

Alternative producer inputs:

- `manifest_uri` (instead of `manifest_path`)
- `dbt_generate_manifest: true` + `dbt_command` (build manifest in workflow)

The producer emits:

- storage artifact (required)
- metadata contract artifact (`nova-build-metadata.json`, required)
- models artifact (optional when `include_models_cache=true`)

## Consumer (reuse in read-only mode)

1. Download and extract the storage artifact.
2. Use the **same** `storage_instance_id` used by the producer.
3. Set read-only env vars before starting Nova.

Required env vars:

- `DBT_NOVA_STORAGE_DIR`
- `DBT_NOVA_STORAGE_INSTANCE_ID`
- `DBT_NOVA_STORAGE_READ_ONLY=true`

Example (CI shell step):

```bash
export DBT_NOVA_STORAGE_DIR="$PWD/dbt-nova-storage"
export DBT_NOVA_STORAGE_INSTANCE_ID="analytics-prod"
export DBT_NOVA_STORAGE_READ_ONLY="true"

# If your consumer uses a local manifest copy:
export DBT_MANIFEST_PATH="$PWD/manifest.json"

dbt-nova health check --manifest-path "$DBT_MANIFEST_PATH" --json
```

If you publish/download the optional models artifact, also set:

- `DBT_NOVA_EMBEDDINGS_CACHE_DIR` to the extracted models directory.

## MCP client env example (read-only consumer)

```json
{
  "mcpServers": {
    "dbt-nova": {
      "command": "/path/to/dbt-nova",
      "env": {
        "DBT_MANIFEST_PATH": "/path/to/manifest.json",
        "DBT_NOVA_STORAGE_DIR": "/path/to/dbt-nova-storage",
        "DBT_NOVA_STORAGE_INSTANCE_ID": "analytics-prod",
        "DBT_NOVA_STORAGE_READ_ONLY": "true"
      }
    }
  }
}
```

## Compatibility guidance

- Keep producer and consumer on the same released Nova version when possible.
- `storage_instance_id` must match between producer and consumer.
- Consumer manifest content must match the producer-built manifest hash.
  Path differences are allowed; content differences are not.
- Metadata contract version must be compatible (`v1` currently).

## Failure modes and fixes

| Symptom | Likely cause | Fix |
| --- | --- | --- |
| `Storage is read-only and no reusable index is available` | Missing storage files, mismatched `storage_instance_id`, or manifest content mismatch | Re-download artifacts, verify instance id, verify manifest is identical to producer input |
| Metadata contract validation fails | Missing/corrupt `nova-build-metadata.json` or unsupported contract version | Re-run producer and consume both storage + metadata artifacts together |
| Health passes but embeddings are missing | Models artifact was not downloaded in slim/read-only flow | Download models artifact (if produced) and set `DBT_NOVA_EMBEDDINGS_CACHE_DIR` |

## Related docs

- [MCP Client Configs](../getting-started/mcp-clients.md)
- [Troubleshooting](troubleshooting.md)
- [CI & Automation](../development/ci.md)
