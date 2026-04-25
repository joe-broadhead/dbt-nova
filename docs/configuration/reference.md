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
- `DBT_NOVA_ARTIFACT_ALLOW_HTTP` – allow `http://` artifact URIs (`true`|`false`, default: `false`)
- `DBT_NOVA_SERVER_TRANSPORT` – MCP server transport (`stdio` or `streamable_http`, default: `stdio`)
- `DBT_NOVA_HTTP_HOST` – bind host for streamable HTTP mode (default: `127.0.0.1`; falls back to `0.0.0.0` when `PORT` is set and `DBT_NOVA_SERVER_TRANSPORT=streamable_http`)
- `DBT_NOVA_HTTP_PORT` – bind port for streamable HTTP mode (default: `8000`; falls back to `PORT` when unset)
- `DBT_NOVA_HTTP_PATH` – HTTP mount path for MCP requests in streamable HTTP mode (default: `/mcp`; reserved probe paths `/healthz` and `/readyz` are not allowed)
- `DBT_NOVA_HTTP_EXPECT_AUTH_PROXY` – required acknowledgement for non-loopback streamable HTTP binds (`true`|`false`, default: `false`; set to `true` only when an authenticating reverse proxy is enforcing access in front of dbt-nova)
- `DBT_NOVA_HTTP_STATEFUL_MODE` – enable stateful streamable HTTP sessions (`true`|`false`, default: `true`)
- `DBT_NOVA_HTTP_SSE_KEEP_ALIVE_SECS` – SSE keepalive interval for streamable HTTP mode (`0` disables keepalives, default: `15`)
- `DBT_NOVA_HTTP_SSE_RETRY_SECS` – SSE retry hint for streamable HTTP mode (`0` disables retry hints, default: `3`)
- `DBT_NOVA_STRICT_SCHEMA` – fail build if schema files are missing or invalid (`true`|`false`, default: `false`; forced `true` in CI)
- `DBT_NOVA_S3_MODE` – S3 fetch mode (`https` or `sdk`, default: `https`)
- `DBT_NOVA_GCS_MODE` – GCS fetch mode (`https` or `sdk`, default: `https`)
- `DBT_NOVA_RECIPES_DIR` – manifest `original_file_path` prefix used to discover recipe `analysis` nodes (default: `analyses/recipes`). Recipe SQL is resolved from manifest `compiled_code` (or `raw_code` fallback). Recipes are documented in [Analysis Recipes](../features/recipes.md).
- `DBT_NOVA_LOG` / `RUST_LOG` – enable structured logs to stderr (e.g., `info`, `debug`, `trace`)
- `DBT_NOVA_DISABLE_TOOL_SCHEMAS` – strip JSON schema hints from MCP tools (useful for strict clients like Gemini; see [MCP Clients](../getting-started/mcp-clients.md))
- `DBT_NOVA_SQL_PROVIDER` – SQL backend for `execute_sql` (`databricks`, `bigquery`, or `duckdb`, default: `databricks`)
- `DBT_NOVA_GCP_PROJECT_ID` – shared Google project id alias (used by BigQuery fallback resolution)
- `DBT_NOVA_GCP_ACCESS_TOKEN` – shared Google OAuth access token alias (used by BigQuery fallback resolution)
- `DBT_NOVA_BIGQUERY_PROJECT_ID` – BigQuery project id when `DBT_NOVA_SQL_PROVIDER=bigquery` (falls back to `DBT_NOVA_GCP_PROJECT_ID`, `GOOGLE_CLOUD_PROJECT`, `GCP_PROJECT_ID`)
- `DBT_NOVA_BIGQUERY_ACCESS_TOKEN` – OAuth access token for BigQuery when `DBT_NOVA_SQL_PROVIDER=bigquery` (falls back to `DBT_NOVA_GCP_ACCESS_TOKEN`, `GCP_ACCESS_TOKEN`, `GOOGLE_OAUTH_ACCESS_TOKEN`, `GOOGLE_APPLICATION_CREDENTIALS`, or gcloud ADC)
- `DBT_NOVA_BIGQUERY_LOCATION` – optional BigQuery location for `execute_sql` and provider preflight
- `DBT_NOVA_BIGQUERY_TIMEOUT_MS` – HTTP timeout for BigQuery API requests (default: `30000`)
- `DBT_NOVA_BIGQUERY_TOKEN_CACHE_TTL_SECS` – cache TTL for BigQuery auth token + HTTP client reuse (default: `3000`, minimum: `60`)
- `DBT_NOVA_DUCKDB_PATH` – required DuckDB database file when `DBT_NOVA_SQL_PROVIDER=duckdb`
- `DBT_NOVA_DUCKDB_FILE_SEARCH_PATH` – optional DuckDB `file_search_path` used for external file-backed objects when `DBT_NOVA_SQL_PROVIDER=duckdb`
- `DBT_NOVA_DUCKDB_POOL_MAX_SIZE` – optional max pooled DuckDB connections per `(duckdb_path,file_search_path)` key (default: falls back to `DBT_NOVA_SQL_MAX_CONCURRENT`, then `10`)
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
- Bootstrap precedence is deterministic: explicit env vars override bootstrap values, and bootstrap values override defaults.
- Streamable HTTP mode has **no built-in authentication**. Keep it bound to loopback for local use, or set `DBT_NOVA_HTTP_EXPECT_AUTH_PROXY=true` only when an authenticating reverse proxy is enforcing access in front of dbt-nova.
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

- `DBT_NOVA_DEFAULT_LIMIT` – fallback result limit when `limit=0` (default: `50`)
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
- `DBT_NOVA_SEARCH_VECTOR_TOP_K` – max vector hits before fusion (default: `200`)
- `DBT_NOVA_SEARCH_VECTOR_MAX_CHARS` – max chars in embedding text (default: `4000`)
- `DBT_NOVA_SEARCH_ENABLE_VECTOR_ANN` – enable ANN buckets (default: `true`)
- `DBT_NOVA_SEARCH_ENABLE_VECTOR_QUANTIZATION` – enable 8‑bit quantization (default: `false`)
- `DBT_NOVA_SEARCH_VECTOR_ANN_BITS` – ANN hash bits (default: `16`)
- `DBT_NOVA_SEARCH_VECTOR_ANN_HAMMING` – ANN Hamming radius (default: `1`)
- `DBT_NOVA_SEARCH_VECTOR_ANN_MAX_CANDIDATES` – max ANN candidates (default: `5000`)
- `DBT_NOVA_SEARCH_VECTOR_ANN_MIN_CANDIDATES` – min before full scan (default: `200`)
- `DBT_NOVA_SEARCH_EMBEDDING_BATCH_SIZE` – embedding batch size (default: `128`)
- `DBT_NOVA_EMBEDDING_MODEL` – embedding model name (default: `intfloat/multilingual-e5-base`)
- `DBT_NOVA_SEARCH_ENABLE_SPARSE` – enable sparse vectors (default: `false`)
- `DBT_NOVA_SEARCH_SPARSE_TOP_K` – max sparse hits before fusion (default: `200`)
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

Staging deboost behavior:
- `DBT_NOVA_SEARCH_STAGING_DEBOOST_FACTOR` applies when an entity matches a configured
  layer rule with layer name `staging`, `stage`, or `stg` (case-insensitive).
- Configure layer mapping with `DBT_NOVA_LAYER_RULES_JSON`.

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
- `duckdb`: requires `DBT_NOVA_DUCKDB_PATH` and executes queries against that file in read-only mode. Optional `DBT_NOVA_DUCKDB_FILE_SEARCH_PATH` configures DuckDB `file_search_path` for external file-backed objects.

Databricks runtime tuning env vars:
- `DATABRICKS_WAIT_TIMEOUT_S` (default: `10`)
- `DATABRICKS_POLL_INTERVAL_MS` (default: `1000`)
- `DATABRICKS_MAX_POLL_SECONDS` (default: `600`)
- `DATABRICKS_TIMEOUT_MS` (derived from wait timeout + 5 seconds, min `30000`)
- `DATABRICKS_MAX_GET_RETRIES` (default: `2`)

Provider diagnostics are available through `execute_sql` with `preflight_only=true`
plus optional `preflight_catalog`, `preflight_schema`, and `preflight_relation`.
Object-level preflight checks (`preflight_catalog`, `preflight_schema`,
`preflight_relation`) are treated as available only when the probe query returns
at least one row.

DuckDB notes:
- Named parameters are supported and rewritten to positional binds.
- `parameter_types` is not supported for DuckDB v1 (pass scalar values via `parameters` only).
- Connections are pooled per process and per `(duckdb_path,file_search_path)` key.

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
