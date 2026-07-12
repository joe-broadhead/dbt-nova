# Security & Limits

Safety limits are enforced and configurable via environment variables:

- Search query length cap: `DBT_NOVA_MAX_QUERY_LENGTH` (default: `2000`)
- Search offset cap: `DBT_NOVA_MAX_OFFSET` (default: `10000`)
- Path pattern length cap: `DBT_NOVA_MAX_PATH_PATTERN_LENGTH` (default: `1000`)
- Lineage depth cap: `DBT_NOVA_MAX_LINEAGE_DEPTH` (default: `200`)
- Column lineage depth cap: `DBT_NOVA_COLUMN_LINEAGE_MAX_DEPTH` (default: `100`)
- Max lineage results: `DBT_NOVA_MAX_LINEAGE_RESULTS` / `DBT_NOVA_MAX_ENTITY_LINEAGE_RESULTS`
- SQL row cap: `DBT_NOVA_SQL_MAX_ROW_LIMIT` (default: `10000`)
- SQL byte cap: `DBT_NOVA_SQL_MAX_BYTE_LIMIT` (default: `100000000`)
- SQL chunk cap: `DBT_NOVA_SQL_MAX_CHUNKS` (default: `100`)
- SQL poll cap: `DBT_NOVA_SQL_MAX_POLL_SECONDS` (default: `900`)
- SQL poll interval floor: `DBT_NOVA_SQL_MIN_POLL_INTERVAL_MS` (default: `200`)
- SQL concurrent slot cap: `DBT_NOVA_SQL_MAX_CONCURRENT` (default: `10`)
- SQL queue cap: `DBT_NOVA_SQL_MAX_QUEUE` (default: `20`)
- SQL queue timeout: `DBT_NOVA_SQL_QUEUE_TIMEOUT_MS` (default: `30000`)
- Embeddings cache decompression cap: `DBT_NOVA_EMBEDDINGS_MAX_DECOMPRESSED_BYTES` (default: `4294967296`)

Embeddings/reranker startup validates proxy env vars. If `HTTP_PROXY`, `HTTPS_PROXY`,
`ALL_PROXY` (or lowercase variants) are set to non-URL values, initialization fails fast
with a configuration error.

Storage path checks prevent traversal, and checksums validate entity store integrity.
See [Configuration](../configuration/reference.md) for full limits.

## DuckDB SQL Sandbox

DuckDB `execute_sql` runs read-only queries and, by default, disables
connection-level external access before locking connection configuration.
Ad-hoc file and URI scan functions such as `read_csv`, `read_parquet`,
`read_json`, `read_text`, `read_blob`, `glob`, and related scan aliases are
rejected during SQL validation even though they parse as queries.

Trusted local file-backed DuckDB use cases can opt into external access only by
setting `DBT_NOVA_DUCKDB_ALLOW_EXTERNAL_ACCESS=true` together with
`DBT_NOVA_DUCKDB_FILE_SEARCH_PATH`; keep this disabled for untrusted MCP clients
and hosted endpoints.

## Hosted HTTP Authentication Posture

`streamable_http` mode exposes the full MCP tool surface, including any configured
warehouse-backed `execute_sql` capability. dbt-nova does **not** provide built-in
authentication or authorization for this transport.

Published container images default to discovery-only, non-admin MCP exposure with
`DBT_NOVA_PRESET=hosted-discovery`. Use `DBT_NOVA_PRESET=hosted-sql-trusted`
only for a SQL-enabled endpoint that is isolated, authenticated, and backed by
least-privilege credentials. Env vars override preset values, so an
empty-but-present `DBT_NOVA_TOOL_DENYLIST=` intentionally clears the denylist.
The hosted `/metrics` endpoint is enabled by default when HTTP mode is running;
it is privacy-safe by label design, but still exposes readiness, tool names,
call counts, error counts, and latency. Restrict scrapes with the same
proxy/network ACL as MCP, or set `DBT_NOVA_METRICS_ENABLED=false`.

Hosted identity and JWT validation are design-track work, not current runtime
behavior. See [Hosted Identity Threat Model](../development/hosted-identity-threat-model.md)
and [Hosted Identity Contract](../development/hosted-identity-contract.md) for
the default-off, proxy-first boundaries. Until an implementation lands, treat
the reverse proxy or platform auth layer as the authentication and
authorization boundary.

Some tools read from the server filesystem. `validate_nova_meta` validates dbt
YAML under the server working directory and rejects absolute or traversal paths
outside the selected project, but callers still control which in-scope project
files are scanned.

Eval MCP tools also use the server filesystem. `validate_eval_suite`,
`get_eval_gate`, `get_eval_history`, and `compare_eval_runs` are read/reporting
tools. `run_eval`, `init_eval_suite`, and `run_agent_eval` are disabled by
default and return structured invalid-parameter errors until the operator opts
in:

- `DBT_NOVA_MCP_ENABLE_EVAL_RUN=1` for bridge eval execution.
- `DBT_NOVA_MCP_ENABLE_EVAL_WRITES=1` for starter suite file writes.
- `DBT_NOVA_MCP_ENABLE_AGENT_EVAL=1` for provider-backed agent eval execution.
- `DBT_NOVA_MCP_ENABLE_CUSTOM_AGENT_PROVIDER=1` for custom agent provider
  commands or argument JSON.

Keep these flags disabled for hosted MCP unless the process runs in an isolated
trusted environment with appropriate filesystem and provider-command controls.

`reload_manifest` can mutate the live server source, refresh cadence, or storage
identity. No-argument reloads refresh the current source, but source-changing
reloads require `DBT_NOVA_MCP_ENABLE_MANIFEST_RELOAD=1`. Keep `reload_manifest`
hidden on normal hosted endpoints unless an operator channel needs it.

`warm_manifest` writes semantic cache artifacts and is also disabled by default.
Set `DBT_NOVA_MCP_ENABLE_MANIFEST_WARM=1` only for trusted local or isolated
operator sessions; the tool rejects read-only storage and always uses the
server/tool-call manifest source rather than accepting an arbitrary source.

`show_config`, `validate_config`, and `inspect_storage` are operator/admin
inspection tools. `prune_storage` and `cleanup_storage` are destructive storage
admin tools and reject by default until `DBT_NOVA_MCP_ENABLE_STORAGE_ADMIN=1` is
set. Keep that flag disabled for hosted MCP unless the process is isolated and
access controlled; use `DBT_NOVA_TOOL_DENYLIST` to hide admin tools from normal
agent clients.

Operator policy:

- Bind to loopback (`127.0.0.1` or `::1`) for local-only use.
- For non-loopback / hosted exposure, place dbt-nova behind an authenticating reverse proxy.
- Set `DBT_NOVA_HTTP_EXPECT_AUTH_PROXY=true` only when that proxy is enforcing access.
- Set `DBT_NOVA_HTTP_ALLOWED_HOSTS` to the public/proxy hostnames clients send in the `Host` header.

Runtime enforcement:

- Non-loopback `streamable_http` binds fail validation unless `DBT_NOVA_HTTP_EXPECT_AUTH_PROXY=true`.
- Streamable HTTP requests are rejected when their `Host` header is outside the transport loopback defaults and `DBT_NOVA_HTTP_ALLOWED_HOSTS`.
- Streamable HTTP request bodies are capped by `DBT_NOVA_HTTP_MAX_BODY_BYTES` before the MCP transport buffers them.
- Successful `streamable_http` startup logs a warning that the transport has no built-in auth.

## Advisory Exceptions

The following RustSec advisories are explicitly ignored in `deny.toml` with documented rationale.
These are transitive dependencies with no safe upgrade available today and are reviewed during
dependency refreshes. Each ignore entry includes `owner=...` and `review_by=YYYY-MM-DD`
metadata in the `reason` field. CI enforces that review dates are not expired via
`scripts/check_advisory_ignores.sh`.

- `RUSTSEC-2024-0384`: `instant` via `tantivy` -> `measure_time`
- `RUSTSEC-2024-0436`: `paste` via `fastembed` -> `tokenizers`
- `RUSTSEC-2025-0119`: `number_prefix` via `fastembed` -> `hf-hub` -> `indicatif`
- `RUSTSEC-2025-0134`: `rustls-pemfile` via `google-cloud-storage` -> `reqwest 0.11`
- `RUSTSEC-2026-0002`: `lru 0.12` via `tantivy`
- `RUSTSEC-2026-0097`: `rand` via `fastembed`/`tokenizers`, `tantivy`/`rand_distr`,
  `rmcp`, and `proptest`

## Dependency Watchlist

Beyond RustSec advisories, Nova tracks known dependency constraints (for example,
the `ort-sys` RC pin and the `reqwest` 0.11/0.12/0.13 transitive split) in a
machine-readable watchlist with owners, review dates, and upgrade triggers.

- Watchlist file: `dependency-watchlist.toml`
- Validation script: `scripts/check_dependency_watchlist.sh`

See [Dependency Watchlist](dependency-watchlist.md) for current entries and upgrade criteria.
