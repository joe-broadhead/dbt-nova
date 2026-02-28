# Changelog

All notable changes to dbt-nova will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Dedicated CLI command docs covering command tree, one-shot usage patterns,
  `tool call` parameter input modes, and server-compatibility behavior when no
  subcommand is passed.

### Changed

- README and Quick Start now explicitly document CLI command mode alongside MCP
  server mode, including examples for `health check` and `tool call`.
- API response format docs now include CLI JSON envelope structure and CLI exit
  code mapping.

## [0.0.2] - 2026-02-28

### Added

- DuckDB SQL provider support (`DBT_NOVA_SQL_PROVIDER=duckdb`) with read-only execution,
  named parameter binding, provider preflight checks, and pooled connections keyed by
  `(duckdb_path, file_search_path)`.
- End-to-end DuckDB integration coverage for `execute_sql` and `run_recipe`.
- Installer support for optional model warmup during slim installs.
- Installer support for optional built-in skill installation to `~/.agents/skills`.

### Changed

- SQL preflight behavior is now harmonized across Databricks, BigQuery, and DuckDB:
  object checks require non-empty probe results and return consistent structured readiness payloads.
- SQL execution limits were tuned for practical payloads:
  higher default byte limits, config-cap enforcement when caller limits are omitted,
  and preserved provider poll defaults when explicit poll limits are not provided.
- Scheduled security/fuzz maintenance moved to monthly automation.
- Release workflow and release docs aligned with current slim asset targets and provenance behavior.

### Fixed

- Recipe raw SQL fallback guard now reliably rejects templated/Jinja content across edge cases,
  including comment markers, dollar-quoted blocks, string literals, and backslash-escaped quotes.
- CI fuzz workflow reliability issues around target/bin resolution were corrected.
- Installer and warmup flows were aligned to avoid model-cache path mismatch confusion in slim installs.

### Documentation

- Updated installation, quickstart, MCP client, configuration, tools, recipes, skills, CI, and release docs
  to reflect DuckDB support, SQL provider behavior, model cache/warmup expectations, and current workflows.

## [0.0.1] - 2026-02-18

### Fixed

- Bundled model layout normalization now handles both `snapshots/<rev>/model.onnx` and
  `snapshots/<rev>/onnx/model.onnx` layouts in install and warmup flows.
- Bundled model discovery now falls back to `~/.local/bin/models` when executable-relative
  model lookup is unavailable in client runtime environments.
- Nightly fuzz workflow now uses explicit fuzz directory resolution and cargo-fuzz manifest
  metadata compatibility.
- Warmup model readiness now counts distinct snapshot roots, preventing duplicate-path
  inflation.

### Changed

- Release packaging now requires and bundles all 3 model families by default
  (embedding, sparse, reranker).

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
