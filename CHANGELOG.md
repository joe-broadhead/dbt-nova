# Changelog

All notable changes to dbt-nova will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.0.0] - 2026-02-17

### Added

- Initial public release of **dbt-nova** as an MCP server for dbt metadata.
- Manifest ingestion pipeline with persistent local caches and zero-copy entity access.
- Hybrid search stack (lexical + semantic + sparse + reranking) with persona-aware ranking:
  `analyst`, `engineer`, and `governance`.
- Full MCP tool surface for discovery, context, lineage, quality, and governance, including:
  `search`, `get_entity`, `get_context`, `get_lineage`, `get_column_lineage`,
  `get_test_coverage`, `get_metadata_score`, `get_undocumented`, `execute_sql`, and more.
- Deterministic **analysis recipes** workflow with:
  `search_recipes`, `get_recipe`, and `run_recipe`.
- Warehouse execution providers for Databricks and BigQuery.
- Persona skill packs and workflow documentation under `.github/skills/`.
- Full documentation site (MkDocs Material) covering API, configuration, features, operations, and release workflow.

### Security

- Configurable safety limits for query size, pagination, lineage depth, SQL rows/bytes/chunks/polling, and queueing.
- SQL concurrency controls with capped concurrent execution and queue timeouts.
- Entity store integrity checks and storage path hardening.
- Proxy environment validation for embeddings/reranker initialization.
- Dependency policy and advisory governance via `cargo-deny` and review metadata in `deny.toml`.

### Performance

- Cached index lifecycle with background refresh/reload and health/readiness reporting.
- Shared storage layout under `.dbt-nova/instances` for fast startup and reuse.
- Search evaluation harness and benchmark coverage for ranking quality and latency tuning.

### Testing

- Comprehensive Rust test suite across unit, integration, property, snapshot, and fuzz targets.
- CI quality gates for formatting, linting, tests, docs, dependency checks, and release validation.

### Notes

- First release cut for `v0.0.0`.
