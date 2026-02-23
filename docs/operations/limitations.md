# Known Limitations

This page documents practical limits and edge cases to consider in production.

## Manifest size and memory

- Large manifests (100k+ nodes) require more memory for indexing and caching.
- Dense embeddings (`DBT_NOVA_SEARCH_ENABLE_VECTOR=true`) can require ~2 GB RAM.
- If memory is constrained, disable dense vectors or lower `DBT_NOVA_SEARCH_VECTOR_TOP_K`.

## Embeddings and models

- First‑run embedding downloads can be large; model cache location is controlled by
  `DBT_NOVA_EMBEDDINGS_CACHE_DIR`.
- Reranker models increase latency; consider disabling if throughput is more important.

## SQL execution

- `execute_sql` uses the configured SQL provider (Databricks by default).
- SQL validation blocks destructive statements (DROP/DELETE/UPDATE/INSERT/ALTER/CREATE).
- Parameterized queries are supported, but only for providers that implement them.
- DuckDB provider is read-only and requires `DBT_NOVA_DUCKDB_PATH`; DuckDB `parameter_types` hints are not supported.
- DuckDB uses a bounded per-process connection pool keyed by `(duckdb_path,file_search_path)`; tune with `DBT_NOVA_DUCKDB_POOL_MAX_SIZE` if needed.
- Request limits are server-guarded: row/byte/chunk/poll values may be clamped by
  `DBT_NOVA_SQL_MAX_*` settings.
- SQL execution concurrency is bounded by `DBT_NOVA_SQL_MAX_CONCURRENT` and
  `DBT_NOVA_SQL_MAX_QUEUE` unless explicitly set to unlimited.

## Column lineage heuristics

- Fuzzy matches can produce false positives at low confidence.
- Tighten with `confidence=high` for audits and governance workflows.

## Remote manifests

- `s3://` and `gs://` use HTTPS by default; SDK modes require credentials and SDK-enabled builds.
- `dbfs://` requires Databricks credentials and correct workspace URL.

## Entity DAG metadata quality

- Nova trusts manifest dependency metadata first for entity lineage.
- If `depends_on.nodes` is missing or malformed in the manifest, lineage quality
  degrades; use the `health` tool’s `manifest_health` diagnostics to identify
  problematic models.

## Tool schemas

- Some MCP clients (e.g., Gemini) reject JSON schema hints. Use
  `DBT_NOVA_DISABLE_TOOL_SCHEMAS=true` when required.
