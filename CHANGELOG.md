# Changelog

All notable changes to dbt-nova will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Added `dbt-nova eval` commands for manifest bridge evals and provider-backed
  agent evals, including sanitized CLI/MCP tool-call tracing, YAML/JSON suite
  definitions, score gates, and JSON/TSV/Markdown report artifacts.

### Fixed

- Agent evals now score Codex/Claude/OpenCode MCP tool calls from provider JSON
  event streams when a local dbt-nova trace file is empty, including MCP servers
  configured with aliases such as `dbt-nova` instead of `nova`, and Claude
  `tool_result` payloads now populate `selected_entities` evidence. Provider
  fallback parsing now filters MCP events by Nova server aliases and validates
  eval artifact path collisions case-insensitively.
- Eval suite validation now rejects `search_columns_rank` assertions that omit
  both `expected_column` and `expected_parent_unique_id`.
- The default OpenCode agent-eval provider preset no longer passes a redundant
  `--dir` flag; dbt-nova sets the provider process working directory directly.
- Agent eval `final_answer` assertions now score extracted assistant final text
  from provider JSON event streams instead of matching against full stdout.
- The Claude provider preset now passes `--verbose` with `--output-format
  stream-json`, matching current Claude Code CLI requirements.
- Eval artifact path generation now rejects colliding sanitized case IDs and
  blocks `.`/`..` path segments; custom provider commands also reject empty
  command values.
- Documentation deployment now runs from `master` instead of release tags, matching
  GitHub Pages environment protection rules.
- Release validation now fetches only the requested release tag, preventing local
  tag-clobber failures on tag-triggered release runs.

## [0.0.4] - 2026-04-26

### Added

- CLI-only `dbt-nova audit nova-meta` validation for `meta.nova`, with project/file/resource/column targeting, JSON output, schema validation against `schemas/nova/v0.json`, and local semantic checks for references, grain consistency, duplicate definitions, and filter-operator rules.
- New search and modeling tools for semantic inventory and cleanup work:
  `search_indicator`, `indicator_inventory`, `search_columns`, `column_inventory`,
  `compare_grains`, `find_entity_overlap`, and `modelling_consistency_report`.
- Deterministic search explain/debug mode for `search` and `search_indicator`, including ranking-factor and retriever contribution output.
- New standalone persona-first skill architecture under `.github/skills/` for analyst, BI engineering, engineering, governance, KPI debugging, model architecture, project cleanup, and metadata authoring workflows across MCP and CLI transports.
- Helper workflow scripts for architecture and cleanup work:
  `scripts/export_entity_inventory.py`, `scripts/export_column_inventory.py`,
  `scripts/build_overlap_report.py`, and `scripts/install_skills.sh`.
- Streamable HTTP server transport for hosted deployments, including container packaging and built-in liveness/readiness probe endpoints.
- Persona-specific search ranking hints via `meta.nova.search.candidates.<persona>` so helper/ops models can stay searchable while being deboosted for analyst discovery.
- Reusable metadata audit workflow plus `dbt-nova audit metadata-score` CLI support for project-wide, changed-entity, and explicit-entity metadata gating in CI.
- Query-aware Nova semantic previews in search results, plus stronger canonical measure/metric ranking so analyst search surfaces the preferred execution model and formula directly.
- Tagged releases now publish a smoke-tested OCI image for hosted/server deployments.
- Reusable asset builds now expose `search_warm_strategy` (`staged` or `full`) and
  `build_timeout_minutes` inputs so large manifests can warm semantic assets in
  bounded stages and choose an explicit job timeout.
- Manifest unique_id pruning via `DBT_NOVA_PRUNE_ALLOW_IDS` and
  `DBT_NOVA_PRUNE_DENY_IDS`, with cache identity isolation and fail-fast
  validation for malformed prune JSON.

### Changed

- Search ranking is now more configurable, deterministic, and analyst-oriented:
  canonical indicator/entity signals, metadata-support signals, parent coherence,
  RRF fusion, reranker alignment, stable tie-breaking, and cleaner support evidence
  all participate in ranking for both `search` and `search_indicator`.
- Search/modeling inventory and consistency tools now respect the same search
  concurrency and timeout guardrails as the main search surface.
- Skill installation now supports standalone persona-first skills directly:
  `--install-skills` installs the full standalone skill set, while `--skill`
  or `DBT_NOVA_SKILL_NAME` installs a single skill. Deprecated `cli`/`mcp`
  bundle selectors are mapped for compatibility.
- CI hardening now includes lower-memory lint/coverage/test settings and extra
  runner disk cleanup for the `test` job.
- Reusable asset publishing now supports GitHub OIDC for GCS targets, refreshes GCS access tokens during longer uploads, uses `gcloud storage cp` for large transfers, defaults semantic cache warmup to staged mode, and applies configurable build/publish timeout budgets.
- Metadata audit and reusable asset workflows now support the shared `DBT_NOVA_SECRET_BUNDLE_JSON` secret-bundle contract for cross-repo dbt execution.
- Analyst search now prefers matched canonical Nova measures and metrics more strongly, including cases where the same business term appears across multiple models or alongside standalone dbt `metric` entities.
- Release automation now pushes the exact smoke-tested OCI image instead of rebuilding a separate image during release.
- Release preparation and tagged releases now validate `Cargo.toml` and `CHANGELOG.md`
  against the requested version/tag so published binaries, OCI images, and release notes
  stay aligned.
- Reusable workflows now derive installer defaults from the supported
  `github.workflow_ref` context only, keeping asset build and metadata audit
  workflow validation compatible with `actionlint`.
- Release builds now attach SPDX JSON SBOMs, use narrower job-level permissions,
  and block release preparation if the `[Unreleased]` changelog section still
  contains entries.
- Crate metadata and the security policy now include fuller publication and
  coordinated-disclosure details.
- CI now lints and tests `--all-features`, and adds an explicit Rust 1.93 MSRV check to
  match the declared minimum supported toolchain.
- Monthly fuzz maintenance now uses a shared Rust cache and a larger timeout budget so nightly fuzz targets spend time fuzzing instead of recompiling.

### Fixed

- `indicator_inventory canonical_only=true` now treats entity-level canonical
  metadata consistently, matching search behavior.
- Modeling consistency and overlap checks now use normalized grain signatures,
  apply pagination offsets correctly, compare the best matching grain variants,
  and detect overlap through repeated shared column names as well as semantic metadata.
- `scripts/build_overlap_report.py` now reports full overlap candidate totals
  separately from explicitly displayed inconsistency counts and deduplicates
  displayed duplicate-indicator names.
- `search` ordering now sorts by the returned final score before parent-signal
  tie-breaks, preventing lower-scored rows from outranking stronger results.
- `audit nova-meta` now rejects unsupported `recommended_filters.operator`
  values explicitly and avoids false missing-field errors for dbt `metric` resources
  that do not declare `columns`.
- Reusable metadata audit workflows now resolve `selection_mode=changed` from
  immutable pull-request SHAs when available, so reruns stay stable after the
  base branch has advanced.
- Manifest metadata readers now fall back from legacy `meta` to dbt 1.11+
  `config.meta` for Nova entity/column metadata, primary-key detection, search
  indexing, and metadata scoring, while preserving legacy-field precedence when
  both shapes are present.
- Public docs, examples, and fixtures were sanitized to remove private
  manifest-specific vocabulary from the public repository.
- Hosted HTTP startup/bind fallback handling is more robust: invalid `PORT` fallback is ignored, MCP paths are normalized/validated, and reserved probe paths are rejected.
- Non-loopback hosted HTTP binds now fail fast unless operators explicitly acknowledge
  that dbt-nova is behind an authenticating reverse proxy via
  `DBT_NOVA_HTTP_EXPECT_AUTH_PROXY=true`; the published container image sets that
  acknowledgement explicitly for hosted deployments, and startup logs now warn
  clearly that the built-in streamable HTTP transport has no authentication layer.
- Cold-start search model failures now degrade safely by disabling broken empty search indexes instead of leaving startup wedged.
- Bootstrap artifact hydration now works when invoked from an existing current-thread
  Tokio runtime, fixing hosted/embedded startup paths that fetch bootstrap contracts
  during server initialization.
- Metadata audit tests now use isolated storage roots under parallel execution, and the reusable audit workflow no longer cancels sibling invocations on the same ref.
- Documentation and dependency maintenance issues that broke docs or security checks were corrected (`Pygments` compatibility and `tar` advisory updates).
- Release installs with `--install-skills` now work with the standalone
  persona-first skill layout instead of assuming the removed legacy
  `cli`/`mcp`/`shared` directory structure.
- Skill installers now reject unsafe skill names before constructing install
  paths or removing existing destination directories.

### Documentation

- Added and refreshed docs for the new search/modeling tools, CLI `audit nova-meta`,
  standalone skill installation, new persona skills, and updated
  analyst/engineering/governance workflows.
- Updated install, quickstart, modes, release, and README guidance to describe
  standalone persona skills and the new `--skill` / `DBT_NOVA_SKILL_NAME`
  controls.
- Added and refreshed docs for hosted deployment, streamable HTTP mode, prebuilt bootstrap/artifact hydration, metadata audit workflows, OCI release behavior, persona-specific search candidate metadata/ranking, and canonical Nova metric/measure search behavior.
- Added explicit security guidance that hosted streamable HTTP deployments must be
  fronted by an authenticating proxy, along with updated hosted examples, release docs,
  and contributor docs for `RELEASE_TAG_TOKEN` and version/tag release flow checks.

## [0.0.3] - 2026-03-19

### Added

- Full one-shot CLI command surface:
  `config show|validate`, `storage inspect|prune|cleanup`, `health check`,
  `manifest load|reload`, and `tool call` parity with MCP tools.
- `tool call reload_manifest` support in CLI mode with one-shot reload output.
- Reusable asset producer workflow (`.github/workflows/nova-build-assets.yml`)
  to build and publish storage artifacts + metadata contract from
  `manifest_path`, `manifest_uri`, or an optional dbt command.
- CI coverage for reusable producer/consumer contracts, including read-only
  consumer smoke tests and negative-path contract checks.
- Optional remote publish targets (`s3`, `gcs`, `dbfs`) for reusable assets,
  including `publish_dry_run` contract coverage.
- Stable bootstrap alias publishing for reusable asset workflows
  (`<storage_instance_id>-latest-bootstrap.json`) so consumers can keep a
  fixed `DBT_NOVA_BOOTSTRAP_URI` across asset refreshes.
- Warm-model integrity controls for direct Hugging Face fallback downloads:
  `DBT_NOVA_WARMUP_CHECKSUM_MODE=off|warn|required` and
  `DBT_NOVA_WARMUP_CHECKSUM_FILE=/path/to/checksums.txt`.
- Protocol-level MCP stdio smoke test coverage (`initialize` + `tools/list`)
  plus shared integration fixture helpers for manifest/search setup.

### Changed

- Read-only index reuse now keys on manifest content hash (path-independent),
  so prebuilt assets can be reused when manifest content matches even if file
  paths differ.
- README and MkDocs now document the prebuilt asset producer/consumer workflow,
  CLI command mode, and MCP read-only consumer env setup.
- CI now validates reusable-asset dry-run publish outputs and metadata contract
  outputs as part of the main PR/push pipeline.
- CI/release/docs workflows now use explicit timeout budgets and consistent
  Linux runner pinning (`ubuntu-22.04`), with improved docs pip cache keys and
  Rust cache coverage for config/coverage jobs.
- Workflow shell-boundary hardening routes sensitive expressions via `env`
  variables, removes token-in-URL git push patterns, and clarifies trusted
  execution of `dbt_command` in the reusable asset workflow docs.
- Manifest loader initialization was decomposed into staged helpers, and MCP
  concurrency-permit error handling is now centralized.
- Internal module layout was further decomposed for maintainability:
  - `manifest/loader.rs` now delegates parsing/runtime/storage helpers to
    `manifest/loader/{parse,runtime,storage}.rs`.
  - `manifest/search/summary.rs` now delegates persona/Nova response shaping to
    `manifest/search/summary/{persona,nova}.rs`.

### Fixed

- CLI error handling now emits structured JSON envelopes for command failures
  and validates/sanitizes manifest source inputs more strictly.
- CLI safety checks now enforce storage instance-id override semantics and
  read-only final instance-id validation.
- CLI manifest-load path now preserves URI refresh/cache semantics.
- Reusable asset CI checks now ignore transient Tantivy lock/managed files and
  hash only persisted contract artifacts.
- Reusable remote publish workflow now emits stable non-null URI outputs and
  fixes DBFS publish heredoc execution parsing in CI.
- Warm-model fallback checksum handling now:
  - enforces checksum validation for cached files without swallowing verifier status
  - re-downloads on hash mismatch
  - preserves usable cache files for non-mismatch verification errors
  - handles CRLF checksum-manifest lines correctly.
- GCP auth environment tests are now hermetic and no longer rely on ambient
  shell/session variables.

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
