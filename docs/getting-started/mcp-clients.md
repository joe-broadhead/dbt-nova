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
`~/.dbt-nova/models`) so model downloads are reused across sessions/clients.
If you installed with:

```bash
curl -fsSL https://raw.githubusercontent.com/joe-broadhead/dbt-nova/master/scripts/install.sh | \
  DBT_NOVA_EMBEDDINGS_CACHE_DIR="$HOME/.dbt-nova/models" \
  DBT_NOVA_WARMUP_REQUIRED_MODELS=3 \
  bash -s -- --slim --warm-models --non-interactive
```

use that exact same `DBT_NOVA_EMBEDDINGS_CACHE_DIR` path in your MCP client env.

## Prebuilt Read-Only Consumer Setup

If you consume prebuilt Nova storage artifacts (built by the reusable producer
workflow), set these env vars in your MCP client:

- `DBT_NOVA_STORAGE_DIR` (local Nova storage root)
- `DBT_NOVA_STORAGE_READ_ONLY=true`
- `DBT_NOVA_BOOTSTRAP_URI` (recommended one-URI setup)
- `DBT_NOVA_ARTIFACT_FETCH_POLICY` (recommended: `if_missing`)

Optional explicit mode (if you do not use bootstrap URI):

- `DBT_NOVA_STORAGE_INSTANCE_ID` (must match producer workflow input)
- `DBT_NOVA_STORAGE_ARTIFACT_URI`
- `DBT_NOVA_METADATA_ARTIFACT_URI`
- `DBT_NOVA_MODELS_ARTIFACT_URI` (optional)

Recommended with prebuilt artifacts:

- Keep `DBT_MANIFEST_PATH` or `DBT_NOVA_MANIFEST_URI` pointed at the same
  manifest content used by the producer build.
- Use the same Nova release on producer and consumer.
- Keep artifact URIs stable and allow Nova to cache them locally.

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
DBT_NOVA_STORAGE_READ_ONLY = "true"
DBT_NOVA_BOOTSTRAP_URI = "s3://my-bucket/nova-assets/prod/analytics-prod-bootstrap.json"
DBT_NOVA_ARTIFACT_FETCH_POLICY = "if_missing"
DBT_NOVA_EMBEDDINGS_CACHE_DIR = "/Users/<you>/.dbt-nova/models"
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
        "DBT_NOVA_EMBEDDINGS_CACHE_DIR": "/Users/<you>/.dbt-nova/models",
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
DATABRICKS_HOST = "https://<workspace>.cloud.databricks.com"
DATABRICKS_HTTP_PATH = "/sql/1.0/warehouses/<warehouse_id>"
DATABRICKS_ACCESS_TOKEN = "<token>"
DBT_NOVA_EMBEDDINGS_CACHE_DIR = "/Users/<you>/.dbt-nova/models"
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
        "DBT_NOVA_EMBEDDINGS_CACHE_DIR": "/Users/<you>/.dbt-nova/models",
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
- `DBT_NOVA_SQL_MAX_ROW_LIMIT` (default: `10000`)
- `DBT_NOVA_SQL_MAX_BYTE_LIMIT` (default: `100000000`)
- `DBT_NOVA_SQL_MAX_CHUNKS` (default: `100`)
- `DBT_NOVA_SQL_MAX_POLL_SECONDS` (default: `900`)
- `DBT_NOVA_SQL_MIN_POLL_INTERVAL_MS` (default: `200`)
- `DBT_NOVA_SQL_MAX_CONCURRENT` (default: `10`)
- `DBT_NOVA_SQL_MAX_QUEUE` (default: `20`)
- `DBT_NOVA_SQL_QUEUE_TIMEOUT_MS` (default: `30000`)

## DuckDB SQL Variables

Required:
- `DBT_NOVA_SQL_PROVIDER=duckdb`
- `DBT_NOVA_DUCKDB_PATH` (absolute path to a readable DuckDB file)

Optional:
- `DBT_NOVA_DUCKDB_FILE_SEARCH_PATH` (DuckDB `file_search_path` for external file-backed objects/views)
- `DBT_NOVA_DUCKDB_POOL_MAX_SIZE` (max pooled DuckDB connections per `(duckdb_path,file_search_path)` key; defaults to `DBT_NOVA_SQL_MAX_CONCURRENT`, then `10`)
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
DBT_NOVA_DUCKDB_POOL_MAX_SIZE = "10"
DBT_NOVA_EMBEDDINGS_CACHE_DIR = "/Users/<you>/.dbt-nova/models"
```
