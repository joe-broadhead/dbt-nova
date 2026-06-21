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

## Hosted HTTP Authentication Posture

`streamable_http` mode exposes the full MCP tool surface, including any configured
warehouse-backed `execute_sql` capability. dbt-nova does **not** provide built-in
authentication or authorization for this transport.

Some tools read from the server filesystem. `validate_nova_meta` validates dbt
YAML under the server working directory and rejects absolute or traversal paths
outside the selected project, but callers still control which in-scope project
files are scanned.

Operator policy:

- Bind to loopback (`127.0.0.1` or `::1`) for local-only use.
- For non-loopback / hosted exposure, place dbt-nova behind an authenticating reverse proxy.
- Set `DBT_NOVA_HTTP_EXPECT_AUTH_PROXY=true` only when that proxy is enforcing access.

Runtime enforcement:

- Non-loopback `streamable_http` binds fail validation unless `DBT_NOVA_HTTP_EXPECT_AUTH_PROXY=true`.
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

## Dependency Watchlist

Beyond RustSec advisories, Nova tracks known dependency constraints (for example,
the `ort-sys` RC pin and `reqwest` transitive version split) in a machine-readable
watchlist with owners, review dates, and upgrade triggers.

- Watchlist file: `dependency-watchlist.toml`
- Validation script: `scripts/check_dependency_watchlist.sh`

See [Dependency Watchlist](dependency-watchlist.md) for current entries and upgrade criteria.
