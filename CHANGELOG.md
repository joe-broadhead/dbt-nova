# Changelog

All notable changes to dbt-nova will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

- Bumped the metadata scoring contract to v2 with resource-shape-aware declared
  grain scoring: aggregate marts can earn primary-key quality credit from
  `time_field` + `dimensions` plus matching uniqueness tests, and raw sources no
  longer receive measure/metric scoring recommendations or meta patch seeds.
- Added keyword phrase boosting for search and `search_indicator` so exact
  multi-word business names, synonyms, and indicator phrases surface above
  per-token noise, with phrase evidence exposed in `support_signals`.
- Made metadata-score recommendations state-aware and split agent-readiness
  `suggested_meta_patches` into `required`, `recommended`, and `refinement`
  severities while preserving the raw patch total.
- Added additive column-lineage evidence for unresolved derived columns and
  rename planning: `get_column_lineage` can now return defining SQL
  expressions, candidate upstream columns, and optional metadata/test/recipe
  references; `get_impact` accepts an optional `column` scope with reference
  counts by kind.
- Fixed CLI `tool call --json` output so `.data` contains the tool payload
  directly, matching dedicated CLI commands; tool response metadata now lives
  under `.meta.tool_response`.
- Fixed `manifest warm`/`warm_manifest` so no-flag warmup respects the resolved
  semantic search config, added governance/tier/canonical `list_entities`
  filters for structured inventory, and accepted `analyst`/`governance`
  `get_context` modes as standard-projection persona aliases.
- Hardened MCP/CLI tool parameter handling so unknown top-level params fail
  fast instead of being silently ignored, and added typed
  `get_metadata_audit` aliases for `personas`, `resource_types`,
  `changed_files`, and `entity_ids`.
- Fixed `get_entity` summary projection drift by including Nova governance
  metadata in `nova_summary` and the established `provenance.owner` block.
- Fixed DuckDB `execute_sql`/`run_recipe` temporal result cells so DATE,
  TIMESTAMP, and TIME values serialize as ISO-style strings instead of raw
  epoch-day or tick integers.
- Added a fail-closed hosted auth config skeleton for future proxy/JWT identity
  work while keeping effective runtime behavior default-off.
- Added design-only hosted identity and JWT threat-model docs, planned config
  contract docs, and guardrail tests for the default-off identity milestone.
- Added machine-checked MCP tool stability metadata, profile membership docs,
  and safety-gate posture for the canonical 53-tool catalog while preserving
  v0.0.x versioning.
- Added an additive top-level `api` response contract marker to Nova-owned MCP
  JSON bodies and CLI JSON envelopes.
- Added fixture-backed contract tests for `agent_readiness.v1` and
  `agent_modelling.v1` payload shapes, including queryability execution-surface
  evidence.
- Added v0.0.x runtime presets (`local-dev`, `ci-audit`, `hosted-discovery`,
  and `hosted-sql-trusted`) plus a `config validate` posture checklist for
  effective tool filters, hosted HTTP proxy posture, and storage/artifact
  writability.
- Added a privacy-safe Prometheus-compatible `GET /metrics` endpoint for
  streamable HTTP mode, backed by the existing tool metrics recorder and
  readiness signal, with `DBT_NOVA_METRICS_ENABLED=false` as an opt-out.
- Tightened docs and packaged skill wording around Nova as a metadata bridge,
  keeping MetricFlow/dbt Semantic Layer references framed as optional manifest
  evidence or externally owned execution paths.
- Added CODEOWNERS plus v0.0.x ADR and release-hygiene docs for maintainership,
  compatibility, hosted security, presets, and metrics posture.
- Added a CI-backed lexical scale guard with a shared synthetic manifest
  generator and measured default-mode performance baselines for manifest load,
  search, inventory, context, and readiness paths.
- Added a module-size ratchet check, hard-threshold exception register, and
  maintainer docs so code-bloat follow-up stays narrow and reviewable.
- Added response-only `direct_sql_queryable` metadata to `search_indicator` and
  `indicator_inventory` so agents can distinguish relation-backed SQL surfaces
  from Semantic Layer-backed indicators that require MetricFlow/dbt Semantic
  Layer execution.
- Added agent modelling audit docs, API guidance, and packaged skill updates so
  agents use relation-backed, Semantic Layer-backed, and metadata-only indicator
  surfaces correctly.
- Documented the v1 decision to keep agent-modelling audits on
  `modelling_consistency_report` and defer a standalone `dbt-nova audit
  modelling` wrapper until a separate report/CI contract is needed.
- Updated `crossbeam-epoch` to address `RUSTSEC-2026-0204` and restore the
  supply-chain advisory gate.
- Added response-only indicator execution metadata to `search_indicator` and
  `indicator_inventory` so agents can distinguish relation-backed,
  Semantic Layer-backed, and metadata-only indicators before execution.
- Documented the accepted v1 agent modelling finding contract, including
  severity semantics, bounded summaries, rule requirements, and deferred
  default-off checks.
- Added the empty `agent_modelling_findings` framework and bounded summary
  fields to `modelling_consistency_report`.
- Added deterministic agent-modelling findings for duplicate/canonical
  indicator ambiguity, non-queryable indicator parents, metric output/grain
  mismatches, missing metric time fields, multi-grain entities, and
  semantic-model grain gaps.
- Added catalog and semantic artifact integrity findings for indicator-field
  catalog drift, catalog-missing fields, catalog-only measure-like columns, and
  unresolved MetricFlow measure references.
- Added cross-grain, multi-fact, canonical primary-key, and analyst-surface
  layering findings to the v1 agent modelling audit response.
- Added search ambiguity, column semantic drift, and governance findings for
  analyst-facing surfaces to the v1 agent modelling audit response.
- Added conservative agent-modelling audit configuration defaults and env vars,
  including bounded findings, readiness thresholds, and SQL-shape checks
  defaulting off.
- Fed severe agent-modelling findings into agent readiness with additive
  `summary.agent_modelling` counts, mapped blockers/improvements, and modelling
  next actions.
- Hardened DuckDB `execute_sql` by disabling external file/table access by
  default, requiring an explicit bounded `DBT_NOVA_DUCKDB_ALLOW_EXTERNAL_ACCESS`
  opt-in, and rejecting DuckDB file-scan functions in SQL validation.
- Added a global streamable HTTP request body cap
  (`DBT_NOVA_HTTP_MAX_BODY_BYTES`) before the mounted MCP transport buffers
  requests.
- Moved indicator/column full-store scans off tokio worker threads, stabilized
  lineage adjacency ordering, expanded fuzz/CI coverage, and removed the unused
  direct `lru` dependency.
- Added MCP tool profiles with a lean default `agent` catalog and
  backwards-compatible `DBT_NOVA_TOOL_PROFILE=all`, plus protocol-level
  `isError` signaling for failed tool calls while retaining Nova's structured
  JSON error body.
- Bridged dbt Semantic Layer / MetricFlow `metrics` and `semantic_models` into
  Nova indicator discovery, analyst ranking, and metadata scoring without
  requiring duplicate `meta.nova` annotations.
- Added optional `catalog.json` ingestion via `DBT_NOVA_CATALOG_PATH` or local
  sibling auto-discovery so column payloads use warehouse data types and expose
  catalog drift signals.
- Documented the exported schema typegen compatibility surface, vendored crate
  patches, response-budget contracts, large-fixture/concurrency benches, and
  benchmark-gated archived-access decision.

## [0.0.6] - 2026-07-04

- Hardened OCI release images to upgrade pinned Debian runtime packages during
  image builds so fixed base-image vulnerabilities are remediated before the
  Trivy release gate runs.
- Hardened Monthly dependency checks by upgrading `anyhow` and `rmcp` past new
  RustSec advisories, retaining streamable HTTP Host validation, and adding
  `DBT_NOVA_HTTP_ALLOWED_HOSTS` for hosted reverse-proxy deployments.
- Added no-warm release hardening coverage for CLI and MCP validation,
  including an all-tool MCP catalog smoke test, a release smoke harness,
  refreshed provider-eval documentation, and packaged agent-skill readiness
  guidance for large manifest testing.
- Hardened production readiness by making installer checksum signature
  verification fail closed by default, resolving `latest` installs to the
  immutable release tag before fetching optional support assets, adding
  cooperative deadlines to synchronous MCP/CLI search scans, accounting
  semantic timeouts as circuit-breaker failures, and correcting release-tag and
  config-reference docs.
- Redacted raw upstream Databricks and BigQuery HTTP error bodies from
  CLI/MCP responses while preserving status and provider error-code context.
- Safety-gated MCP `reload_manifest` source, refresh, and storage mutations
  behind `DBT_NOVA_MCP_ENABLE_MANIFEST_RELOAD=1`; no-argument MCP reloads still
  refresh the current source.
- Bounded broad `find_by_path` scans to the requested page plus a sentinel match
  and added offset validation for parity with other search tools.
- Hardened reusable workflow installs by rejecting mutable installer refs by
  default, verifying signed release checksums, and retrying transient Cargo
  source-build network failures.
- Updated release tagging preflight validation, dependency watchlist state,
  OpenCode design-doc status, and packaged skill version metadata for the
  `v0.0.6` release line.
- Added configurable runner inputs to the reusable asset and metadata-audit
  workflows so downstream repositories can run them on self-hosted or
  organization runner pools while preserving the `ubuntu-22.04` default.

- Hardened hosted HTTP container defaults by removing the baked-in auth-proxy
  acknowledgement, defaulting published images to discovery-only SQL tool
  exposure, and documenting separate local, hosted discovery-only, and
  SQL-enabled deployment profiles.
- Fixed manifest refresh and semantic-cache correctness by comparing scoped
  manifest hashes during refresh, refusing incomplete dense/sparse semantic
  caches, making vector ANN construction deterministic, and restoring
  no-default feature lifecycle test coverage.
- Bounded remote artifact hydration, archive extraction, trace-file handling,
  and semantic search work near request deadlines with configurable resource
  limits, advisory trace file locks, and runtime health/status visibility.
- Added a reusable domain reference template and synthetic starter commerce
  example to the meta-authoring skill, plus analyst guidance and docs for using
  curated domain references as stable context rather than raw query dumps.
- Added an adversarial `reviewer` skill and provider-backed reviewer eval
  coverage for semantic-layer bypass and stale/unknown freshness caveats, plus
  analyst and eval-author guidance for high-stakes answer review.
- Added a skill/reference maintenance check for dbt model changes, with
  advisory or required CI modes, configurable evidence globs, and explicit
  allowlist suppressions for deliberate no-doc-update changes.
- Tightened the analyst skill around semantic-first KPI discovery, added
  eval-author guidance and starter eval expectations for `search_indicator`
  before context or SQL execution, and added scorer coverage for rejecting SQL
  before semantic discovery.
- Added an OpenCode reference pack design covering the target `.opencode`
  layout, role-to-tool permission model, local and remote MCP examples, smoke
  eval expectations, and portability boundaries for later Codex/Claude work.
- Added default-off extended metadata search config for allowlisted non-Nova
  dbt `meta.*` paths, including value modes, caps, sensitive-key guardrails,
  docs/reference output, and manifest-scoped search index identity.
- Extended metadata search now indexes configured entity-level and column-level
  allowlisted fields into dedicated `meta.<alias>` Tantivy fields, including
  fielded alias queries and deterministic extraction-time caps.
- Search results now expose `extended_meta_summary` for configured extended
  metadata fields with `summary: true`, while full detail retains the complete
  raw dbt metadata payload.
- Hardened CI workflows by caching reusable semantic model snapshots during
  model artifact builds and upgrading maintained GitHub Actions to Node
  24-compatible pinned releases.
- Added Snowflake SQL provider support (`DBT_NOVA_SQL_PROVIDER=snowflake`) via
  the Snowflake SQL API, including key-pair JWT, OAuth, and programmatic access
  token auth, named parameter binding, partitioned result fetching, cancellation
  on local poll timeout, and provider preflight checks.
- Snowflake key-pair JWT auth now excludes legacy locator-style region suffixes
  from JWT account claims while preserving organization/account identifiers,
  including account names that resemble Snowflake region IDs.
- Snowflake SQL API async status responses with code `333334` are now treated
  as in-progress rather than failed.
- Snowflake catalog and schema preflight checks now use bounded `SHOW` probes and
  avoid wildcard matching for identifiers containing underscores by requiring
  exact result names.
- Snowflake SQL provider now supports local interactive browser SSO with
  `DBT_NOVA_SNOWFLAKE_AUTH=externalbrowser`.
- Snowflake externalbrowser auth now accepts token-only browser callbacks used
  by Okta SAML SSO while still validating callback proof keys when they are
  supplied, and omits `PROOF_KEY` from the login request when the callback did
  not return one.
- Snowflake fixed-point numeric result values are now kept exact instead of
  coercing scaled decimals or large integers through floating-point JSON values,
  non-finite floating-point values are preserved as text instead of JSON `null`,
  and Snowflake statement timeout `0` is preserved as the SQL API maximum-timeout
  sentinel.
- Added configurable result profiles for CLI and MCP responses, including
  compact MCP defaults, bounded MCP page sizes, and `next_offset` metadata for
  paginated tool responses.
- Added tokenomics bridge eval fixtures and CI coverage to guard compact
  response budgets, KPI discovery, and MCP trace behavior for agent workflows.
- Eval traces now include a monotonic `tool_call_index`, and provider fallback
  scoring captures response byte evidence for budget assertions.
- Eval runs can now append per-assertion JSONL telemetry with
  `--telemetry`, keep newest rows with `--telemetry-retention`, and print
  filtered run history with `dbt-nova eval history --suite <NAME> --since
  <YYYY-MM-DD>`.
- Eval suites can now declare advisory readiness gates with `gate.threshold`;
  `dbt-nova eval gate <NAME> --json` reads the latest full-suite telemetry and
  reports allowed/blocked status, pass rate, and failed case/assertion ids,
  blocking stale suite-file telemetry before allowing configured gates.
- Eval bridge and agent runs now emit an `eval_card.v1` summary in
  `results.json`, write a PR-ready `card.md`, and prepend the same card to
  `report.md`, including suite purpose, scope, case counts, pass rate, gate
  evidence, telemetry status, provider metadata, and known gaps.
- Eval suites can now declare `snapshot_date`, `date_range_start`,
  `date_range_end`, and `date_field` anchors at suite or case level; Nova
  validates anchor dates, injects effective anchors into agent prompts, and
  includes them in reports and eval telemetry.
- Eval suites can now grade SQL structure with `sql_structure` assertions and
  agent `expected.sql_structures`, comparing tables, joins, projections,
  filters, and groupings while masking string/date/numeric literals and
  reporting structured clause diffs.
- Added `dbt-nova eval compare` and MCP/CLI `tool call` parity through
  `compare_eval_runs` for PR-ready before/after eval deltas, including pass-rate
  movement, newly passing/failing cases, and agent trace counters when available.
- Added a development negative-results log for ranking, retrieval, skill, and
  eval experiments that do not improve accuracy, latency, maintainability, or
  agent behavior.
- Updated `memmap2` to `0.9.11` to resolve `RUSTSEC-2026-0186`.
- Documented bridge and provider-backed eval CI templates with GitHub Actions
  examples for manifest generation, telemetry retention, eval gates, artifact
  upload, private OpenCode provider runs, and redacted trace handling.
- Added `dbt-nova trace inspect`, `trace summarize`, and `trace redact`, plus
  MCP/CLI `tool call` parity through `inspect_tool_trace`,
  `summarize_tool_trace`, and `redact_tool_trace`, for inspecting sanitized
  tool-call JSONL, writing Markdown trace summaries, and producing conservative
  redacted trace artifacts for safe sharing. MCP trace writes require
  `DBT_NOVA_MCP_ENABLE_TRACE_WRITES=1`.
- Added `dbt-nova trace replay` and MCP/CLI `tool call` parity through
  `replay_tool_trace` to replay supported deterministic trace rows against a
  manifest, compare compact response-shape evidence, and explicitly skip
  unsupported, unsafe, under-specified, or SQL-execution rows.
- Added `dbt-nova audit agent-readiness` for metadata-only agent readiness
  reports that combine project metadata scores, entity signal gaps, Nova
  indicator checks, optional eval gate evidence, JSON/Markdown artifacts, and
  opt-in blocker exit behavior.
- Agent-readiness reports now include advisory `suggested_meta_patches` for
  reviewable `meta.nova` remediation and draft `golden_question_seeds` for
  turning readiness gaps into eval backlog candidates.
- Documented the agent-readiness CI workflow with local and GitHub Actions
  commands, readiness bands, blocker categories, report artifact handling, and
  guidance for turning readiness output into remediation and eval work.
- Metadata scoring now returns a shared `metadata_score_contract.v1` scoring
  contract plus machine-readable diagnostics for description tiers, array-count
  tier progress, invalid Nova grain shapes, and primary-key integrity test gaps.
- Metadata score, metadata audit, agent-readiness, and modelling consistency
  outputs now include compact agent triage summaries with score/grade buckets,
  weak spots, repeated recommendation fields, bounded examples, and drill-down
  hints for fetching detailed rows intentionally.
- Added the `get_agent_readiness` MCP tool and CLI `tool call` bridge support
  for the same `agent_readiness.v1` report without CLI report-file writes.
- Added the `get_metadata_audit` MCP tool and CLI `tool call` bridge support
  for metadata audit reports and required/advisory gate status.
- Added the `validate_nova_meta` MCP tool and CLI `tool call` bridge support
  for nova-meta schema/semantic validation with scoped local path access.
- Added MCP and CLI `tool call` parity for eval workflows:
  `validate_eval_suite`, `get_eval_gate`, `get_eval_history`,
  `compare_eval_runs`, `run_eval`, `init_eval_suite`, and `run_agent_eval`, with
  explicit opt-in environment gates for local writes, bridge eval execution,
  provider-backed agent evals, and custom provider commands.
- Added safety-gated `warm_manifest` MCP and CLI `tool call` parity for
  semantic cache warmup, using the current manifest source and requiring
  `DBT_NOVA_MCP_ENABLE_MANIFEST_WARM=1`.
- Added config and storage admin MCP parity with `show_config`,
  `validate_config`, `inspect_storage`, and safety-gated `prune_storage` /
  `cleanup_storage` behind `DBT_NOVA_MCP_ENABLE_STORAGE_ADMIN=1`.
- Redacted secret-bearing config, reload, eval artifact, and eval evidence
  outputs by default, with an explicit unsafe local opt-in for raw provider
  logs.
- Fixed `audit nova-meta` and `validate_nova_meta` so dbt 1.11+
  `config.meta.nova` is validated with the same legacy/config merge semantics
  used by runtime search and scoring.
- Fixed release OCI publishing to build images from native Linux release binaries
  instead of compiling Rust inside the ARM64 Docker/QEMU path.
- Fixed the Monthly workflow by refreshing advisory/dependency-watchlist review
  metadata and making the fuzz workspace use the same vendored dependency
  patches as the main workspace.
- Made storage pruning deterministic when directories have equal modification
  timestamps, avoiding arbitrary cache eviction on coarse timestamp filesystems.

## [0.0.5] - 2026-05-13

### Added

- Added `dbt-nova eval` commands for manifest bridge evals and provider-backed
  agent evals, including sanitized CLI/MCP tool-call tracing, YAML/JSON suite
  definitions, score gates, and JSON/TSV/Markdown report artifacts.
- Added `dbt-nova eval validate`, repeatable `--case-id` filters for bridge and
  agent eval runs, value-aware context assertions, agent `called_with` and
  ranked entity expectations, and ordered `top_unique_ids` trace evidence.
- Added an `eval-author` packaged skill for designing, debugging, and
  operationalizing Nova bridge and provider-backed agent eval suites.
- Release hardening now publishes native `linux-arm64` and `macos-x86_64`
  binary assets, scans the release OCI image with Trivy, documents SBOM outputs
  and crates.io posture, and adds container digest pinning plus a `/healthz`
  Docker `HEALTHCHECK`.

### Fixed

- Eval `tool_success` assertions now fail when a tool returns an explicit
  `success: false` response instead of treating any successful dispatch as a
  passing tool result.
- Agent evals now score Codex/Claude/OpenCode MCP tool calls from provider JSON
  event streams when a local dbt-nova trace file is empty, including MCP servers
  configured with aliases such as `dbt-nova` instead of `nova`, and Claude
  `tool_result` payloads now populate `selected_entities` evidence. Provider
  fallback parsing now filters MCP events by Nova server aliases and validates
  eval artifact path collisions case-insensitively.
- Eval suite validation now rejects `search_columns_rank` assertions that omit
  both `expected_column` and `expected_parent_unique_id`.
- Eval `recipe_rank` assertions now match the `id` field returned by
  `search_recipes`, in addition to the legacy `recipe_id` alias.
- CI search eval smoke now runs without default semantic/storage features and
  has a longer timeout, avoiding cold-compile cancellations for the lexical
  smoke profile.
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
- Release OCI publishing now produces a signed, attested multi-arch
  `linux/amd64` + `linux/arm64` manifest while preserving a smoke-tested amd64
  image path before publication.
- CI coverage enforcement now ratchets the line floor to 70 percent, backed by
  storage pruning behavior tests for release-critical artifact cache cleanup.

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
