# MCP Client Configs

All MCP clients should set one of:
- `DBT_MANIFEST_PATH`
- `DBT_NOVA_MANIFEST_URI`
- `DBT_NOVA_BOOTSTRAP_URI` (bootstrap can populate `manifest_uri`)
Databricks variables are required only if you use the `execute_sql` tool with
`DBT_NOVA_SQL_PROVIDER=databricks` (default).
BigQuery variables are required only if you use `DBT_NOVA_SQL_PROVIDER=bigquery`.
DuckDB variables are required only if you use `DBT_NOVA_SQL_PROVIDER=duckdb`.
For all SQL providers, object-level preflight checks (`preflight_catalog`,
`preflight_schema`, `preflight_relation`) pass only when the probe returns at
least one row.

For full installation/runtime combination guidance (manifest vs bootstrap vs
remote artifacts vs model cache strategies), see
[Modes & Combinations](modes-and-combinations.md).

For slim installs, set a stable `DBT_NOVA_EMBEDDINGS_CACHE_DIR` (recommended:
`~/.dbt-nova/.fastembed_cache`) so model downloads are reused across sessions/clients.
Install `cosign` first when using the strict release installer example below.
If you installed with:

```bash
curl -fsSL https://raw.githubusercontent.com/joe-broadhead/dbt-nova/v0.0.6/scripts/install.sh | \
  DBT_NOVA_EMBEDDINGS_CACHE_DIR="$HOME/.dbt-nova/.fastembed_cache" \
  DBT_NOVA_WARMUP_REQUIRED_MODELS=3 \
  DBT_NOVA_VERSION=v0.0.6 DBT_NOVA_VERIFY_SIGNATURE=1 bash -s -- --slim --warm-models --non-interactive
```

use that exact same `DBT_NOVA_EMBEDDINGS_CACHE_DIR` path in your MCP client env.

Semantic layers are disabled by default. If you want dense search, sparse search,
or reranking in your MCP client, enable them explicitly with:

- `DBT_NOVA_SEARCH_ENABLE_VECTOR=true`
- `DBT_NOVA_SEARCH_ENABLE_SPARSE=true`
- `DBT_NOVA_SEARCH_ENABLE_RERANKER=true`

For large manifests or memory-constrained machines, keep those three variables
unset or explicitly set them to `false`, and do not call `warm_manifest` unless
you have intentionally provisioned the machine for semantic cache writes.

## Readiness Polling

MCP clients may receive `INDEX_BUILDING` while the manifest is still loading or
indexes are still materializing. Treat that as a startup state, not as a failed
tool contract.

Automation should wait for one of:

- MCP `health` with `data.ready_for_traffic=true`
- HTTP `/readyz` returning ready when using streamable HTTP

Do not treat `tools/list` or MCP `initialize` as readiness proof; those can
succeed before search and metadata tools are safe to call. For local release or
large-manifest checks, use:

```bash
scripts/smoke_release_no_warm.sh --manifest-path target/manifest.json
```

Optional manifest pruning (applies to MCP server startup and reloads):
- `DBT_NOVA_PRUNE_ALLOW_IDS` (JSON array of dbt `unique_id` patterns)
- `DBT_NOVA_PRUNE_DENY_IDS` (JSON array of dbt `unique_id` patterns; deny wins)
- Matching is on `unique_id`, not `fqn`.
- Invalid JSON fails startup; do not use comma-separated strings.

## Prebuilt Consumer Setup

If you consume prebuilt Nova storage artifacts (built by the reusable producer
workflow), set these env vars in your MCP client:

- `DBT_NOVA_STORAGE_DIR` (local Nova storage root)
- `DBT_NOVA_BOOTSTRAP_URI` (recommended one-URI setup)
- `DBT_NOVA_ARTIFACT_FETCH_POLICY` (recommended first run: `if_missing`)

For first-run bootstrap/artifact hydration, leave `DBT_NOVA_STORAGE_READ_ONLY`
unset or set it to `false`.

Use strict read-only mode only after local artifacts already exist:

- `DBT_NOVA_STORAGE_READ_ONLY=true`
- `DBT_NOVA_ARTIFACT_FETCH_POLICY=never`

Optional explicit mode (if you do not use bootstrap URI):

- `DBT_NOVA_STORAGE_INSTANCE_ID` (must match producer workflow input)
- `DBT_NOVA_STORAGE_ARTIFACT_URI`
- `DBT_NOVA_METADATA_ARTIFACT_URI`
- `DBT_NOVA_MODELS_ARTIFACT_URI` (optional)

Recommended with prebuilt artifacts:

- Keep `DBT_MANIFEST_PATH` or `DBT_NOVA_MANIFEST_URI` pointed at the same
  manifest content used by the producer build.
- Use the same Nova release on producer and consumer.
- Prefer the stable bootstrap alias (`<storage_instance_id>-latest-bootstrap.json`) and allow Nova to cache fetched artifacts locally.
- After a producer publishes new assets, run `reload_manifest` with no arguments
  to pick up the newer bootstrap via the same URI. Changing source URI/path,
  refresh interval, or storage identity from MCP requires
  `DBT_NOVA_MCP_ENABLE_MANIFEST_RELOAD=1`.
- Do not combine `DBT_NOVA_STORAGE_READ_ONLY=true` with `DBT_NOVA_ARTIFACT_FETCH_POLICY=if_missing|always` on a cold machine.

Bootstrap precedence reminder:

- Explicit env vars win.
- Bootstrap only fills missing fields.
- `manifest_uri` from bootstrap is skipped when `DBT_MANIFEST_PATH` was explicitly set.

Codex TOML example:

```toml
[mcp_servers.dbt-nova]
command = "/path/to/dbt-nova"
startup_timeout_sec = 60

[mcp_servers.dbt-nova.env]
DBT_NOVA_STORAGE_DIR = "/path/to/.dbt-nova"
DBT_NOVA_BOOTSTRAP_URI = "s3://my-bucket/nova-assets/prod/analytics-prod-latest-bootstrap.json"
DBT_NOVA_ARTIFACT_FETCH_POLICY = "if_missing"
DBT_NOVA_EMBEDDINGS_CACHE_DIR = "/Users/<you>/.dbt-nova/.fastembed_cache"
```

Strict read-only variant after local materialization:

```toml
[mcp_servers.dbt-nova]
command = "/path/to/dbt-nova"
startup_timeout_sec = 60

[mcp_servers.dbt-nova.env]
DBT_NOVA_STORAGE_DIR = "/path/to/.dbt-nova"
DBT_NOVA_STORAGE_READ_ONLY = "true"
DBT_NOVA_BOOTSTRAP_URI = "s3://my-bucket/nova-assets/prod/analytics-prod-latest-bootstrap.json"
DBT_NOVA_ARTIFACT_FETCH_POLICY = "never"
DBT_NOVA_EMBEDDINGS_CACHE_DIR = "/Users/<you>/.dbt-nova/.fastembed_cache"
```

For producer/consumer workflow details, see:
- [Prebuilt Asset Workflow](../operations/prebuilt-assets.md)

## Claude Desktop (`claude_desktop_config.json`)

```json
{
  "mcpServers": {
    "dbt-nova": {
      "command": "/path/to/dbt-nova",
      "env": {
        "DBT_MANIFEST_PATH": "/path/to/manifest.json",
        "DBT_NOVA_PRUNE_ALLOW_IDS": "[\"model.my_proj.fct_orders\",\"model.my_proj.dim_*\"]",
        "DBT_NOVA_PRUNE_DENY_IDS": "[\"model.my_proj.dim_legacy_*\"]",
        "DBT_NOVA_EMBEDDINGS_CACHE_DIR": "/Users/<you>/.dbt-nova/.fastembed_cache",
        "DATABRICKS_HOST": "https://<workspace>.cloud.databricks.com",
        "DATABRICKS_HTTP_PATH": "/sql/1.0/warehouses/<warehouse_id>",
        "DATABRICKS_ACCESS_TOKEN": "<token>"
      }
    }
  }
}
```

## Codex CLI (`config.toml`)

```toml
[mcp_servers.dbt-nova]
command = "/path/to/dbt-nova"
startup_timeout_sec = 60

[mcp_servers.dbt-nova.env]
DBT_MANIFEST_PATH = "/path/to/manifest.json"
DBT_NOVA_PRUNE_ALLOW_IDS = "[\"model.my_proj.fct_orders\",\"model.my_proj.dim_*\"]"
DBT_NOVA_PRUNE_DENY_IDS = "[\"model.my_proj.dim_legacy_*\"]"
DATABRICKS_HOST = "https://<workspace>.cloud.databricks.com"
DATABRICKS_HTTP_PATH = "/sql/1.0/warehouses/<warehouse_id>"
DATABRICKS_ACCESS_TOKEN = "<token>"
DBT_NOVA_EMBEDDINGS_CACHE_DIR = "/Users/<you>/.dbt-nova/.fastembed_cache"
```

## Gemini CLI (config JSON)

Gemini CLI validates tool JSON schemas more strictly than other MCP clients.
If you see errors like:

- `no schema with key or ref "https://json-schema.org/draft/2020-12/schema"`
- `"nullable" cannot be used without "type"`

set `DBT_NOVA_DISABLE_TOOL_SCHEMAS=1` in the Gemini MCP env to strip schema
hints from tool definitions.

```json
{
  "mcpServers": {
    "dbt-nova": {
      "command": "/path/to/dbt-nova",
      "env": {
        "DBT_MANIFEST_PATH": "/path/to/manifest.json",
        "DBT_NOVA_EMBEDDINGS_CACHE_DIR": "/Users/<you>/.dbt-nova/.fastembed_cache",
        "DBT_NOVA_DISABLE_TOOL_SCHEMAS": "1",
        "DATABRICKS_HOST": "https://<workspace>.cloud.databricks.com",
        "DATABRICKS_HTTP_PATH": "/sql/1.0/warehouses/<warehouse_id>",
        "DATABRICKS_ACCESS_TOKEN": "<token>"
      }
    }
  }
}
```

## Databricks SQL Variables

Required:
- `DATABRICKS_HOST`
- `DATABRICKS_ACCESS_TOKEN`
- One of:
  - `DATABRICKS_HTTP_PATH`
  - `DATABRICKS_SQL_WAREHOUSE_ID`

Optional:
- `DATABRICKS_WAIT_TIMEOUT_S` (default: `10`, clamped to `0` or `5–50`)
- `DATABRICKS_POLL_INTERVAL_MS` (default: `1000`)
- `DATABRICKS_MAX_POLL_SECONDS` (default: `600`)
- `DATABRICKS_TIMEOUT_MS` (default: derived from wait timeout, minimum `30000`)
- `DATABRICKS_MAX_GET_RETRIES` (default: `2`)
- `DBT_NOVA_SQL_MAX_ROW_LIMIT` (default: `10000`)
- `DBT_NOVA_SQL_MAX_BYTE_LIMIT` (default: `100000000`)
- `DBT_NOVA_SQL_MAX_CHUNKS` (default: `100`)
- `DBT_NOVA_SQL_MAX_POLL_SECONDS` (default: `900`)
- `DBT_NOVA_SQL_MIN_POLL_INTERVAL_MS` (default: `200`)
- `DBT_NOVA_SQL_MAX_CONCURRENT` (default: `10`)
- `DBT_NOVA_SQL_MAX_QUEUE` (default: `20`)
- `DBT_NOVA_SQL_QUEUE_TIMEOUT_MS` (default: `30000`)

## BigQuery SQL Variables

Required:
- One of:
  - `DBT_NOVA_BIGQUERY_PROJECT_ID`
  - `DBT_NOVA_GCP_PROJECT_ID`
  - `GOOGLE_CLOUD_PROJECT`
  - `GCP_PROJECT_ID`
- One of:
  - `DBT_NOVA_BIGQUERY_ACCESS_TOKEN`
  - `DBT_NOVA_GCP_ACCESS_TOKEN`
  - `GCP_ACCESS_TOKEN`
  - `GOOGLE_OAUTH_ACCESS_TOKEN`
  - `GOOGLE_APPLICATION_CREDENTIALS` (service-account JSON)
  - `gcloud auth application-default print-access-token` available in PATH

Optional:
- `DBT_NOVA_BIGQUERY_LOCATION` (e.g., `US`, `EU`)
- `DBT_NOVA_BIGQUERY_TIMEOUT_MS` (default: `30000`)
- `DBT_NOVA_BIGQUERY_API_BASE_URL` (advanced/test override; defaults to Google;
  `http://` is accepted only for loopback test servers)
- `DBT_NOVA_SQL_MAX_ROW_LIMIT` (default: `10000`)
- `DBT_NOVA_SQL_MAX_BYTE_LIMIT` (default: `100000000`)
- `DBT_NOVA_SQL_MAX_CHUNKS` (default: `100`)
- `DBT_NOVA_SQL_MAX_POLL_SECONDS` (default: `900`)
- `DBT_NOVA_SQL_MIN_POLL_INTERVAL_MS` (default: `200`)
- `DBT_NOVA_SQL_MAX_CONCURRENT` (default: `10`)
- `DBT_NOVA_SQL_MAX_QUEUE` (default: `20`)
- `DBT_NOVA_SQL_QUEUE_TIMEOUT_MS` (default: `30000`)

## Snowflake SQL Variables

Required:
- `DBT_NOVA_SQL_PROVIDER=snowflake`
- One of:
  - `DBT_NOVA_SNOWFLAKE_ACCOUNT`
  - `DBT_NOVA_SNOWFLAKE_ACCOUNT_URL`
- `DBT_NOVA_SNOWFLAKE_WAREHOUSE`
- One auth mode:

| Mode | Best for | Required variables | Notes |
| --- | --- | --- | --- |
| `keypair` | CI, services, and long-running MCP servers | `DBT_NOVA_SNOWFLAKE_USER` plus `DBT_NOVA_SNOWFLAKE_PRIVATE_KEY_PATH` or `DBT_NOVA_SNOWFLAKE_PRIVATE_KEY_PEM` | Default when no token env vars are present. Keys must be unencrypted RSA PEM today. |
| `oauth` | Existing Snowflake OAuth or External OAuth token flows | `DBT_NOVA_SNOWFLAKE_AUTH=oauth` plus `DBT_NOVA_SNOWFLAKE_OAUTH_TOKEN` | Nova uses a supplied bearer token; it does not mint or refresh OAuth tokens. |
| `pat` | Simple service or personal token setup | `DBT_NOVA_SNOWFLAKE_AUTH=pat` plus `DBT_NOVA_SNOWFLAKE_PAT` | Nova sends the token as a Snowflake endpoint token. Keep it in a secret manager and rotate it. |
| `externalbrowser` | Local desktop SSO and Okta/SAML testing | `DBT_NOVA_SNOWFLAKE_AUTH=externalbrowser`, `DBT_NOVA_SNOWFLAKE_USER`, and `DBT_NOVA_SNOWFLAKE_ACCOUNT` | Local interactive only. Not allowed in CI or non-loopback hosted HTTP mode. |

Optional:
- `DBT_NOVA_SNOWFLAKE_DATABASE`
- `DBT_NOVA_SNOWFLAKE_SCHEMA`
- `DBT_NOVA_SNOWFLAKE_ROLE`
- `DBT_NOVA_SNOWFLAKE_JWT_ACCOUNT` (JWT account identifier override; required for key-pair auth when using `DBT_NOVA_SNOWFLAKE_ACCOUNT_URL` without `DBT_NOVA_SNOWFLAKE_ACCOUNT`)
- `DBT_NOVA_SNOWFLAKE_TIMEOUT_MS` (default: `30000`)
- `DBT_NOVA_SNOWFLAKE_STATEMENT_TIMEOUT_S` (default: `60`; `0` requests Snowflake SQL API's maximum timeout window)
- `DBT_NOVA_SNOWFLAKE_POLL_INTERVAL_MS` (default: `1000`)
- `DBT_NOVA_SNOWFLAKE_MAX_POLL_SECONDS` (default: `600`)
- `DBT_NOVA_SNOWFLAKE_MAX_CHUNKS` (default: `50`)
- `DBT_NOVA_SNOWFLAKE_EXTERNAL_BROWSER_TIMEOUT_S` (default: `120`)
- `DBT_NOVA_SNOWFLAKE_EXTERNAL_BROWSER_OPEN` (`true`|`false`, default: `true`)
- `DBT_NOVA_SNOWFLAKE_EXTERNAL_BROWSER_CALLBACK_PORT` (optional fixed loopback callback port)
- `DBT_NOVA_SQL_MAX_ROW_LIMIT` (default: `10000`)
- `DBT_NOVA_SQL_MAX_BYTE_LIMIT` (default: `100000000`)
- `DBT_NOVA_SQL_MAX_CHUNKS` (default: `100`)
- `DBT_NOVA_SQL_MAX_POLL_SECONDS` (default: `900`)
- `DBT_NOVA_SQL_MIN_POLL_INTERVAL_MS` (default: `200`)
- `DBT_NOVA_SQL_MAX_CONCURRENT` (default: `10`)
- `DBT_NOVA_SQL_MAX_QUEUE` (default: `20`)
- `DBT_NOVA_SQL_QUEUE_TIMEOUT_MS` (default: `30000`)

Snowflake behavior notes:
- Named `:parameter` placeholders are rewritten to Snowflake SQL API `?` binds.
- Null SQL parameters require explicit `parameter_types`.
- `warehouse_id` overrides `DBT_NOVA_SNOWFLAKE_WAREHOUSE` for a single call.
- `DBT_NOVA_SNOWFLAKE_ACCOUNT_URL` must be an account root URL such as `https://<host>`, not an `/api` endpoint.
- If `DBT_NOVA_SNOWFLAKE_AUTH` is omitted, Nova infers `pat` when
  `DBT_NOVA_SNOWFLAKE_PAT` is set, infers `oauth` when
  `DBT_NOVA_SNOWFLAKE_OAUTH_TOKEN` is set, and otherwise uses key-pair JWT auth.
- Key-pair JWT claims normalize account identifiers for Snowflake: account/user values are uppercased, periods are replaced with hyphens, legacy locator-style account region suffixes are excluded, and fully qualified organization/account names are preserved.
- For key-pair auth with a private-link or custom account URL, set both `DBT_NOVA_SNOWFLAKE_ACCOUNT_URL` and `DBT_NOVA_SNOWFLAKE_JWT_ACCOUNT`.
- Key-pair auth supports unencrypted RSA PEM keys; encrypted private keys are not supported yet.
- Snowflake SQL API workload identity federation is not implemented yet. Use
  key-pair JWT, OAuth, or PAT auth for hosted automation.
- External browser auth is for local interactive use. Nova binds a `127.0.0.1`
  callback listener, opens the system browser for Snowflake SSO, keeps the
  returned Snowflake session token only in memory, and supports Okta SAML
  callbacks that omit `proofKey`. Use key-pair JWT, OAuth, or PAT auth for CI
  and hosted/non-loopback streamable HTTP deployments.

### Snowflake PAT Example (Codex CLI)

```toml
[mcp_servers.dbt-nova]
command = "/path/to/dbt-nova"
startup_timeout_sec = 60

[mcp_servers.dbt-nova.env]
DBT_MANIFEST_PATH = "/path/to/manifest.json"
DBT_NOVA_SQL_PROVIDER = "snowflake"
DBT_NOVA_SNOWFLAKE_ACCOUNT = "myorg-myaccount"
DBT_NOVA_SNOWFLAKE_WAREHOUSE = "ANALYST_WH"
DBT_NOVA_SNOWFLAKE_DATABASE = "ANALYTICS"
DBT_NOVA_SNOWFLAKE_SCHEMA = "REPORTING"
DBT_NOVA_SNOWFLAKE_ROLE = "REPORTER"
DBT_NOVA_SNOWFLAKE_AUTH = "pat"
DBT_NOVA_SNOWFLAKE_PAT = "<token>"
DBT_NOVA_EMBEDDINGS_CACHE_DIR = "/Users/<you>/.dbt-nova/.fastembed_cache"
```

Key-pair JWT example:

```toml
[mcp_servers.dbt-nova]
command = "/path/to/dbt-nova"
startup_timeout_sec = 60

[mcp_servers.dbt-nova.env]
DBT_MANIFEST_PATH = "/path/to/manifest.json"
DBT_NOVA_SQL_PROVIDER = "snowflake"
DBT_NOVA_SNOWFLAKE_ACCOUNT = "myorg-myaccount"
DBT_NOVA_SNOWFLAKE_USER = "svc_dbt_nova"
DBT_NOVA_SNOWFLAKE_WAREHOUSE = "ANALYST_WH"
DBT_NOVA_SNOWFLAKE_DATABASE = "ANALYTICS"
DBT_NOVA_SNOWFLAKE_SCHEMA = "REPORTING"
DBT_NOVA_SNOWFLAKE_ROLE = "REPORTER"
DBT_NOVA_SNOWFLAKE_AUTH = "keypair"
DBT_NOVA_SNOWFLAKE_PRIVATE_KEY_PATH = "/secure/path/rsa_key.p8"
DBT_NOVA_EMBEDDINGS_CACHE_DIR = "/Users/<you>/.dbt-nova/.fastembed_cache"
```

OAuth bearer token example:

```toml
[mcp_servers.dbt-nova]
command = "/path/to/dbt-nova"
startup_timeout_sec = 60

[mcp_servers.dbt-nova.env]
DBT_MANIFEST_PATH = "/path/to/manifest.json"
DBT_NOVA_SQL_PROVIDER = "snowflake"
DBT_NOVA_SNOWFLAKE_ACCOUNT = "myorg-myaccount"
DBT_NOVA_SNOWFLAKE_WAREHOUSE = "ANALYST_WH"
DBT_NOVA_SNOWFLAKE_DATABASE = "ANALYTICS"
DBT_NOVA_SNOWFLAKE_SCHEMA = "REPORTING"
DBT_NOVA_SNOWFLAKE_ROLE = "REPORTER"
DBT_NOVA_SNOWFLAKE_AUTH = "oauth"
DBT_NOVA_SNOWFLAKE_OAUTH_TOKEN = "<access-token>"
DBT_NOVA_EMBEDDINGS_CACHE_DIR = "/Users/<you>/.dbt-nova/.fastembed_cache"
```

External browser SSO example:

```toml
[mcp_servers.dbt-nova]
command = "/path/to/dbt-nova"
startup_timeout_sec = 60

[mcp_servers.dbt-nova.env]
DBT_MANIFEST_PATH = "/path/to/manifest.json"
DBT_NOVA_SQL_PROVIDER = "snowflake"
DBT_NOVA_SNOWFLAKE_ACCOUNT = "myorg-myaccount"
DBT_NOVA_SNOWFLAKE_USER = "you@example.com"
DBT_NOVA_SNOWFLAKE_WAREHOUSE = "ANALYST_WH"
DBT_NOVA_SNOWFLAKE_DATABASE = "ANALYTICS"
DBT_NOVA_SNOWFLAKE_SCHEMA = "REPORTING"
DBT_NOVA_SNOWFLAKE_ROLE = "REPORTER"
DBT_NOVA_SNOWFLAKE_AUTH = "externalbrowser"
DBT_NOVA_EMBEDDINGS_CACHE_DIR = "/Users/<you>/.dbt-nova/.fastembed_cache"
```

## DuckDB SQL Variables

Required:
- `DBT_NOVA_SQL_PROVIDER=duckdb`
- `DBT_NOVA_DUCKDB_PATH` (absolute path to a readable DuckDB file)

Optional:
- `DBT_NOVA_DUCKDB_FILE_SEARCH_PATH` (DuckDB `file_search_path` and `allowed_directories` bound used only with `DBT_NOVA_DUCKDB_ALLOW_EXTERNAL_ACCESS=true`)
- `DBT_NOVA_DUCKDB_ALLOW_EXTERNAL_ACCESS` (opt in to connection-level external access for trusted file-backed database objects under the configured search path; default `false`; ad-hoc file-scan functions in `execute_sql` text remain rejected)
- `DBT_NOVA_DUCKDB_POOL_MAX_SIZE` (max pooled DuckDB connections per `(duckdb_path,file_search_path,external_access)` key; defaults to `DBT_NOVA_SQL_MAX_CONCURRENT`, then `10`)
- `DBT_NOVA_SQL_MAX_ROW_LIMIT` (default: `10000`)
- `DBT_NOVA_SQL_MAX_BYTE_LIMIT` (default: `100000000`)
- `DBT_NOVA_SQL_MAX_CHUNKS` (default: `100`)
- `DBT_NOVA_SQL_MAX_POLL_SECONDS` (default: `900`)
- `DBT_NOVA_SQL_MIN_POLL_INTERVAL_MS` (default: `200`)
- `DBT_NOVA_SQL_MAX_CONCURRENT` (default: `10`)
- `DBT_NOVA_SQL_MAX_QUEUE` (default: `20`)
- `DBT_NOVA_SQL_QUEUE_TIMEOUT_MS` (default: `30000`)

DuckDB behavior notes:
- `parameter_types` is not supported; pass scalar values via `parameters`.
- Ad-hoc file-scan functions in `execute_sql` text are rejected even when connection-level external access is enabled.

### DuckDB Example (Codex CLI)

```toml
[mcp_servers.dbt-nova]
command = "/path/to/dbt-nova"
startup_timeout_sec = 60

[mcp_servers.dbt-nova.env]
DBT_MANIFEST_PATH = "/path/to/manifest.json"
DBT_NOVA_SQL_PROVIDER = "duckdb"
DBT_NOVA_DUCKDB_PATH = "/absolute/path/to/analytics.duckdb"
DBT_NOVA_DUCKDB_FILE_SEARCH_PATH = "/absolute/path/to/external/files"
DBT_NOVA_DUCKDB_ALLOW_EXTERNAL_ACCESS = "true"
DBT_NOVA_DUCKDB_POOL_MAX_SIZE = "10"
DBT_NOVA_EMBEDDINGS_CACHE_DIR = "/Users/<you>/.dbt-nova/.fastembed_cache"
```
