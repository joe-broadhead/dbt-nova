# Prebuilt Asset Workflow

Use this workflow when you want to build Nova storage assets once in CI and
reuse them across jobs/repos with writable first-run hydration or strict
read-only reuse after local materialization.

## Quick setup checklist

1. Pin the reusable workflow to a release tag or commit SHA (do not use `@master` in production).
2. Pick one producer source:
   - existing `manifest.json` (`manifest_path` or `manifest_uri`), or
   - generate manifest in workflow (`dbt_generate_manifest: true`).
3. Choose a stable `storage_instance_id` (same value must be used by consumers).
4. Choose `models_distribution_mode`:
   - `none` for local/pre-warmed models,
   - `publish_only` for optional consumer opt-in,
   - `publish_and_bootstrap` for bootstrap-driven model hydration.
5. If publishing remotely, set `publish_targets` and matching auth secrets.
6. Trigger the workflow and confirm success.
7. Use the produced stable bootstrap alias in consumer env (`DBT_NOVA_BOOTSTRAP_URI`).
8. On first consumer run, keep `DBT_NOVA_STORAGE_READ_ONLY` unset and hydrate locally with `DBT_NOVA_ARTIFACT_FETCH_POLICY=if_missing`.
9. Switch to strict read-only only after local artifacts already exist (`DBT_NOVA_STORAGE_READ_ONLY=true`, `DBT_NOVA_ARTIFACT_FETCH_POLICY=never`).

## What this solves

- Removes repeated index builds on consumer jobs.
- Makes consumer startup deterministic.
- Enforces a strict contract (`storage_instance_id` + manifest content hash).

## v1 boundaries

- Producer workflow + GitHub Artifacts are supported.
- Optional models distribution is controlled by `models_distribution_mode`.
- Consumers can hydrate artifacts locally on first run, then optionally switch
  to strict read-only reuse.
- Strict read-only consumers (`DBT_NOVA_STORAGE_READ_ONLY=true`) do **not**
  fall back to rebuilding or re-materializing missing artifacts.
- Optional S3/GCS/DBFS publish targets are supported and disabled by default.

## Producer (build once)

Create a workflow in the downstream repo that calls Nova's reusable producer.

The examples below pin `v0.0.14`. For production, pin either a release tag or an
immutable commit SHA and keep `installer_ref` aligned with the workflow ref.

```yaml
name: Build Nova Assets

on:
  workflow_dispatch:

jobs:
  build_nova_assets:
    # Pin to a release tag or commit SHA
    uses: joe-broadhead/dbt-nova/.github/workflows/nova-build-assets.yml@v0.0.14
    with:
      manifest_path: target/manifest.json
      storage_instance_id: analytics-prod
      installer_ref: v0.0.14
      installer_install_mode: auto
      artifact_name_prefix: analytics-prod
      retention_days: 14
      models_distribution_mode: none
      publish_targets: ""
```

Alternative producer inputs:

- `manifest_uri` (instead of `manifest_path`)
- `dbt_generate_manifest: true` + structured invocation (`dbt_command_args_json`, optional `dbt_executable`, optional `dbt_allow_unsafe_executable`)
- `dbt_generate_manifest: true` + trusted shell invocation (`dbt_command`)
- `dbt_env_json` (JSON object of non-secret env vars exported before dbt invocation)
- `dbt_secret_env_map_json` (JSON object mapping env var names to secret names)
- `models_distribution_mode` (`none`, `publish_only`, `publish_and_bootstrap`)
- `search_warm_strategy` (`staged` by default, or `full`)
- `build_timeout_minutes` (integer `1..360`, default `360`)
- workflow_call secret `DBT_NOVA_SECRET_BUNDLE_JSON` (optional JSON object of
  `secret-name -> secret-value` entries for cross-owner reusable workflow calls)

Structured mode is the recommended default:

- `dbt_command_args_json` must be a JSON array of strings.
- The workflow executes `[dbt_executable, *dbt_command_args_json]` with no shell interpolation.
- `dbt_executable` defaults to `dbt`.
- By default `dbt_executable` must resolve to `dbt`/`dbt.exe`; set
  `dbt_allow_unsafe_executable: true` only in trusted contexts.

Trusted shell mode is still available for advanced cases:

- `dbt_command` is executed as `bash -lc "<command>"` inside the caller repository checkout.
- Treat it as a trusted command surface and only use it in trusted repos/branches.
- `dbt_command` and `dbt_command_args_json` are mutually exclusive.

When using `dbt_generate_manifest: true`, prefer this structured pattern so the
workflow works across Databricks, BigQuery, Snowflake, DuckDB, and mixed profiles:

```yaml
jobs:
  build_nova_assets:
    uses: joe-broadhead/dbt-nova/.github/workflows/nova-build-assets.yml@v0.0.14
    with:
      dbt_generate_manifest: true
      dbt_command_args_json: >-
        ["compile","--target","prod"]
      dbt_env_json: >-
        {"DBT_TARGET":"prod","DBT_PROFILES_DIR":"./"}
      dbt_secret_env_map_json: >-
        {"DBT_ACCESS_TOKEN":"DBT_ACCESS_TOKEN","DBT_BIGQUERY_KEYFILE_JSON":"DBT_BIGQUERY_KEYFILE_JSON"}
      storage_instance_id: analytics-prod
    secrets:
      DBT_NOVA_SECRET_BUNDLE_JSON: ${{ secrets.DBT_NOVA_SECRET_BUNDLE_JSON }}
```

DBFS publish wrapper example:

```yaml
jobs:
  build_nova_assets:
    uses: joe-broadhead/dbt-nova/.github/workflows/nova-build-assets.yml@v0.0.14
    with:
      dbt_generate_manifest: true
      dbt_command_args_json: >-
        ["compile","--target","prod"]
      dbt_env_json: >-
        {"DBT_TARGET":"prod","DBT_PROFILES_DIR":"./","DATABRICKS_HOST":"https://<workspace>.cloud.databricks.com"}
      dbt_secret_env_map_json: >-
        {"DBT_ACCESS_TOKEN":"DBT_ACCESS_TOKEN","DATABRICKS_ACCESS_TOKEN":"DBT_ACCESS_TOKEN"}
      storage_instance_id: analytics-prod
      installer_ref: v0.0.14
      installer_install_mode: source
      publish_targets: dbfs
      publish_dbfs_prefix: dbfs:/FileStore/projects/my-project/nova-assets/prod
      publish_dry_run: false
      models_distribution_mode: none
      search_warm_strategy: staged
      build_timeout_minutes: 360
    secrets:
      DBT_NOVA_SECRET_BUNDLE_JSON: ${{ secrets.DBT_NOVA_SECRET_BUNDLE_JSON }}
```

For DBFS publish with `publish_dry_run: false`, `DATABRICKS_HOST` and
`DATABRICKS_ACCESS_TOKEN` must be present at publish time. The example above
wires both through `dbt_env_json` + `dbt_secret_env_map_json`.

Notes:

- `dbt_env_json` values are plain strings.
- `dbt_secret_env_map_json` values are looked up in this order:
  1) keys in `DBT_NOVA_SECRET_BUNDLE_JSON` (when provided),
  2) inherited workflow secrets (`secrets: inherit`, same-owner/org calls).
- Missing mapped secrets fail fast before dbt invocation.
- Use trusted `dbt_command` only when you explicitly need shell semantics.
- Keep `search_warm_strategy: staged` for large manifests or memory-constrained
  runners. It warms dense, sparse, and reranker assets in separate bounded steps
  instead of one full warmup process. Use `full` only when the runner has enough
  memory and you want the legacy single-step warm behavior.
- Increase `build_timeout_minutes` for large dbt projects or remote publishes
  that need more than the default job budget; the workflow validates values from
  `1` to `360`.

Secret setup patterns:

| Call pattern | How secrets are resolved |
|---|---|
| Same org/user, reusable workflow call | `secrets: inherit` can expose caller secrets directly |
| Cross-owner call or strict least-privilege | Pass one `DBT_NOVA_SECRET_BUNDLE_JSON` secret and map keys via `dbt_secret_env_map_json` |
| No dbt manifest generation | `dbt_secret_env_map_json` is not required |

Publish target auth requirements:

| Target | Required credentials |
|---|---|
| `s3` | Standard AWS auth env (`AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` / session token or OIDC role) |
| `gcs` | Token via one of `DBT_NOVA_GCP_ACCESS_TOKEN`, `DBT_NOVA_BIGQUERY_ACCESS_TOKEN`, `GCP_ACCESS_TOKEN`, `GOOGLE_OAUTH_ACCESS_TOKEN`, or GitHub OIDC via `gcp_workload_identity_provider` + `gcp_service_account` |
| `dbfs` | `DATABRICKS_HOST` and `DATABRICKS_ACCESS_TOKEN` |

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

When remote publish is enabled, Nova produces two bootstrap paths per target:

- versioned bootstrap: immutable, manifest-hash-specific, useful for rollback/debugging
- stable bootstrap alias: mutable latest pointer derived from `storage_instance_id`, recommended for consumers

Models distribution modes:

- `none`: do not package or publish models artifact; bootstrap omits `models_artifact_uri`.
- `publish_only`: package/publish models artifact, but bootstrap still omits `models_artifact_uri`.
- `publish_and_bootstrap`: package/publish models artifact and include `models_artifact_uri` in bootstrap.

Consumer impact by mode:

| `models_distribution_mode` | Models archive published | Bootstrap includes models URI | Consumer expectation |
|---|---|---|---|
| `none` | No | No | Use local model cache (`DBT_NOVA_EMBEDDINGS_CACHE_DIR`) or on-demand model download |
| `publish_only` | Yes | No | Optional manual consumer opt-in via `DBT_NOVA_MODELS_ARTIFACT_URI` |
| `publish_and_bootstrap` | Yes | Yes | Bootstrap-native remote model hydration |

## Optional remote publish targets

Use this when consumers should pull artifacts from cloud storage instead of
GitHub Actions artifacts.

Workflow inputs:

- `publish_targets`: comma-separated list from `s3,gcs,dbfs`
- `publish_s3_prefix`: e.g. `s3://my-bucket/nova-assets/prod`
- `publish_gcs_prefix`: e.g. `gs://my-bucket/nova-assets/prod`
- `gcp_workload_identity_provider`: optional Google workload identity provider resource for OIDC-backed GCS publish
- `gcp_service_account`: optional Google service account email used with workload identity federation for GCS publish
- `gcp_project_id`: optional Google project id passed to `google-github-actions/auth`
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
  `GCP_ACCESS_TOKEN`, `GOOGLE_OAUTH_ACCESS_TOKEN`, or GitHub OIDC-backed
  federation via `gcp_workload_identity_provider` + `gcp_service_account`
- `dbfs`: `DATABRICKS_HOST` and `DATABRICKS_ACCESS_TOKEN`

GitHub OIDC notes for GCS:

- grant the caller workflow `permissions: id-token: write`
- pass `gcp_workload_identity_provider` and `gcp_service_account`
- optional `gcp_project_id` is forwarded to `google-github-actions/auth`
- if OIDC is configured, you do not need a static `DBT_NOVA_GCP_ACCESS_TOKEN`

Installer mode guidance:

- Keep `installer_install_mode: auto` as the default.
- Use `installer_install_mode: release` with a release tag ref (for example
  `installer_ref: v0.0.14` or newer) to minimize runtime on compatible runners.
- Use `installer_install_mode: source` when you need an unreleased commit SHA
  or your runner image is incompatible with the prebuilt binary (for example
  older glibc environments).

Published object naming is deterministic:

- storage: `<prefix>/<artifact_name_storage>.tar.gz`
- manifest: `<prefix>/<artifact_name_manifest>.json`
- metadata: `<prefix>/<artifact_name_metadata>.json`
- bootstrap (versioned): `<prefix>/<artifact_name_bootstrap>.json`
- bootstrap (stable alias): `<prefix>/<storage_instance_id>-latest-bootstrap.json`
- models (optional): `<prefix>/<artifact_name_models>.tar.gz`

Producer outputs include:

- `published_targets` (comma-separated successful targets)
- `published_bootstrap_latest_uris` (stable alias URIs keyed by target)
- `artifact_name_publish_summary` (artifact containing `published_*_uris` JSON payloads)
- legacy `published_*_uris` outputs are deprecated and return `{}` for compatibility

Example (extract DBFS stable bootstrap alias from publish summary artifact):

```bash
PUBLISH_SUMMARY_DIR="$(mktemp -d)"
gh run download "$GITHUB_RUN_ID" --name "$ARTIFACT_NAME_PUBLISH_SUMMARY" --dir "$PUBLISH_SUMMARY_DIR"
PUBLISH_SUMMARY_JSON="$(find "$PUBLISH_SUMMARY_DIR" -type f -name '*.json' | head -n 1)"
BOOTSTRAP_URI="$(jq -r '.published_bootstrap_latest_uris.dbfs' "$PUBLISH_SUMMARY_JSON")"
```

Example `publish-summary` JSON shape:

```json
{
  "published_storage_uris": {
    "dbfs": "dbfs:/FileStore/projects/my-project/nova-assets/prod/analytics-storage-<run>-<manifest>.tar.gz"
  },
  "published_manifest_uris": {
    "dbfs": "dbfs:/FileStore/projects/my-project/nova-assets/prod/analytics-manifest-<run>-<manifest>.json"
  },
  "published_metadata_uris": {
    "dbfs": "dbfs:/FileStore/projects/my-project/nova-assets/prod/analytics-storage-<run>-<manifest>-metadata.json"
  },
  "published_bootstrap_uris": {
    "dbfs": "dbfs:/FileStore/projects/my-project/nova-assets/prod/analytics-prod-storage-<run>-<manifest>-bootstrap.json"
  },
  "published_bootstrap_latest_uris": {
    "dbfs": "dbfs:/FileStore/projects/my-project/nova-assets/prod/analytics-prod-latest-bootstrap.json"
  },
  "published_models_uris": {}
}
```

Post-run verification checklist:

1. Check workflow summary for resolved artifact names and publish targets.
2. Download `artifact_name_publish_summary` and confirm expected target URI keys.
3. Set `DBT_NOVA_BOOTSTRAP_URI` to the stable bootstrap alias and run `dbt-nova health check --json`.
4. Confirm health reports `bootstrap.loaded=true` and `artifact_consumer.storage_materialized=true`.
5. After a later publish, run `reload_manifest` to adopt the newer assets without editing MCP config.

## Consumer

Nova now supports **native remote artifact consumption**. Manual download/extract is optional.

### Option A (recommended): writable first-run hydration

Set these env vars:

- `DBT_NOVA_BOOTSTRAP_URI` (recommended one-URI setup)
- `DBT_NOVA_ARTIFACT_FETCH_POLICY=if_missing` (or `always` when you intentionally want to re-materialize)
- `DBT_NOVA_STORAGE_READ_ONLY` unset or `false`

Optional:

- `DBT_NOVA_STORAGE_INSTANCE_ID`, `DBT_NOVA_STORAGE_ARTIFACT_URI`, `DBT_NOVA_METADATA_ARTIFACT_URI`,
  `DBT_NOVA_MODELS_ARTIFACT_URI` (explicit mode if you do not use bootstrap URI)
- `DBT_NOVA_ARTIFACTS_CACHE_DIR` (defaults to `<storage_root>/artifacts`)
- `DBT_NOVA_ARTIFACT_TIMEOUT_SECS` (default `300`)
- `DBT_NOVA_ARTIFACT_MAX_BYTES` (compressed artifact cap, default `2147483648`)
- `DBT_NOVA_ARTIFACT_ARCHIVE_MAX_ENTRIES` (archive entry cap, default `200000`)
- `DBT_NOVA_ARTIFACT_ARCHIVE_MAX_UNCOMPRESSED_BYTES` (decompressed archive cap, default `10737418240`)
- `DBT_NOVA_ARTIFACT_ALLOW_HTTP=true` (only for non-TLS artifact URIs; not recommended)

Local shell example:

```bash
export DBT_MANIFEST_PATH="$PWD/manifest.json"
export DBT_NOVA_STORAGE_DIR="$PWD/.dbt-nova"
export DBT_NOVA_BOOTSTRAP_URI="s3://my-bucket/nova-assets/prod/analytics-prod-latest-bootstrap.json"
export DBT_NOVA_ARTIFACT_FETCH_POLICY="if_missing"
unset DBT_NOVA_STORAGE_READ_ONLY

dbt-nova health check --json
```

CI shell example:

```bash
export DBT_NOVA_STORAGE_DIR="$RUNNER_TEMP/.dbt-nova"
export DBT_NOVA_BOOTSTRAP_URI="$BOOTSTRAP_URI"
export DBT_NOVA_ARTIFACT_FETCH_POLICY="if_missing"
unset DBT_NOVA_STORAGE_READ_ONLY

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
        "DBT_NOVA_BOOTSTRAP_URI": "s3://my-bucket/nova-assets/prod/analytics-prod-latest-bootstrap.json",
        "DBT_NOVA_ARTIFACT_FETCH_POLICY": "if_missing"
      }
    }
  }
}
```

Recommended consumer pattern:

- Point `DBT_NOVA_BOOTSTRAP_URI` at the stable alias (`<storage_instance_id>-latest-bootstrap.json`).
- Keep the versioned bootstrap URIs for rollback/debugging only.
- After producers publish a newer asset set, run `reload_manifest` so Nova re-fetches the stable bootstrap alias and adopts the new artifacts.
- Do not combine `DBT_NOVA_STORAGE_READ_ONLY=true` with `DBT_NOVA_ARTIFACT_FETCH_POLICY=if_missing|always` on a cold machine.

Use `health` to verify runtime decisions:

- `artifact_consumer.enabled`
- `artifact_consumer.storage_read_only`
- `artifact_consumer.consumer_mode_hint`
- `artifact_consumer.guidance`
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

### Option B: strict read-only reuse after pre-materialization

Use this only after local artifacts already exist from a previous writable run
or from manual extraction.

Set:

- `DBT_NOVA_STORAGE_READ_ONLY=true`
- `DBT_NOVA_ARTIFACT_FETCH_POLICY=never`
- `DBT_NOVA_BOOTSTRAP_URI` (or explicit artifact URIs)

Example:

```bash
export DBT_NOVA_STORAGE_DIR="$PWD/.dbt-nova"
export DBT_NOVA_STORAGE_READ_ONLY="true"
export DBT_NOVA_BOOTSTRAP_URI="s3://my-bucket/nova-assets/prod/analytics-prod-latest-bootstrap.json"
export DBT_NOVA_ARTIFACT_FETCH_POLICY="never"

dbt-nova health check --json
```

### Option C: manual extraction fallback

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
| `Storage is read-only; cannot materialize storage artifacts on first-run hydration` | Cold machine is configured with `DBT_NOVA_STORAGE_READ_ONLY=true` and `DBT_NOVA_ARTIFACT_FETCH_POLICY=if_missing|always` | First hydrate with writable storage (`DBT_NOVA_STORAGE_READ_ONLY=false` or unset), then switch to `DBT_NOVA_STORAGE_READ_ONLY=true` with `DBT_NOVA_ARTIFACT_FETCH_POLICY=never` after assets exist locally |
| `Storage is read-only and no reusable index is available` | Missing storage files, mismatched `storage_instance_id`, or manifest content mismatch | Re-download artifacts, verify instance id, verify manifest is identical to producer input |
| Metadata contract validation fails | Missing/corrupt `nova-build-metadata.json` or unsupported contract version | Re-run producer and consume both storage + metadata artifacts together |
| Health passes but embeddings are missing | Models artifact not provided in consumer setup | Native mode: set `DBT_NOVA_MODELS_ARTIFACT_URI` to the producer models artifact URI. Manual mode: extract models artifact and set `DBT_NOVA_EMBEDDINGS_CACHE_DIR`. |

## Related docs

- [MCP Client Configs](../getting-started/mcp-clients.md)
- [Modes & Combinations](../getting-started/modes-and-combinations.md)
- [Troubleshooting](troubleshooting.md)
- [CI & Automation](../development/ci.md)
