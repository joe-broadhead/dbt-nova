# MCP Client Configs

All MCP clients should set either `DBT_MANIFEST_PATH` or `DBT_NOVA_MANIFEST_URI`.
Databricks variables are required only if you use the `execute_sql` tool with
`DBT_NOVA_SQL_PROVIDER=databricks` (default).
BigQuery variables are required only if you use `DBT_NOVA_SQL_PROVIDER=bigquery`.
DuckDB variables are required only if you use `DBT_NOVA_SQL_PROVIDER=duckdb`.
For all SQL providers, object-level preflight checks (`preflight_catalog`,
`preflight_schema`, `preflight_relation`) pass only when the probe returns at
least one row.

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
- `DBT_NOVA_SQL_MAX_BYTE_LIMIT` (default: `25000000`)
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
- `DBT_NOVA_SQL_MAX_BYTE_LIMIT` (default: `25000000`)
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
- `DBT_NOVA_SQL_MAX_BYTE_LIMIT` (default: `25000000`)
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
