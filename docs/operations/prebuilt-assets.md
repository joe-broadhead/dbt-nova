# Prebuilt Asset Workflow

Use this workflow when you want to build Nova storage assets once in CI and
reuse them across jobs/repos in read-only mode.

## What this solves

- Removes repeated index builds on consumer jobs.
- Makes consumer startup deterministic.
- Enforces a strict contract (`storage_instance_id` + manifest content hash).

## v1 boundaries

- Producer workflow + GitHub Artifacts are supported.
- Optional models distribution is controlled by `models_distribution_mode`.
- Consumers are read-only (`DBT_NOVA_STORAGE_READ_ONLY=true`) and do **not**
  fall back to rebuilding.
- Optional S3/GCS/DBFS publish targets are supported and disabled by default.

## Producer (build once)

Create a workflow in the downstream repo that calls Nova's reusable producer.

```yaml
name: Build Nova Assets

on:
  workflow_dispatch:

jobs:
  build_nova_assets:
    uses: joe-broadhead/dbt-nova/.github/workflows/nova-build-assets.yml@master
    with:
      manifest_path: target/manifest.json
      storage_instance_id: analytics-prod
      installer_ref: v0.0.2
      installer_install_mode: auto
      artifact_name_prefix: analytics-prod
      retention_days: 14
      models_distribution_mode: none
      publish_targets: ""
```

Alternative producer inputs:

- `manifest_uri` (instead of `manifest_path`)
- `dbt_generate_manifest: true` + `dbt_command` (build manifest in workflow)
- `dbt_env_json` (JSON object of non-secret env vars exported before `dbt_command`)
- `dbt_secret_env_map_json` (JSON object mapping env var names to secret names)
- `models_distribution_mode` (`none`, `publish_only`, `publish_and_bootstrap`)
- workflow_call secret `DBT_NOVA_SECRET_BUNDLE_JSON` (optional JSON object of
  `secret-name -> secret-value` entries for cross-owner reusable workflow calls)

`dbt_command` is executed as `bash -lc "<command>"` inside the caller
repository checkout. Treat it as a trusted command surface and only use this
mode in repositories/branches where workflow callers are trusted.

When using `dbt_generate_manifest: true`, prefer this generic pattern so the
workflow works across Databricks, BigQuery, DuckDB, and mixed profiles:

```yaml
jobs:
  build_nova_assets:
    uses: joe-broadhead/dbt-nova/.github/workflows/nova-build-assets.yml@master
    with:
      dbt_generate_manifest: true
      dbt_command: |
        set -euo pipefail
        dbt deps
        dbt compile -t "$DBT_TARGET"
      dbt_env_json: >-
        {"DBT_TARGET":"prod","DBT_PROFILES_DIR":"./"}
      dbt_secret_env_map_json: >-
        {"DBT_ACCESS_TOKEN":"DBT_ACCESS_TOKEN","DBT_BIGQUERY_KEYFILE_JSON":"DBT_BIGQUERY_KEYFILE_JSON"}
      storage_instance_id: analytics-prod
    secrets:
      DBT_NOVA_SECRET_BUNDLE_JSON: ${{ secrets.DBT_NOVA_SECRET_BUNDLE_JSON }}
```

Notes:

- `dbt_env_json` values are plain strings.
- `dbt_secret_env_map_json` values are looked up in this order:
  1) keys in `DBT_NOVA_SECRET_BUNDLE_JSON` (when provided),
  2) inherited workflow secrets (`secrets: inherit`, same-owner/org calls).
- Missing mapped secrets fail fast before `dbt_command` runs.

Common downstream pattern: keep a repo-local `workflow_dispatch` wrapper and
call this reusable workflow from it. This lets each repo set its own target
defaults, storage instance naming, and publish prefixes without forking Nova's
workflow.

The producer emits:

- storage artifact (required)
- manifest artifact (`manifest.json`, always exported)
- metadata contract artifact (`nova-build-metadata.json`, required)
- models artifact (optional when `models_distribution_mode != none`)
- bootstrap contract artifact(s) when remote publish is configured
- publish summary artifact (`artifact_name_publish_summary`) with published URIs by target

Models distribution modes:

- `none`: do not package or publish models artifact; bootstrap omits `models_artifact_uri`.
- `publish_only`: package/publish models artifact, but bootstrap still omits `models_artifact_uri`.
- `publish_and_bootstrap`: package/publish models artifact and include `models_artifact_uri` in bootstrap.

Compatibility:

- `include_models_cache` is still accepted for legacy callers.
- When `models_distribution_mode` is set, it takes precedence.
- When `models_distribution_mode` is empty, workflow derives behavior from `include_models_cache`:
  `true -> publish_and_bootstrap`, `false -> none`.

## Optional remote publish targets

Use this when consumers should pull artifacts from cloud storage instead of
GitHub Actions artifacts.

Workflow inputs:

- `publish_targets`: comma-separated list from `s3,gcs,dbfs`
- `publish_s3_prefix`: e.g. `s3://my-bucket/nova-assets/prod`
- `publish_gcs_prefix`: e.g. `gs://my-bucket/nova-assets/prod`
- `publish_dbfs_prefix`: e.g. `dbfs:/FileStore/projects/my-project/nova-assets/prod`
- `publish_dry_run`: `true` to compute publish URIs without network uploads
- `installer_repository` / `installer_ref` (advanced override for which repo/ref
  is used for installing `dbt-nova`; defaults to
  `joe-broadhead/dbt-nova` plus the resolved reusable-workflow ref when available)
- `installer_install_mode`: `auto` (default; try release binary then fall back to source build),
  `release` (release binary only), or `source` (always build from source)

Auth per target:

- `s3`: standard AWS env credentials used by `aws` CLI
- `gcs`: one of `DBT_NOVA_GCP_ACCESS_TOKEN`, `DBT_NOVA_BIGQUERY_ACCESS_TOKEN`,
  `GCP_ACCESS_TOKEN`, `GOOGLE_OAUTH_ACCESS_TOKEN` (or gcloud ADC token)
- `dbfs`: `DATABRICKS_HOST` and `DATABRICKS_ACCESS_TOKEN`

Installer mode guidance:

- Keep `installer_install_mode: auto` as the default.
- Use `installer_install_mode: release` with a release tag ref (for example
  `installer_ref: v0.0.2`) to minimize runtime on compatible runners.
- Use `installer_install_mode: source` when you need an unreleased commit SHA
  or your runner image is incompatible with the prebuilt binary (for example
  older glibc environments).

Published object naming is deterministic:

- storage: `<prefix>/<artifact_name_storage>.tar.gz`
- manifest: `<prefix>/<artifact_name_manifest>.json`
- metadata: `<prefix>/<artifact_name_metadata>.json`
- bootstrap: `<prefix>/<artifact_name_bootstrap>.json`
- models (optional): `<prefix>/<artifact_name_models>.tar.gz`

Producer outputs include:

- `published_targets` (comma-separated successful targets)
- `artifact_name_publish_summary` (artifact containing `published_*_uris` JSON payloads)
- legacy `published_*_uris` outputs are deprecated and return `{}` for compatibility

Example (extract DBFS bootstrap URI from publish summary artifact):

```bash
PUBLISH_SUMMARY_DIR="$(mktemp -d)"
gh run download "$GITHUB_RUN_ID" --name "$ARTIFACT_NAME_PUBLISH_SUMMARY" --dir "$PUBLISH_SUMMARY_DIR"
PUBLISH_SUMMARY_JSON="$(find "$PUBLISH_SUMMARY_DIR" -type f -name '*.json' | head -n 1)"
BOOTSTRAP_URI="$(jq -r '.published_bootstrap_uris.dbfs' "$PUBLISH_SUMMARY_JSON")"
```

## Consumer (reuse in read-only mode)

Nova now supports **native remote artifact consumption**. Manual download/extract is optional.

### Option A (recommended): native remote artifact mode

Set these env vars:

- `DBT_NOVA_STORAGE_READ_ONLY=true`
- `DBT_NOVA_BOOTSTRAP_URI` (recommended one-URI setup)
- `DBT_NOVA_ARTIFACT_FETCH_POLICY=if_missing|always|never` (default `if_missing`)

Optional:

- `DBT_NOVA_STORAGE_INSTANCE_ID`, `DBT_NOVA_STORAGE_ARTIFACT_URI`, `DBT_NOVA_METADATA_ARTIFACT_URI`,
  `DBT_NOVA_MODELS_ARTIFACT_URI` (explicit mode if you do not use bootstrap URI)
- `DBT_NOVA_ARTIFACTS_CACHE_DIR` (defaults to `<storage_root>/artifacts`)
- `DBT_NOVA_ARTIFACT_TIMEOUT_SECS` (default `300`)
- `DBT_NOVA_ARTIFACT_ALLOW_HTTP=true` (only for non-TLS artifact URIs; not recommended)

Local shell example:

```bash
export DBT_MANIFEST_PATH="$PWD/manifest.json"
export DBT_NOVA_STORAGE_DIR="$PWD/.dbt-nova"
export DBT_NOVA_STORAGE_READ_ONLY="true"
export DBT_NOVA_BOOTSTRAP_URI="s3://my-bucket/nova-assets/prod/analytics-prod-bootstrap.json"
export DBT_NOVA_ARTIFACT_FETCH_POLICY="if_missing"

dbt-nova health check --json
```

CI shell example:

```bash
export DBT_NOVA_STORAGE_DIR="$RUNNER_TEMP/.dbt-nova"
export DBT_NOVA_STORAGE_READ_ONLY="true"
export DBT_NOVA_BOOTSTRAP_URI="$BOOTSTRAP_URI"
export DBT_NOVA_ARTIFACT_FETCH_POLICY="if_missing"

dbt-nova health check --json
```

MCP client env example:

```json
{
  "mcpServers": {
    "dbt-nova": {
      "command": "/path/to/dbt-nova",
      "env": {
        "DBT_NOVA_STORAGE_DIR": "/path/to/.dbt-nova",
        "DBT_NOVA_STORAGE_READ_ONLY": "true",
        "DBT_NOVA_BOOTSTRAP_URI": "s3://my-bucket/nova-assets/prod/analytics-prod-bootstrap.json",
        "DBT_NOVA_ARTIFACT_FETCH_POLICY": "if_missing"
      }
    }
  }
}
```

Use `health` to verify runtime decisions:

- `artifact_consumer.enabled`
- `artifact_consumer.fetch_policy`
- `artifact_consumer.metadata_validated`
- `artifact_consumer.storage_materialized`
- `artifact_consumer.models_materialized`
- `artifact_consumer.last_evaluated_at_ms`
- `artifact_consumer.last_materialized_at_ms`
- `bootstrap.enabled`
- `bootstrap.uri`
- `bootstrap.contract_version`
- `bootstrap.loaded`
- `bootstrap.validated`
- `bootstrap.applied_fields`
- `bootstrap.last_evaluated_at_ms`

### Option B: manual extraction fallback

If you prefer manual extraction, keep using pre-extracted artifacts with:

- `DBT_NOVA_STORAGE_DIR`
- `DBT_NOVA_STORAGE_INSTANCE_ID`
- `DBT_NOVA_STORAGE_READ_ONLY=true`

If you extracted models separately, set:

- `DBT_NOVA_EMBEDDINGS_CACHE_DIR` to the extracted models directory.

### Manual retrieval examples

S3:

```bash
aws s3 cp s3://my-bucket/nova-assets/prod/<artifact_name_storage>.tar.gz .
tar -xzf <artifact_name_storage>.tar.gz
```

GCS:

```bash
gcloud storage cp gs://my-bucket/nova-assets/prod/<artifact_name_storage>.tar.gz .
tar -xzf <artifact_name_storage>.tar.gz
```

DBFS (Databricks CLI):

```bash
databricks fs cp dbfs:/FileStore/projects/my-project/nova-assets/prod/<artifact_name_storage>.tar.gz .
tar -xzf <artifact_name_storage>.tar.gz
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
| Health passes but embeddings are missing | Models artifact not provided in consumer setup | Native mode: set `DBT_NOVA_MODELS_ARTIFACT_URI` to the producer models artifact URI. Manual mode: extract models artifact and set `DBT_NOVA_EMBEDDINGS_CACHE_DIR`. |

## Related docs

- [MCP Client Configs](../getting-started/mcp-clients.md)
- [Troubleshooting](troubleshooting.md)
- [CI & Automation](../development/ci.md)
