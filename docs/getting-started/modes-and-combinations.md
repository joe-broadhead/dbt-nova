# Modes & Combinations

dbt-nova behavior is the product of **five independent planes**:

1. How you install the binary
2. Where manifest content comes from
3. Where storage/index artifacts come from
4. Where embedding/reranker model files come from
5. Which SQL provider is active

This page maps those planes, shows precedence rules, and gives validated deployment profiles.

## 1) Binary Installation Plane

| Mode | How | Notes |
|---|---|---|
| Release slim (default) | `scripts/install.sh --slim` | Binary only. Semantic layers are opt-in; model files are resolved only when those layers are enabled. |
| Release bundled | `scripts/install.sh --bundled` | Binary + colocated `models/` when bundled assets exist. Installer falls back to slim if bundled artifact is unavailable. |
| Source build | `cargo build --release` | Use for unreleased commits or unsupported runner/platform. |

Installer controls:

- `DBT_NOVA_INSTALL_FLAVOR=bundled|slim`
- `DBT_NOVA_INSTALL_WARM_MODELS=1` (or `--warm-models`, slim only)
- `DBT_NOVA_INSTALL_SKILLS=1` (or `--install-skills`)
- `DBT_NOVA_SKILL_NAME=<skill>` (optional single standalone skill)
- `DBT_NOVA_INSTALL_NONINTERACTIVE=1`
- `DBT_NOVA_INSTALL_DIR`, `DBT_NOVA_SKILLS_DIR`
- `DBT_NOVA_VERIFY_CHECKSUM=1|0`, `DBT_NOVA_VERIFY_SIGNATURE=auto|1|0`

## 2) Manifest Source Plane

| Source | Primary config | Typical use |
|---|---|---|
| Local file | `DBT_MANIFEST_PATH` | Local dev, deterministic testing |
| Remote manifest URI | `DBT_NOVA_MANIFEST_URI` | Centralized manifest hosting (`s3://`, `gs://`, `dbfs:/`, `https://`) |
| Bootstrap contract | `DBT_NOVA_BOOTSTRAP_URI` | One-URI onboarding (can set `manifest_uri` + artifact URIs) |

Manifest precedence:

1. Explicit env/CLI values win.
2. Bootstrap only fills missing fields.
3. `manifest_uri` from bootstrap is only applied when:
   - `DBT_NOVA_MANIFEST_URI` is empty, and
   - `DBT_MANIFEST_PATH` was not explicitly set (including explicit `manifest.json`).

## 3) Storage / Index Artifact Plane

| Mode | Required config | Behavior |
|---|---|---|
| Local build (mutable) | none (default) | Nova builds indexes locally from manifest and refresh policy. |
| Remote artifact consumer | `DBT_NOVA_STORAGE_ARTIFACT_URI` + `DBT_NOVA_METADATA_ARTIFACT_URI` | Nova materializes prebuilt storage artifacts locally, then serves from them. |
| Bootstrap-driven remote consumer | `DBT_NOVA_BOOTSTRAP_URI` | Bootstrap can inject storage/metadata/model artifact URIs. |

Artifact controls:

- `DBT_NOVA_ARTIFACT_FETCH_POLICY=if_missing|always|never`
- `DBT_NOVA_ARTIFACTS_CACHE_DIR`
- `DBT_NOVA_ARTIFACT_TIMEOUT_SECS`
- `DBT_NOVA_ARTIFACT_MAX_BYTES`
- `DBT_NOVA_ARTIFACT_ARCHIVE_MAX_ENTRIES`
- `DBT_NOVA_ARTIFACT_ARCHIVE_MAX_UNCOMPRESSED_BYTES`
- `DBT_NOVA_STORAGE_READ_ONLY=true` only after local artifacts are already materialized

Rule: remote artifact mode is valid only when **both** storage + metadata artifact URIs are present.

## 4) Model Asset Plane (Embeddings + Sparse + Reranker)

### Resolution Order

If `DBT_NOVA_EMBEDDINGS_CACHE_DIR` is unset, Nova resolves model path in this order:

1. `models/` next to the executable
2. `~/.local/bin/models` (if present)
3. `~/.dbt-nova/.fastembed_cache`

### Ways to supply models

| Strategy | How | Best for |
|---|---|---|
| Colocated bundled models | Bundled install with `models/` beside binary | Air-gapped/local fixed runtime |
| Pre-warmed local cache | `--warm-models` or `scripts/warm_models.sh` + fixed `DBT_NOVA_EMBEDDINGS_CACHE_DIR` | Laptops, consistent MCP startup |
| Remote models artifact | `DBT_NOVA_MODELS_ARTIFACT_URI` (or bootstrap field) | Fully centralized artifact distribution |
| On-demand download | Slim install without prewarm + semantic layers enabled | Fastest install path, slower first semantic startup |

Important: if bootstrap omits `models_artifact_uri` (for example producer `models_distribution_mode=none`), consumers must rely on pre-warmed/local cache or on-demand model download.

## 5) SQL Provider Plane

`execute_sql` and `run_recipe` use `DBT_NOVA_SQL_PROVIDER`:

- `databricks` (default): requires Databricks host/token + warehouse pointer
- `bigquery`: requires project id + Google auth path/token
- `snowflake`: requires account URL/account id + warehouse + key-pair, OAuth, PAT, or local external browser auth
- `duckdb`: requires `DBT_NOVA_DUCKDB_PATH` (read-only execution model)

Provider diagnostics:

```bash
dbt-nova tool call execute_sql \
  --params-json '{"preflight_only":true}' \
  --json
```

## Recommended Deployment Profiles

| Profile | Manifest | Storage | Models | Key config |
|---|---|---|---|---|
| Local dev (simple) | local path | local build | on-demand | `DBT_MANIFEST_PATH` |
| Local dev (stable) | local path | local build | pre-warmed local cache | `DBT_MANIFEST_PATH`, `DBT_NOVA_EMBEDDINGS_CACHE_DIR` |
| Remote manifest, local index | `DBT_NOVA_MANIFEST_URI` | local build | pre-warmed local cache | `DBT_NOVA_MANIFEST_URI`, `DBT_NOVA_EMBEDDINGS_CACHE_DIR` |
| Hosted bootstrap consumer, discovery-only | bootstrap | prebuilt artifacts hydrated locally | remote models artifact or pre-warmed cache | `DBT_NOVA_SERVER_TRANSPORT=streamable_http`, `PORT`, `DBT_NOVA_HTTP_EXPECT_AUTH_PROXY=true`, `DBT_NOVA_TOOL_DENYLIST=execute_sql,run_recipe,reload_manifest,show_config,validate_config,inspect_storage,prune_storage,cleanup_storage,warm_manifest`, `DBT_NOVA_STORAGE_DIR=/tmp/dbt-nova`, `DBT_NOVA_EMBEDDINGS_CACHE_DIR=/tmp/dbt-nova/models`, `DBT_NOVA_BOOTSTRAP_URI`, `DBT_NOVA_ARTIFACT_FETCH_POLICY=if_missing`, `DBT_NOVA_STORAGE_READ_ONLY=false` |
| Hosted bootstrap consumer, SQL-enabled | bootstrap | prebuilt artifacts hydrated locally | remote models artifact or pre-warmed cache | Hosted discovery-only config plus SQL provider credentials, least-privilege warehouse access, and an empty or custom `DBT_NOVA_TOOL_DENYLIST` |
| Prebuilt writable first-run consumer | bootstrap or explicit artifact URIs | prebuilt artifacts hydrated locally | local pre-warmed cache or remote models artifact | `DBT_NOVA_BOOTSTRAP_URI` or artifact URIs, `DBT_NOVA_ARTIFACT_FETCH_POLICY=if_missing`, `DBT_NOVA_STORAGE_READ_ONLY=false` |
| Prebuilt strict read-only consumer | bootstrap or explicit artifact URIs | pre-materialized local storage | pre-materialized local models or pre-warmed cache | `DBT_NOVA_STORAGE_READ_ONLY=true`, `DBT_NOVA_ARTIFACT_FETCH_POLICY=never`, bootstrap/artifact vars, `DBT_NOVA_EMBEDDINGS_CACHE_DIR` |

## Producer/Consumer Alignment (Prebuilt Assets)

When using `.github/workflows/nova-build-assets.yml`:

- `models_distribution_mode=none`: no models artifact published, bootstrap excludes models URI
- `models_distribution_mode=publish_only`: models artifact published, bootstrap still excludes models URI
- `models_distribution_mode=publish_and_bootstrap`: models artifact published and bootstrap includes models URI

Use `publish_and_bootstrap` only when you want consumers to hydrate models from remote artifacts by default.

## Validation Checklist

```bash
# 1) Validate merged config
dbt-nova config validate --json

# 2) Check runtime state and what was applied
dbt-nova health check --json

# 3) Confirm SQL provider wiring
dbt-nova tool call execute_sql --params-json '{"preflight_only":true}' --json
```

In `health`, verify:

- `bootstrap.loaded`, `bootstrap.applied_fields`
- `artifact_consumer.enabled`, `artifact_consumer.fetch_policy`
- `artifact_consumer.storage_materialized`
- `artifact_consumer.models_materialized`
- `status` is `ready` for steady-state operation

## See Also

- [Installation](installation.md)
- [MCP Client Configs](mcp-clients.md)
- [Configuration Reference](../configuration/reference.md)
- [Prebuilt Asset Workflow](../operations/prebuilt-assets.md)
