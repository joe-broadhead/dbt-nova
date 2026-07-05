# Configuration Reference

All configuration is via environment variables. Defaults are tuned for “high‑alpha”
search on real‑world manifests.

**Canonical defaults** are captured in `docs/config_defaults.json` and generated from
`src/config/` via `scripts/update_config_reference.sh`. When defaults change, regenerate
that file and ensure this page stays in sync.

For end-to-end install/runtime composition patterns (binary + manifest + artifacts + models + SQL provider),
see [Modes & Combinations](../getting-started/modes-and-combinations.md).

## Core

- `DBT_MANIFEST_PATH` – path to `manifest.json` (default: `manifest.json`)
- `DBT_NOVA_MANIFEST_URI` – optional manifest URI (`file://`, `http(s)://`, `dbfs://`, `s3://`, `gs://`)
- `DBT_NOVA_MANIFEST_CACHE_DIR` – optional local cache dir for remote manifests (default: `<storage_root>/manifests`)
- `DBT_NOVA_MANIFEST_REFRESH_SECS` – refresh interval for remote manifests (`0` = never refresh, default: `300`)
- `DBT_NOVA_MANIFEST_MAX_BYTES` – max bytes allowed for remote manifest fetches (`0` = unlimited, default: `268435456`)
- `DBT_NOVA_MANIFEST_HTTP_CONNECT_TIMEOUT_SECS` – HTTP connect timeout for manifest fetches (`0` = disabled, default: `10`)
- `DBT_NOVA_MANIFEST_HTTP_TIMEOUT_SECS` – HTTP request timeout for manifest fetches (`0` = disabled, default: `120`)
- `DBT_NOVA_MANIFEST_FETCH_TIMEOUT_SECS` – total fetch deadline for manifest fetches (`0` = disabled, default: `300`)
- `DBT_NOVA_MANIFEST_ALLOW_HTTP` – allow `http://` manifest URIs (`true`|`false`, default: `false`)
- `DBT_NOVA_PRUNE_ALLOW_IDS` – optional JSON array of dbt `unique_id` patterns to retain (exact or glob, default: `[]`)
- `DBT_NOVA_PRUNE_DENY_IDS` – optional JSON array of dbt `unique_id` patterns to exclude (exact or glob, default: `[]`; deny wins overlaps)
- `DBT_NOVA_STORAGE_ARTIFACT_URI` – optional URI to prebuilt storage archive (`file://`, `s3://`, `gs://`, `dbfs://`, `http(s)://`)
- `DBT_NOVA_METADATA_ARTIFACT_URI` – optional URI to prebuilt metadata contract JSON (required with `DBT_NOVA_STORAGE_ARTIFACT_URI`)
- `DBT_NOVA_MODELS_ARTIFACT_URI` – optional URI to prebuilt models archive
- `DBT_NOVA_BOOTSTRAP_URI` – optional URI to a bootstrap contract JSON that can populate `manifest_uri`, `storage_instance_id`, and prebuilt artifact URIs (same supported schemes as prebuilt artifact URIs)
- `DBT_NOVA_ARTIFACTS_CACHE_DIR` – optional cache dir for downloaded artifact archives (default: `<storage_root>/artifacts`)
- `DBT_NOVA_ARTIFACT_FETCH_POLICY` – artifact fetch policy (`if_missing`, `always`, `never`; default: `if_missing`; use `never` with `DBT_NOVA_STORAGE_READ_ONLY=true`, use `if_missing|always` for writable first-run hydration)
- `DBT_NOVA_ARTIFACT_TIMEOUT_SECS` – fetch timeout for remote artifact downloads (`0` = disabled, default: `300`)
- `DBT_NOVA_ARTIFACT_MAX_BYTES` – maximum compressed bytes allowed for each remote prebuilt artifact download (`0` = unlimited, default: `3221225472`)
- `DBT_NOVA_ARTIFACT_ARCHIVE_MAX_ENTRIES` – maximum number of entries allowed while extracting a prebuilt artifact archive (`0` = unlimited, default: `200000`)
- `DBT_NOVA_ARTIFACT_ARCHIVE_MAX_UNCOMPRESSED_BYTES` – maximum decompressed bytes allowed while extracting a prebuilt artifact archive (`0` = unlimited, default: `10737418240`)
- `DBT_NOVA_ARTIFACT_ALLOW_HTTP` – allow `http://` artifact URIs (`true`|`false`, default: `false`)
- `DBT_NOVA_SERVER_TRANSPORT` – MCP server transport (`stdio` or `streamable_http`, default: `stdio`)
- `DBT_NOVA_HTTP_HOST` – bind host for streamable HTTP mode (default: `127.0.0.1`; falls back to `0.0.0.0` when `PORT` is set and `DBT_NOVA_SERVER_TRANSPORT=streamable_http`)
- `DBT_NOVA_HTTP_PORT` – bind port for streamable HTTP mode (default: `8000`; falls back to `PORT` when unset)
- `DBT_NOVA_HTTP_PATH` – HTTP mount path for MCP requests in streamable HTTP mode (default: `/mcp`; reserved probe paths `/healthz` and `/readyz` are not allowed)
- `DBT_NOVA_HTTP_EXPECT_AUTH_PROXY` – required acknowledgement for non-loopback streamable HTTP binds (`true`|`false`, default: `false`; set to `true` only when an authenticating reverse proxy is enforcing access in front of dbt-nova)
- `DBT_NOVA_HTTP_ALLOWED_HOSTS` – comma-separated additional `Host` header values accepted by streamable HTTP mode (default: empty; loopback hosts are always allowed by the transport)
- `DBT_NOVA_HTTP_STATEFUL_MODE` – enable stateful streamable HTTP sessions (`true`|`false`, default: `true`)
- `DBT_NOVA_HTTP_SSE_KEEP_ALIVE_SECS` – SSE keepalive interval for streamable HTTP mode (`0` disables keepalives, default: `15`)
- `DBT_NOVA_HTTP_SSE_RETRY_SECS` – SSE retry hint for streamable HTTP mode (`0` disables retry hints, default: `3`)
- `DBT_NOVA_HTTP_MAX_BODY_BYTES` – global streamable HTTP request body cap (`0` disables the in-process cap, default: `16777216`)
- `DBT_NOVA_STRICT_SCHEMA` – fail build if schema files are missing or invalid (`true`|`false`, default: `false`; forced `true` in CI)
- `DBT_NOVA_S3_MODE` – S3 fetch mode (`https` or `sdk`, default: `https`)
- `DBT_NOVA_GCS_MODE` – GCS fetch mode (`https` or `sdk`, default: `https`)
- `DBT_NOVA_RECIPES_DIR` – manifest `original_file_path` prefix used to discover recipe `analysis` nodes (default: `analyses/recipes`). Recipe SQL is resolved from manifest `compiled_code` (or `raw_code` fallback). Recipes are documented in [Analysis Recipes](../features/recipes.md).
- `DBT_NOVA_LOG` / `RUST_LOG` – enable structured logs to stderr (e.g., `info`, `debug`, `trace`)
- `DBT_NOVA_DISABLE_TOOL_SCHEMAS` – strip JSON schema hints from MCP tools (useful for strict clients like Gemini; see [MCP Clients](../getting-started/mcp-clients.md))
- `DBT_NOVA_TOOL_ALLOWLIST` – optional comma-separated allowlist of exact MCP tool names to expose; when set, only these tools are eligible for exposure
- `DBT_NOVA_TOOL_DENYLIST` – optional comma-separated denylist of exact MCP tool names to hide after allowlist processing
- `DBT_NOVA_RESULT_PROFILE` – default detail profile when CLI/tool-call requests omit `detail` (`compact`, `standard`, or `full`; default: `standard`)
- `DBT_NOVA_MCP_RESULT_PROFILE` – default detail profile when MCP requests omit `detail` (`compact`, `standard`, or `full`; default: `compact`)
- `DBT_NOVA_MCP_DEFAULT_LIMIT` – MCP result limit used when paginated MCP requests omit `limit` or pass `limit=0` (default: `10`)
- `DBT_NOVA_MCP_MAX_PAGE_SIZE` – MCP-specific cap for paginated result requests before the global search cap is applied (`0` disables the MCP-specific cap, default: `100`)
- `DBT_NOVA_MCP_MAX_RESPONSE_BYTES` – central serialized MCP tool response budget in bytes (`0` disables central budgeting, default: `65536`)
- `DBT_NOVA_MCP_MAX_STRING_CHARS` – max characters retained for long strings when central MCP budgeting truncates (default: `4096`)
- `DBT_NOVA_MCP_INCLUDE_TRUNCATION_META` – include `_nova_result_meta` when a central MCP budget pass truncates a response (`true`|`false`, default: `true`)
- `DBT_NOVA_MCP_ENABLE_MANIFEST_RELOAD` – allow MCP `reload_manifest` to change `manifest_uri`, `manifest_path`, `refresh_secs`, or `storage_instance_id`; no-argument current-source reloads do not require this opt-in
- `DBT_NOVA_MCP_ENABLE_MANIFEST_WARM` – allow MCP/`tool call` `warm_manifest` semantic cache writes
- `DBT_NOVA_SQL_PROVIDER` – SQL backend for `execute_sql` (`databricks`, `bigquery`, `snowflake`, or `duckdb`, default: `databricks`)
- `DBT_NOVA_GCP_PROJECT_ID` – shared Google project id alias (used by BigQuery fallback resolution)
- `DBT_NOVA_GCP_ACCESS_TOKEN` – shared Google OAuth access token alias (used by BigQuery fallback resolution)
- `DBT_NOVA_BIGQUERY_PROJECT_ID` – BigQuery project id when `DBT_NOVA_SQL_PROVIDER=bigquery` (falls back to `DBT_NOVA_GCP_PROJECT_ID`, `GOOGLE_CLOUD_PROJECT`, `GCP_PROJECT_ID`)
- `DBT_NOVA_BIGQUERY_ACCESS_TOKEN` – OAuth access token for BigQuery when `DBT_NOVA_SQL_PROVIDER=bigquery` (falls back to `DBT_NOVA_GCP_ACCESS_TOKEN`, `GCP_ACCESS_TOKEN`, `GOOGLE_OAUTH_ACCESS_TOKEN`, `GOOGLE_APPLICATION_CREDENTIALS`, or gcloud ADC)
- `DBT_NOVA_BIGQUERY_LOCATION` – optional BigQuery location for `execute_sql` and provider preflight
- `DBT_NOVA_BIGQUERY_TIMEOUT_MS` – HTTP timeout for BigQuery API requests (default: `30000`)
- `DBT_NOVA_BIGQUERY_TOKEN_CACHE_TTL_SECS` – cache TTL for BigQuery auth token + HTTP client reuse (default: `3000`, minimum: `60`)
- `DBT_NOVA_BIGQUERY_API_BASE_URL` – advanced/test override for the BigQuery API origin (default: `https://bigquery.googleapis.com`; `http://` is accepted only for loopback test servers)
- `DBT_NOVA_SNOWFLAKE_ACCOUNT` – Snowflake account identifier when `DBT_NOVA_SQL_PROVIDER=snowflake`; used to build `https://<account>.snowflakecomputing.com`
- `DBT_NOVA_SNOWFLAKE_ACCOUNT_URL` – optional explicit Snowflake account root URL for SQL API calls; use `https://<host>`, not an `/api` path
- `DBT_NOVA_SNOWFLAKE_WAREHOUSE` – Snowflake warehouse used by `execute_sql` and preflight
- `DBT_NOVA_SNOWFLAKE_DATABASE` – optional default Snowflake database
- `DBT_NOVA_SNOWFLAKE_SCHEMA` – optional default Snowflake schema
- `DBT_NOVA_SNOWFLAKE_ROLE` – optional default Snowflake role
- `DBT_NOVA_SNOWFLAKE_AUTH` – Snowflake auth mode (`keypair`, `oauth`, `pat`, or `externalbrowser`; default: inferred from provided token variables, otherwise `keypair`)
- `DBT_NOVA_SNOWFLAKE_USER` – Snowflake user for key-pair or external browser auth
- `DBT_NOVA_SNOWFLAKE_JWT_ACCOUNT` – account identifier override for key-pair JWT claims; required when key-pair auth uses `DBT_NOVA_SNOWFLAKE_ACCOUNT_URL` without `DBT_NOVA_SNOWFLAKE_ACCOUNT`. JWT account identifiers are uppercased, periods are replaced with hyphens, legacy locator-style region suffixes are excluded, and fully qualified organization/account names are preserved.
- `DBT_NOVA_SNOWFLAKE_PRIVATE_KEY_PATH` – path to an unencrypted RSA private key PEM for key-pair auth
- `DBT_NOVA_SNOWFLAKE_PRIVATE_KEY_PEM` – inline unencrypted RSA private key PEM for key-pair auth (`\n` escapes are accepted)
- `DBT_NOVA_SNOWFLAKE_OAUTH_TOKEN` – OAuth bearer token for `DBT_NOVA_SNOWFLAKE_AUTH=oauth`
- `DBT_NOVA_SNOWFLAKE_PAT` – programmatic access token for `DBT_NOVA_SNOWFLAKE_AUTH=pat`
- `DBT_NOVA_SNOWFLAKE_EXTERNAL_BROWSER_TIMEOUT_S` – local browser SSO timeout for `DBT_NOVA_SNOWFLAKE_AUTH=externalbrowser` (default: `120`)
- `DBT_NOVA_SNOWFLAKE_EXTERNAL_BROWSER_OPEN` – open the system browser automatically for external browser auth (`true`|`false`, default: `true`; when false, Nova prints the SSO URL for manual opening)
- `DBT_NOVA_SNOWFLAKE_EXTERNAL_BROWSER_CALLBACK_PORT` – optional fixed loopback callback port for external browser auth (default: bind an ephemeral `127.0.0.1` port)
- `DBT_NOVA_SNOWFLAKE_TIMEOUT_MS` – HTTP timeout for Snowflake SQL API requests (default: `30000`)
- `DBT_NOVA_SNOWFLAKE_STATEMENT_TIMEOUT_S` – Snowflake statement timeout in seconds when caller omits `wait_timeout_s` (default: `60`; `0` requests Snowflake SQL API's maximum timeout window)
- `DBT_NOVA_SNOWFLAKE_POLL_INTERVAL_MS` – provider default polling interval (default: `1000`)
- `DBT_NOVA_SNOWFLAKE_MAX_POLL_SECONDS` – provider default polling duration before local cancellation (default: `600`)
- `DBT_NOVA_SNOWFLAKE_MAX_CHUNKS` – provider default max result partitions fetched (default: `50`)
- `DBT_NOVA_DUCKDB_PATH` – required DuckDB database file when `DBT_NOVA_SQL_PROVIDER=duckdb`
- `DBT_NOVA_DUCKDB_FILE_SEARCH_PATH` – optional DuckDB `file_search_path` and `allowed_directories` bound used only when `DBT_NOVA_DUCKDB_ALLOW_EXTERNAL_ACCESS=true`
- `DBT_NOVA_DUCKDB_ALLOW_EXTERNAL_ACCESS` – opt in to connection-level DuckDB external access for trusted file-backed database objects under the configured `DBT_NOVA_DUCKDB_FILE_SEARCH_PATH` (`true`|`false`, default: `false`); ad-hoc file-scan functions in `execute_sql` text remain rejected
- `DBT_NOVA_DUCKDB_POOL_MAX_SIZE` – optional max pooled DuckDB connections per `(duckdb_path,file_search_path,external_access)` key (default: falls back to `DBT_NOVA_SQL_MAX_CONCURRENT`, then `10`)
- `DATABRICKS_HOST` – Databricks workspace URL for `dbfs://` manifests and `execute_sql`
- `DATABRICKS_ACCESS_TOKEN` – Databricks access token for `dbfs://` and `execute_sql`

For auth details by source, see `docs/configuration/manifest-sources.md`.

Remote manifest notes:
- `s3://` and `gs://` are fetched over HTTPS by default (public or presigned URLs).
- Optional overrides: `DBT_NOVA_S3_ENDPOINT`, `DBT_NOVA_GCS_ENDPOINT`.
- `dbfs://` requires `DATABRICKS_HOST` + `DATABRICKS_ACCESS_TOKEN`.
- To use SDK credentials, set `DBT_NOVA_S3_MODE=sdk` or `DBT_NOVA_GCS_MODE=sdk` (SDKs are included by default builds).
- To force HTTPS for public/presigned URLs, set `DBT_NOVA_S3_MODE=https` or `DBT_NOVA_GCS_MODE=https`.
- To allow insecure `http://` manifests (not recommended), set `DBT_NOVA_MANIFEST_ALLOW_HTTP=true`.
- To allow insecure `http://` prebuilt artifact URIs (not recommended), set `DBT_NOVA_ARTIFACT_ALLOW_HTTP=true`.
- Remote artifact downloads and extraction are bounded by `DBT_NOVA_ARTIFACT_MAX_BYTES`, `DBT_NOVA_ARTIFACT_ARCHIVE_MAX_ENTRIES`, and `DBT_NOVA_ARTIFACT_ARCHIVE_MAX_UNCOMPRESSED_BYTES`.
- Bootstrap precedence is deterministic: explicit env vars override bootstrap values, and bootstrap values override defaults.
- Tool filtering precedence is deterministic: allowlist is applied first, then denylist; denylist wins when both include the same tool.
- Tool filter names are strict and case-sensitive. Unknown names in either list fail startup validation with a hard error.
- Result profile defaults only fill omitted `detail` values. Callers can still
  request `detail=standard` or `detail=full` explicitly when they need richer
  payloads. With the default compact MCP profile, omitted `search_indicator`
  group output defaults to `group_mode=top`; explicit `group_mode=all` remains
  available for debugging.
- Central MCP response budgeting is a backstop, not a replacement for tool-level
  limits. Prefer compact tool parameters first; truncated responses include
  `_nova_result_meta` with byte budget, omitted path evidence, and `next_offset`
  for paginated MCP responses when enabled.
- Tool filter examples:
  - Allowlist only (expose only discovery + entity lookup):
    - `DBT_NOVA_TOOL_ALLOWLIST=search,get_entity`
  - Denylist only (hosted discovery-only and non-admin posture):
    - `DBT_NOVA_TOOL_DENYLIST=execute_sql,run_recipe,reload_manifest,show_config,validate_config,inspect_storage,prune_storage,cleanup_storage,warm_manifest`
  - Deny operator/admin tools for normal agent clients:
    - `DBT_NOVA_TOOL_DENYLIST=show_config,validate_config,inspect_storage,prune_storage,cleanup_storage`
  - Combined with denylist precedence (effective exposed set: `search`):
    - `DBT_NOVA_TOOL_ALLOWLIST=search,execute_sql`
    - `DBT_NOVA_TOOL_DENYLIST=execute_sql`
- Source-changing `reload_manifest` calls require
  `DBT_NOVA_MCP_ENABLE_MANIFEST_RELOAD=1`; no-argument reloads refresh the
  current source.
- Destructive storage admin tools (`prune_storage`, `cleanup_storage`) are also
  disabled by default and require `DBT_NOVA_MCP_ENABLE_STORAGE_ADMIN=1`.
- Published container images default to `DBT_NOVA_TOOL_DENYLIST=execute_sql,run_recipe,reload_manifest,show_config,validate_config,inspect_storage,prune_storage,cleanup_storage,warm_manifest` so hosted image starts are discovery-only and non-admin unless an operator clears or customizes the denylist.
- Streamable HTTP mode has **no built-in authentication**. Keep it bound to loopback for local use, or set `DBT_NOVA_HTTP_EXPECT_AUTH_PROXY=true` only when an authenticating reverse proxy is enforcing access in front of dbt-nova. For hosted/proxied deployments, set `DBT_NOVA_HTTP_ALLOWED_HOSTS` to the public/proxy hostnames clients send in `Host`. Published container images do not set these acknowledgements by default.
- Streamable HTTP mode applies a global request body cap before the mounted MCP transport reads request bodies. Keep `DBT_NOVA_HTTP_MAX_BODY_BYTES` bounded in hosted deployments unless an outer proxy enforces a stricter limit.
- The MCP endpoint is mounted at `DBT_NOVA_HTTP_PATH`; plain probe endpoints are always available at `/healthz` and `/readyz`.

Manifest pruning notes:
- Matching is against dbt `unique_id` (not `fqn`).
- Prune variables must be valid JSON arrays of strings; invalid JSON fails config validation and server startup.
- If `DBT_NOVA_PRUNE_ALLOW_IDS` is empty, pruning starts from all entities, then applies deny rules.
- `analysis` nodes are auto-included only when they directly depend on retained nodes and have no extra direct dependencies outside the retained set.
- To generate a valid allow-list from dbt selectors, use `dbt ls` JSON output and extract `unique_id`:

```bash
export DBT_NOVA_PRUNE_ALLOW_IDS="$(
  dbt ls -s <lineage selection expression> --output json --quiet \
  | jq -cs '[.[] | .unique_id]'
)"
```

## Storage

!!! warning "Data Loss Risk"
    Setting `DBT_NOVA_CLEANUP_STORAGE_ON_START=true` deletes all cached indexes on startup.
    Use only for development or when you need a clean slate.

- `DBT_NOVA_STORAGE_DIR` – base directory for on‑disk storage (default: `.dbt-nova`)
- `DBT_NOVA_STORAGE_INSTANCE_ID` – optional instance id (auto‑generated when unset)
- `DBT_NOVA_CLEANUP_STORAGE_ON_START` – delete instance dir on startup (`true`|`false`, default: `false`)
- `DBT_NOVA_STORAGE_MAX_INSTANCES` – max instance dirs to retain (`0` = unlimited, default: `3`)
- `DBT_NOVA_STORAGE_MIN_VERSIONS` – minimum manifest versions retained per instance (default: `2`)
- `DBT_NOVA_STORAGE_MAX_BYTES` – max total bytes across instances (`0` = unlimited, default: `5368709120`)
- `DBT_NOVA_STORAGE_BUILD_LOCK_WAIT_SECS` – max seconds to wait for another process to finish building (default: `300`)
- `DBT_NOVA_STORAGE_READ_ONLY` – do not build indexes or materialize prebuilt artifacts locally (`true`|`false`, default: `false`; incompatible with cold-start bootstrap/artifact hydration)
- `DBT_NOVA_ENTITY_CACHE_SIZE` – max entities cached in memory (`0` disables, default: `1000`)
- `DBT_NOVA_EMBEDDINGS_CACHE_DIR` – embeddings cache directory (default: `models/` next to executable if present, else `~/.dbt-nova/.fastembed_cache`)

Embeddings cache resolution order when `DBT_NOVA_EMBEDDINGS_CACHE_DIR` is unset:

1. `models/` next to the active executable
2. `~/.local/bin/models` (if present)
3. `~/.dbt-nova/.fastembed_cache`

Notes:
- The entity cache backend is currently fixed to `moka` (no env override).

Instance directories live under `<storage_root>/instances/<instance_id>`.

## Safety Limits

- `DBT_NOVA_BATCH_GET_MAX_ITEMS` – max ids accepted by `batch_get_entities` (`0` = unlimited, default: `5000`)
- `DBT_NOVA_MAX_PAGE_SIZE` – max results per page (default: `2000`)
- `DBT_NOVA_MAX_OFFSET` – max pagination offset (default: `10000`)
- `DBT_NOVA_MAX_QUERY_LENGTH` – max search query length (default: `2000`)
- `DBT_NOVA_MAX_PATH_PATTERN_LENGTH` – max path pattern length (default: `1000`)
- `DBT_NOVA_MAX_SQL_CHUNK_BYTES` – max bytes of SQL indexed per field (default: `262144`)
- `DBT_NOVA_EMBEDDINGS_MAX_DECOMPRESSED_BYTES` – max decompressed bytes for embedding caches (`0` = unlimited, default: `4294967296`)
- `DBT_NOVA_TOOL_RATE_LIMITS` – per-tool rate limits (default: `search=60,execute_sql=20,default=120`)
- `DBT_NOVA_TOOL_RATE_LIMIT_WINDOW_SECS` – rate limit window seconds (default: `60`)
- `DBT_NOVA_MCP_DEFAULT_LIMIT` – MCP default result limit when omitted or `0` (default: `10`)
- `DBT_NOVA_MCP_MAX_PAGE_SIZE` – MCP-specific max result page size (`0` disables, default: `100`)
- `DBT_NOVA_MCP_MAX_RESPONSE_BYTES` – max serialized MCP response bytes before deterministic truncation (`0` disables, default: `65536`)
- `DBT_NOVA_MCP_MAX_STRING_CHARS` – max long-string characters retained during MCP truncation (default: `4096`)
- `DBT_NOVA_SQL_MAX_ROW_LIMIT` – max rows accepted by `execute_sql` (`0` = unlimited, default: `10000`)
- `DBT_NOVA_SQL_MAX_BYTE_LIMIT` – max bytes accepted by `execute_sql` (`0` = unlimited, default: `100000000`)
- `DBT_NOVA_SQL_MAX_CHUNKS` – max result chunks accepted by `execute_sql` (`0` = unlimited, default: `100`)
- `DBT_NOVA_SQL_MAX_POLL_SECONDS` – max polling duration accepted by `execute_sql` (`0` = unlimited, default: `900`)
- `DBT_NOVA_SQL_MIN_POLL_INTERVAL_MS` – minimum poll interval accepted by `execute_sql` (`0` disables floor, default: `200`)
- `DBT_NOVA_SQL_MAX_CONCURRENT` – max concurrent SQL executions across `execute_sql` and `run_recipe` (`0` = unlimited, default: `10`)
- `DBT_NOVA_SQL_MAX_QUEUE` – max queued SQL executions while all slots are busy (default: `20`)
- `DBT_NOVA_SQL_QUEUE_TIMEOUT_MS` – max wait time for a SQL execution slot (`0` disables timeout, default: `30000`)

## Lineage

- `DBT_NOVA_DEFAULT_CONFIDENCE` – column lineage confidence (`high`|`medium`|`low`, default: `medium`)
- `DBT_NOVA_LEVENSHTEIN_THRESHOLD` – similarity threshold (default: `0.75`)
- `DBT_NOVA_MIN_PREFIX_SUFFIX_LENGTH` – min length for prefix/suffix matches (default: `2`)
- `DBT_NOVA_MIN_LEVENSHTEIN_LENGTH` – min length to run Levenshtein (default: `3`)
- `DBT_NOVA_SQL_PROXIMITY_MAX_DISTANCE` – max SQL proximity distance (default: `100`)
- `DBT_NOVA_MAX_LINEAGE_RESULTS` – max column lineage results (default: `10000`)
- `DBT_NOVA_COLUMN_LINEAGE_MAX_DEPTH` – max column lineage depth (default: `100`)
- `DBT_NOVA_COLUMN_LINEAGE_MAX_CANDIDATES` – max column match candidates before capping (default: `10000`)
- `DBT_NOVA_COLUMN_LINEAGE_PRECOMPUTE` – precompute SQL aliases for column lineage (`true`|`false`, default: `true`)
- `DBT_NOVA_MAX_ENTITY_LINEAGE_RESULTS` – max entity lineage results (default: `10000`)
- `DBT_NOVA_MAX_LINEAGE_DEPTH` – max entity lineage depth (default: `200`)
- `DBT_NOVA_LINEAGE_CACHE_SIZE` – cache size for entity lineage responses (`0` disables, default: `2048`)

Note: `DBT_NOVA_MAX_LINEAGE_RESULTS` applies to **column lineage** (`get_column_lineage`), while
`DBT_NOVA_MAX_ENTITY_LINEAGE_RESULTS` applies to **entity lineage** (`get_lineage`).

## Tantivy / Lexical Search

- `DBT_NOVA_DEFAULT_LIMIT` – fallback result limit when `limit` is omitted or `limit=0` (default: `50`)
- `DBT_NOVA_MIN_WORD_LENGTH` – minimum word length for indexing (default: `2`)
- `DBT_NOVA_INDEX_DIR` – Tantivy index directory name inside storage (default: `index`)
- `DBT_NOVA_INDEX_WRITER_HEAP_BYTES` – Tantivy writer heap size (default: `128000000`)
- `DBT_NOVA_SEARCH_DEDUP_FETCH_MULTIPLIER` – fetch multiplier for de‑duping (default: `8`)
- `DBT_NOVA_SEARCH_ENABLE_NGRAM` – enable n‑gram indexing (default: `true`)
- `DBT_NOVA_SEARCH_NGRAM_MIN` – min n‑gram size (default: `3`)
- `DBT_NOVA_SEARCH_NGRAM_MAX` – max n‑gram size (default: `3`)
- `DBT_NOVA_SEARCH_NGRAM_BOOST` – n‑gram boost (default: `0.35`)
- `DBT_NOVA_FUZZY_MIN_LENGTH` – min term length for fuzzy (default: `4`)
- `DBT_NOVA_FUZZY_MID_LENGTH` – medium fuzzy threshold (default: `7`)
- `DBT_NOVA_FUZZY_MAX_DISTANCE` – max edit distance (default: `2`)
- `DBT_NOVA_SEARCH_HIGHLIGHT_MAX_CHARS` – max snippet length (default: `240`)
- `DBT_NOVA_SEARCH_HIGHLIGHT_MAX_FIELDS` – max highlighted fields per result (default: `5`)
- `DBT_NOVA_SEARCH_HIGHLIGHT_FORMAT` – `text` or `html` (default: `text`)
- `DBT_NOVA_SEARCH_ENABLE_SUGGESTIONS` – enable suggestions (default: `true`)
- `DBT_NOVA_SEARCH_SUGGESTIONS_LIMIT` – max suggestions (default: `7`)
- `DBT_NOVA_SEARCH_ENABLE_RRF` – enable RRF fusion (default: `true`)
- `DBT_NOVA_SEARCH_RRF_K` – RRF smoothing constant (default: `60`)
- `DBT_NOVA_SEARCH_RRF_OVERFETCH` – overfetch multiplier (default: `3`)
- `DBT_NOVA_SEARCH_TIMEOUT_MS` – search timeout in ms (`0` disables, default: `30000`)
- `DBT_NOVA_SEARCH_MAX_CONCURRENT` – max concurrent search requests (`0` = unlimited, default: `4`)
- `DBT_NOVA_SEARCH_MAX_QUEUE` – max queued searches when saturated (default: `8`)

## Extended Metadata Search

Extended metadata search config is default-off. It only describes an explicit
allowlist of non-Nova dbt metadata paths that Nova extracts into dedicated
search fields. No extended metadata is indexed when `fields` is empty.

- `DBT_NOVA_SEARCH_EXTENDED_META_FIELDS_JSON` – JSON array of allowlisted non-Nova dbt metadata fields (default: `[]`)
- `DBT_NOVA_SEARCH_EXTENDED_META_MAX_FIELDS` – max configured fields (default: `32`, hard cap: `128`)
- `DBT_NOVA_SEARCH_EXTENDED_META_MAX_VALUES_PER_FIELD` – max values retained per field (default: `64`, hard cap: `1024`)
- `DBT_NOVA_SEARCH_EXTENDED_META_MAX_BYTES_PER_VALUE` – max bytes retained per value (default: `4096`, hard cap: `65536`)

Each field object accepts:
- `path` – logical dbt metadata path beginning with `meta.` or `columns.*.meta.`
- `alias` – lowercase ASCII field alias exposed as `meta.<alias>` for fielded search
- `mode` – one of `keyword`, `text`, `string_array`, or `bool`
- `boost` – non-negative ranking boost (default: `1.0`)
- `summary` – whether standard/full search rows include the field in
  `extended_meta_summary` (default: `false`)

Example:

```bash
export DBT_NOVA_SEARCH_EXTENDED_META_FIELDS_JSON='[
  {"path":"meta.owner","alias":"owner","mode":"keyword","boost":1.25,"summary":true},
  {"path":"columns.*.meta.semantic_group","alias":"semantic_group","mode":"string_array"}
]'
```

With that config, unfielded searches can match allowlisted values, and fielded
queries can target the configured aliases:

```text
meta.owner:alice
meta.semantic_group:lifecycle
```

Because `meta.owner` has `summary: true`, matching standard/full search rows can
also include a compact summary:

```json
{
  "extended_meta_summary": {
    "fields": [
      {
        "alias": "owner",
        "path": "meta.owner",
        "search_field": "meta.owner",
        "mode": "keyword",
        "values": ["alice"]
      }
    ]
  }
}
```

Guardrails:
- No fields preserves current behavior and produces no search-index fingerprint.
- `meta.nova` paths are rejected because Nova metadata is already indexed.
- Sensitive key segments are rejected before indexing: `token`, `secret`, `password`, `credential`, `private_key`, and `api_key`.
- `*` is only accepted in `columns.*.meta.` paths; runtime schema discovery is not performed.
- Values are capped deterministically by `max_values_per_field` and
  `max_bytes_per_value`; excess values are dropped during indexing and summary
  rendering. Summary fields set `truncated: true` when values were dropped or
  byte-truncated, with `dropped_values` and `byte_truncated_values` counts when
  applicable.
- Changing extended metadata config changes the manifest-scoped search index identity.

## Embeddings (Dense + Sparse)

!!! warning "High Memory Usage"
    Enabling dense vectors (`DBT_NOVA_SEARCH_ENABLE_VECTOR=true`) requires ~2 GB RAM
    for embeddings. Disable on memory-constrained systems.

- Semantic layers are disabled by default. Opt in explicitly with
  `DBT_NOVA_SEARCH_ENABLE_VECTOR=true`,
  `DBT_NOVA_SEARCH_ENABLE_SPARSE=true`, and/or
  `DBT_NOVA_SEARCH_ENABLE_RERANKER=true` after warming model files and, for
  vector/sparse, manifest-scoped caches.

- `DBT_NOVA_SEARCH_ENABLE_VECTOR` – enable dense vectors (default: `false`)
- `DBT_NOVA_SEARCH_COLD_START_POLICY` – semantic cache behavior when
  manifest-scoped caches are missing: `degrade` skips semantic startup work,
  `build` creates missing caches during startup (default: `degrade`)
- `DBT_NOVA_SEARCH_VECTOR_TOP_K` – max vector hits before fusion (default: `200`)
- `DBT_NOVA_SEARCH_VECTOR_MAX_CHARS` – max chars in embedding text (default: `4000`)
- `DBT_NOVA_SEARCH_ENABLE_VECTOR_ANN` – enable ANN buckets (default: `true`)
- `DBT_NOVA_SEARCH_ENABLE_VECTOR_QUANTIZATION` – enable 8‑bit quantization (default: `false`)
- `DBT_NOVA_SEARCH_VECTOR_ANN_BITS` – ANN hash bits (default: `16`)
- `DBT_NOVA_SEARCH_VECTOR_ANN_HAMMING` – ANN Hamming radius (default: `1`)
- `DBT_NOVA_SEARCH_VECTOR_ANN_MAX_CANDIDATES` – max ANN candidates (default: `5000`)
- `DBT_NOVA_SEARCH_VECTOR_ANN_MIN_CANDIDATES` – min before full scan (default: `200`)
- `DBT_NOVA_SEARCH_ONNX_THREADS` – ONNX intra-thread count for vector, sparse,
  and reranker models (default: min available parallelism, capped at `4`)
- `DBT_NOVA_SEARCH_EMBEDDING_BATCH_SIZE` – embedding batch size (default: `128`)
- `DBT_NOVA_EMBEDDING_MODEL` – embedding model name (default: `intfloat/multilingual-e5-base`)
- `DBT_NOVA_SEARCH_ENABLE_SPARSE` – enable sparse vectors (default: `false`)
- `DBT_NOVA_SEARCH_SPARSE_TOP_K` – max sparse hits before fusion (default: `200`)
- `DBT_NOVA_SEARCH_SPARSE_EMBEDDING_BATCH_SIZE` – sparse embedding batch size
  (default: `16`; falls back to `DBT_NOVA_SEARCH_EMBEDDING_BATCH_SIZE` only
  when the sparse-specific variable is unset)
- `DBT_NOVA_SEARCH_ENABLE_RERANKER` – enable cross‑encoder reranker (default: `false`)
- `DBT_NOVA_RERANKER_MODEL` – reranker model (default: `jinaai/jina-reranker-v2-base-multilingual`)
- `DBT_NOVA_SEARCH_RERANK_TOP_N` – max results reranked (default: `20`)

Proxy validation:
- `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY` (and lowercase variants) must be absolute URLs when set.
- Invalid proxy values fail fast during embeddings/reranker initialization.

## Persona Defaults + Overrides

- `DBT_NOVA_SEARCH_DEFAULT_PERSONA` – default persona if request omits `persona`
- `DBT_NOVA_SEARCH_PERSONA_ANALYST_WEIGHTS` – comma‑separated overrides (e.g. `vector=1.6,docs=1.3`)
- `DBT_NOVA_SEARCH_PERSONA_ENGINEER_WEIGHTS` – overrides for engineer persona
- `DBT_NOVA_SEARCH_PERSONA_GOVERNANCE_WEIGHTS` – overrides for governance persona
- `DBT_NOVA_SEARCH_PERSONA_DEFAULT_WEIGHTS` – overrides for default persona

Aliases: `docs` and `documentation` are both accepted in persona weight overrides.

Analyst semantic multipliers (advanced tuning):
- `DBT_NOVA_SEARCH_ANALYST_METRIC_DEF_MULTIPLIER` (default: `1.09`)
- `DBT_NOVA_SEARCH_ANALYST_MEASURE_DEF_MULTIPLIER` (default: `1.05`)
- `DBT_NOVA_SEARCH_ANALYST_GRAIN_MULTIPLIER` (default: `1.05`)
- `DBT_NOVA_SEARCH_ANALYST_TIME_FIELD_MULTIPLIER` (default: `1.03`)
- `DBT_NOVA_SEARCH_ANALYST_DIM_OVERLAP_ONE_MULTIPLIER` (default: `1.03`)
- `DBT_NOVA_SEARCH_ANALYST_DIM_OVERLAP_TWO_MULTIPLIER` (default: `1.06`)
- `DBT_NOVA_SEARCH_ANALYST_DIM_OVERLAP_THREE_PLUS_MULTIPLIER` (default: `1.09`)
- `DBT_NOVA_SEARCH_ANALYST_MISSING_METRIC_MEASURE_MULTIPLIER` (default: `0.96`)
- `DBT_NOVA_SEARCH_ANALYST_MISSING_GRAIN_MULTIPLIER` (default: `0.97`)
- `DBT_NOVA_SEARCH_ANALYST_MIN_MULTIPLIER` (default: `0.85`)
- `DBT_NOVA_SEARCH_ANALYST_MAX_MULTIPLIER` (default: `1.35`)

## Metadata Scoring

Metadata scoring uses the weight defaults under `metadata_score.persona_weights` in
`docs/config_defaults.json` (sourced from `src/config/metadata_score.rs`). There are
currently no environment overrides for these weights.

## Governance Gate Policy

- `DBT_NOVA_GOV_GATE_PROFILE` – preset governance gate policy (`strict`|`standard`|`advisory`)
- `DBT_NOVA_GOV_GATE_POLICY` – full policy override as JSON
- `DBT_NOVA_GOV_GATE_MIN_METADATA_SCORE` – minimum metadata score threshold (`0..=100`)
- `DBT_NOVA_GOV_GATE_MIN_DOC_COVERAGE_PCT` – minimum documentation coverage threshold (`0..=100`)
- `DBT_NOVA_GOV_GATE_REQUIRE_TESTS` – require tests to pass governance gate (`true`|`false`)
- `DBT_NOVA_GOV_GATE_REQUIRE_OWNER` – require owner metadata to pass governance gate (`true`|`false`)
- `DBT_NOVA_GOV_GATE_REQUIRE_REQUIRED_FIELDS` – enforce `DBT_NOVA_GOV_REQUIRED_FIELDS` checks (`true`|`false`)
- `DBT_NOVA_GOV_GATE_REQUIRE_COMPLIANCE_FOR_PII` – require compliance tags when PII is declared (`true`|`false`)
- `DBT_NOVA_GOV_GATE_BLOCK_ON_FAILURE` – return blocking `fail` status on gate failure (`true`), or advisory-only status (`false`)

## Provenance

- `DBT_NOVA_PROVENANCE_STALE_AFTER_DAYS` – days after which source freshness or manifest-generated timestamps are marked `stale` in search, context, and lineage `provenance` blocks (default: `30`)

## Circuit Breakers

- `DBT_NOVA_SEARCH_CIRCUIT_FAILURE_THRESHOLD` – failures before opening (default: `3`)
- `DBT_NOVA_SEARCH_CIRCUIT_OPEN_SECONDS` – open duration seconds (default: `60`)

## Search Boosts & Nova Ranking

These variables control lexical boosts and Nova‑specific ranking multipliers.
Full rationale and defaults are documented in `docs/features/search-ranking.md`.

Base field boosts:
- `DBT_NOVA_ALIAS_BOOST` (default: `18.0`)
- `DBT_NOVA_NAME_BOOST` (default: `12.0`)
- `DBT_NOVA_DESCRIPTION_BOOST` (default: `6.0`)
- `DBT_NOVA_COLUMN_BOOST` (default: `4.0`)
- `DBT_NOVA_TAG_BOOST` (default: `3.0`)
- `DBT_NOVA_PATH_BOOST` (default: `2.0`)
- `DBT_NOVA_CODE_BOOST` (default: `1.5`)

Nova meta boosts:
- `DBT_NOVA_META_SYNONYMS_BOOST` (default: `7.0`)
- `DBT_NOVA_META_DOMAINS_BOOST` (default: `4.0`)
- `DBT_NOVA_META_USE_CASES_BOOST` (default: `4.0`)
- `DBT_NOVA_META_MEASURES_BOOST` (default: `8.0`)
- `DBT_NOVA_META_METRIC_BOOST` (default: `10.0`)
- `DBT_NOVA_META_SENSITIVITY_BOOST` (default: `6.0`)
- `DBT_NOVA_META_PII_BOOST` (default: `8.0`)
- `DBT_NOVA_META_COMPLIANCE_BOOST` (default: `6.0`)

Post‑retrieval tuning:
- `DBT_NOVA_SEARCH_STAGING_DEBOOST_FACTOR` (default: `0.6`)
- `DBT_NOVA_SEARCH_ANALYST_CANDIDATE_FALSE_DEBOOST_FACTOR` (default: `0.45`)
- `DBT_NOVA_SEARCH_ENGINEER_CANDIDATE_FALSE_DEBOOST_FACTOR` (default: `1.0`)
- `DBT_NOVA_SEARCH_GOVERNANCE_CANDIDATE_FALSE_DEBOOST_FACTOR` (default: `1.0`)
- `DBT_NOVA_SEARCH_MEASURE_MATCH_MULTIPLIER` (default: `1.15`)
- `DBT_NOVA_SEARCH_METRIC_MATCH_MULTIPLIER` (default: `1.2`)
- `DBT_NOVA_SEARCH_ANALYST_SEMANTIC_MATCH_MULTIPLIER` (default: `1.35`)
- `DBT_NOVA_SEARCH_NON_ANALYST_SEMANTIC_MATCH_MULTIPLIER` (default: `1.05`)
- `DBT_NOVA_SEARCH_SEMANTIC_NAME_MATCH_MULTIPLIER` (default: `1.12`)
- `DBT_NOVA_SEARCH_SEMANTIC_SYNONYM_MATCH_MULTIPLIER` (default: `1.08`)
- `DBT_NOVA_SEARCH_SEMANTIC_DEFINITION_MATCH_MULTIPLIER` (default: `1.03`)
- `DBT_NOVA_SEARCH_SEMANTIC_CANONICAL_MATCH_MULTIPLIER` (default: `1.25`)
- `DBT_NOVA_SEARCH_SEMANTIC_CANONICAL_MATCH_BONUS` (default: `1.5`)
- `DBT_NOVA_SEARCH_SYNONYM_MATCH_MULTIPLIER` (default: `1.2`)
- `DBT_NOVA_SEARCH_CANONICAL_MATCH_MULTIPLIER` (default: `1.08`)
- `DBT_NOVA_SEARCH_CANONICAL_META_MATCH_MULTIPLIER` (default: `1.35`)
- `DBT_NOVA_SEARCH_CANONICAL_META_MATCH_BONUS` (default: `2.5`)
- `DBT_NOVA_SEARCH_ENGINEER_EXACT_MATCH_MULTIPLIER` (default: `2.0`)

Indicator search and grouping:
- `DBT_NOVA_SEARCH_INDICATOR_ENABLE_PARENT_COHERENCE` (default: `true`)
- `DBT_NOVA_SEARCH_INDICATOR_PARENT_GROUP_MAX_GROUPS` (default: `5`)
- `DBT_NOVA_SEARCH_INDICATOR_PARENT_GROUP_MAX_INDICATORS` (default: `4`)
- `DBT_NOVA_SEARCH_INDICATOR_GENERIC_LABEL_BONUS_ONE_TOKEN` (default: `2.5`)
- `DBT_NOVA_SEARCH_INDICATOR_GENERIC_LABEL_BONUS_TWO_TOKENS` (default: `1.5`)
- `DBT_NOVA_SEARCH_INDICATOR_GENERIC_LABEL_BONUS_THREE_PLUS_TOKENS` (default: `0.75`)
- `DBT_NOVA_SEARCH_INDICATOR_PARENT_SYNONYM_BONUS_ONE_TOKEN` (default: `1.5`)
- `DBT_NOVA_SEARCH_INDICATOR_PARENT_SYNONYM_BONUS_TWO_TOKENS` (default: `1.0`)
- `DBT_NOVA_SEARCH_INDICATOR_PARENT_SYNONYM_BONUS_THREE_PLUS_TOKENS` (default: `0.5`)
- `DBT_NOVA_SEARCH_INDICATOR_SEMANTIC_LABEL_PRECISION_SCALE` (default: `0.14`)
- `DBT_NOVA_SEARCH_INDICATOR_SEMANTIC_LABEL_PRECISION_CANONICAL_BONUS` (default: `0.12`)
- `DBT_NOVA_SEARCH_INDICATOR_PARENT_COHERENCE_DIVERSITY_BONUS` (default: `0.28`)
- `DBT_NOVA_SEARCH_INDICATOR_PARENT_COHERENCE_CANONICAL_BONUS` (default: `0.14`)
- `DBT_NOVA_SEARCH_INDICATOR_PARENT_COHERENCE_STRONG_MATCH_BONUS` (default: `0.08`)
- `DBT_NOVA_SEARCH_INDICATOR_PARENT_COHERENCE_SUPPORT_SURFACE_BONUS` (default: `0.06`)
- `DBT_NOVA_SEARCH_INDICATOR_PARENT_COHERENCE_TIME_FIELD_BONUS` (default: `0.06`)
- `DBT_NOVA_SEARCH_INDICATOR_PARENT_COHERENCE_DIMENSION_BONUS` (default: `0.06`)
- `DBT_NOVA_SEARCH_INDICATOR_PARENT_COHERENCE_MAX_BONUS` (default: `0.95`)
- `DBT_NOVA_SEARCH_INDICATOR_SEARCH_PARENT_BONUS_SCALE` (default: `0.55`)
- `DBT_NOVA_SEARCH_INDICATOR_SEARCH_MISSING_PARENT_MULTIPLIER` (default: `0.75`)
- `DBT_NOVA_SEARCH_INDICATOR_SEARCH_PARENT_TOP_K` (default: `3`)
- `DBT_NOVA_SEARCH_INDICATOR_RRF_SCORE_WEIGHT` (default: `1.0`)
- `DBT_NOVA_SEARCH_INDICATOR_RERANKER_SCORE_WEIGHT` (default: `1.0`)

Metadata support signals:
- `DBT_NOVA_SEARCH_METADATA_SUPPORT_PARENT_SYNONYM_WEIGHT` (default: `0.4`)
- `DBT_NOVA_SEARCH_METADATA_SUPPORT_DOMAIN_WEIGHT` (default: `0.35`)
- `DBT_NOVA_SEARCH_METADATA_SUPPORT_USE_CASE_WEIGHT` (default: `0.25`)
- `DBT_NOVA_SEARCH_METADATA_SUPPORT_DIMENSION_WEIGHT` (default: `0.45`)
- `DBT_NOVA_SEARCH_METADATA_SUPPORT_COLUMN_NAME_WEIGHT` (default: `0.2`)
- `DBT_NOVA_SEARCH_METADATA_SUPPORT_COLUMN_ROLE_WEIGHT` (default: `0.2`)
- `DBT_NOVA_SEARCH_METADATA_SUPPORT_SEMANTIC_TYPE_WEIGHT` (default: `0.35`)
- `DBT_NOVA_SEARCH_METADATA_SUPPORT_EXAMPLE_VALUE_WEIGHT` (default: `0.5`)
- `DBT_NOVA_SEARCH_METADATA_SUPPORT_MAX_BONUS` (default: `1.25`)
- `DBT_NOVA_SEARCH_METADATA_SUPPORT_MAX_VALUES_PER_FIELD` (default: `4`)

Staging deboost behavior:
- `DBT_NOVA_SEARCH_STAGING_DEBOOST_FACTOR` applies when an entity matches a configured
  layer rule with layer name `staging`, `stage`, or `stg` (case-insensitive).
- Configure layer mapping with `DBT_NOVA_LAYER_RULES`.

Persona candidate behavior:
- `DBT_NOVA_SEARCH_*_CANDIDATE_FALSE_DEBOOST_FACTOR` applies when
  `meta.nova.search.candidates.<persona>` is explicitly `false`.
- Missing candidate metadata defaults to `true`.
- Candidate deboosts are ranking hints only; they do not hide entities.
- Exact matches on entity name, alias, unique id, or file path bypass the deboost.

## SQL Providers

`execute_sql` uses the SQL provider configured by `DBT_NOVA_SQL_PROVIDER`.
Supported providers:

- `databricks` (default): requires `DATABRICKS_HOST`, `DATABRICKS_ACCESS_TOKEN`, and a warehouse id (`DATABRICKS_HTTP_PATH` or `DATABRICKS_SQL_WAREHOUSE_ID`).
- `bigquery`: requires a project id (`DBT_NOVA_BIGQUERY_PROJECT_ID`, `DBT_NOVA_GCP_PROJECT_ID`, or `GOOGLE_CLOUD_PROJECT`) and credentials from one of:
  - OAuth token env: `DBT_NOVA_BIGQUERY_ACCESS_TOKEN`, `DBT_NOVA_GCP_ACCESS_TOKEN`, `GCP_ACCESS_TOKEN`, `GOOGLE_OAUTH_ACCESS_TOKEN`
  - Service-account key path: `GOOGLE_APPLICATION_CREDENTIALS`
  - gcloud ADC (`gcloud auth application-default login`)
- `snowflake`: requires `DBT_NOVA_SNOWFLAKE_ACCOUNT` or `DBT_NOVA_SNOWFLAKE_ACCOUNT_URL`, `DBT_NOVA_SNOWFLAKE_WAREHOUSE`, and one supported auth mode:
  - Key-pair JWT auth: `DBT_NOVA_SNOWFLAKE_USER` plus `DBT_NOVA_SNOWFLAKE_PRIVATE_KEY_PATH` or `DBT_NOVA_SNOWFLAKE_PRIVATE_KEY_PEM`. This is the default when no token env vars are present. If only `DBT_NOVA_SNOWFLAKE_ACCOUNT_URL` is set, also set `DBT_NOVA_SNOWFLAKE_JWT_ACCOUNT`. Legacy locator-style region suffixes are excluded from JWT claims while organization/account identifiers are preserved.
  - OAuth token auth: `DBT_NOVA_SNOWFLAKE_AUTH=oauth` plus `DBT_NOVA_SNOWFLAKE_OAUTH_TOKEN`. Nova uses the supplied bearer token and does not mint or refresh OAuth tokens.
  - Programmatic access token auth: `DBT_NOVA_SNOWFLAKE_AUTH=pat` plus `DBT_NOVA_SNOWFLAKE_PAT`. PAT is inferred when `DBT_NOVA_SNOWFLAKE_AUTH` is omitted and `DBT_NOVA_SNOWFLAKE_PAT` is set.
  - External browser SSO auth: `DBT_NOVA_SNOWFLAKE_AUTH=externalbrowser` plus `DBT_NOVA_SNOWFLAKE_USER`; requires `DBT_NOVA_SNOWFLAKE_ACCOUNT` because browser SSO login needs the account name. This is local interactive auth only.
- `duckdb`: requires `DBT_NOVA_DUCKDB_PATH` and executes queries against that file in read-only mode. Ad-hoc DuckDB file-scan functions in `execute_sql` text are rejected. Connection-level external access for trusted file-backed database objects is disabled by default; enabling it requires `DBT_NOVA_DUCKDB_ALLOW_EXTERNAL_ACCESS=true` and `DBT_NOVA_DUCKDB_FILE_SEARCH_PATH`, which is applied as both the DuckDB `file_search_path` and an `allowed_directories` bound before configuration is locked.

Databricks runtime tuning env vars:
- `DATABRICKS_WAIT_TIMEOUT_S` (default: `10`)
- `DATABRICKS_POLL_INTERVAL_MS` (default: `1000`)
- `DATABRICKS_MAX_POLL_SECONDS` (default: `600`)
- `DATABRICKS_TIMEOUT_MS` (derived from wait timeout + 5 seconds, min `30000`)
- `DATABRICKS_MAX_GET_RETRIES` (default: `2`)

Snowflake notes:
- Named `:parameter` placeholders are rewritten to Snowflake SQL API positional
  `?` binds. Null parameters require explicit `parameter_types`.
- `warehouse_id` overrides `DBT_NOVA_SNOWFLAKE_WAREHOUSE` for a single request.
- Result partitions are fetched through the SQL API and remain bounded by
  `row_limit`, `byte_limit`, `fetch_all_chunks`, and `max_chunks`.
- If local polling exceeds `max_poll_seconds`, Nova calls Snowflake's statement
  cancel endpoint before returning a timeout error.
- Key-pair auth supports unencrypted RSA PEM keys. Encrypted private keys are not
  supported yet; use OAuth or PAT auth if a passphrase-protected key is required.
- Snowflake SQL API workload identity federation is not implemented yet. Use
  key-pair JWT, OAuth, or PAT auth for hosted automation.
- External browser auth is a local interactive mode. It binds a loopback callback
  listener, opens the system browser to Snowflake SSO, keeps the returned session
  token only in process memory, rejects CI or non-loopback streamable HTTP
  deployments, and supports Okta SAML callbacks that omit `proofKey`.

Provider diagnostics are available through `execute_sql` with `preflight_only=true`
plus optional `preflight_catalog`, `preflight_schema`, and `preflight_relation`.
Object-level preflight checks (`preflight_catalog`, `preflight_schema`,
`preflight_relation`) are treated as available only when the probe query returns
at least one row.

DuckDB notes:
- Named parameters are supported and rewritten to positional binds.
- `parameter_types` is not supported for DuckDB v1 (pass scalar values via `parameters` only).
- Connections are pooled per process and per `(duckdb_path,file_search_path,external_access)` key.

When provided by callers, `row_limit`, `byte_limit`, `max_chunks`, and
`max_poll_seconds` are clamped to the configured `DBT_NOVA_SQL_MAX_*` values.
`poll_interval_ms` is raised to `DBT_NOVA_SQL_MIN_POLL_INTERVAL_MS` when below
the configured minimum.

See [MCP Client Configs](../getting-started/mcp-clients.md) for required variables.

Search boost and Nova ranking tuning variables are documented in
[Search Ranking](../features/search-ranking.md).

---

## See Also

- [Manifest Sources](manifest-sources.md) - Remote manifest authentication
- [Search Defaults](search-defaults.md) - Default search behavior
- [Search Ranking](../features/search-ranking.md) - Ranking algorithms and boosts
- [MCP Clients](../getting-started/mcp-clients.md) - Client configuration examples
