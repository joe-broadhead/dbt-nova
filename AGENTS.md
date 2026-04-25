# AGENTS.md

Guidance for coding agents working in this repository.

## Project Overview

`dbt-nova` is a Rust CLI and MCP server for turning dbt `manifest.json`
artifacts into agent-ready search, lineage, metadata scoring, SQL execution,
and recipe tools.

Core areas:

- `src/main.rs`, `src/cli/` - CLI entrypoints and command handling.
- `src/server/` - MCP stdio and streamable HTTP server behavior.
- `src/tools/` - MCP/CLI tool implementations.
- `src/manifest/` - manifest loading, entity normalization, storage, search,
  embeddings, bootstrap, and prebuilt artifact hydration.
- `src/warehouse/` - Databricks, BigQuery, and DuckDB SQL providers.
- `src/nova_meta/` - `meta.nova` schema validation and semantic checks.
- `.github/workflows/` - CI, release, reusable asset build, and metadata audit
  automation.
- `.github/skills/` - packaged agent skills installed by the release/install
  tooling.
- `docs/` - MkDocs documentation site.
- `schemas/nova/v0.json` - public Nova metadata schema.

## High-Signal Commands

Prefer focused checks while developing, then run the relevant release-grade
checks before handoff.

```bash
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
uv run mkdocs build --strict
cargo deny check
bash scripts/check_dependency_watchlist.sh
```

Useful targeted commands:

```bash
cargo test --locked <module_or_test_name>
cargo test --locked --test <integration_test_name>
DBT_NOVA_SEARCH_ENABLE_VECTOR=false \
DBT_NOVA_SEARCH_ENABLE_SPARSE=false \
DBT_NOVA_SEARCH_ENABLE_RERANKER=false \
  cargo test --locked --all-features
```

When editing workflow files, run:

```bash
actionlint .github/workflows/<workflow>.yml
```

When editing docs or config defaults, also check:

```bash
uv run mkdocs build --strict
bash scripts/check_config_reference.sh
```

## Development Rules

- Keep changes scoped. Avoid broad refactors inside release, CI, or security
  patches unless the task explicitly requires them.
- Preserve existing public CLI/MCP contracts unless the user asks for a breaking
  change.
- Use `rg`/`rg --files` for search.
- Add or update tests for behavior changes.
- Prefer explicit `DbtNovaError` propagation over panics.
- Do not add `.expect()`/`.unwrap()` in production paths unless there is a
  strong invariant and a clear message.
- Do not silently ignore errors; return them or log enough context.
- Keep source and docs ASCII unless the file already requires non-ASCII.
- Do not commit generated build artifacts, caches, or local manifests.

## Rust Notes

- Minimum supported Rust version is 1.93.
- Release artifacts are built with default features, so CI lint/test uses
  `--all-features`.
- `Cargo.lock` is committed and should remain reproducible.
- Default features include embeddings plus S3/GCS support. Use
  `--no-default-features` only when explicitly testing that mode.
- Hot paths are search, manifest loading, artifact hydration, and SQL execution.
  Avoid unnecessary allocations in those areas.

## Manifest, Search, and Metadata Rules

- `meta.nova` is the first-class semantic contract. Keep schema, docs, tests,
  and search/scoring behavior aligned.
- If changing `schemas/nova/v0.json`, update docs under `docs/features/` and add
  validation/scoring tests.
- dbt 1.11+ may place metadata under `config.meta`; preserve fallback behavior
  from legacy `meta` to `config.meta`.
- Avoid real customer/company/table names in examples and tests. Use synthetic
  names like `analytics_reporting`, `model.pkg.orders`, or `jaffle_shop`.
- Do not enable vector, sparse, or reranker work in smoke tests unless the task
  specifically requires semantic model behavior.

## MCP and Hosted Server Rules

- `streamable_http` has no built-in authentication. Non-loopback binds require
  `DBT_NOVA_HTTP_EXPECT_AUTH_PROXY=true` and must be behind an authenticating
  proxy.
- Probe endpoints are reserved:
  - `/healthz` for process liveness
  - `/readyz` for manifest/search readiness
- The MCP endpoint defaults to `/mcp`.
- Hosted bootstrap/artifact consumers need writable local storage on first run
  unless artifacts are already materialized and strict read-only mode is used.

## Reusable Workflow Rules

For `.github/workflows/nova-build-assets.yml`:

- Prefer structured dbt invocation with `dbt_command_args_json`.
- Treat `dbt_command` as trusted shell execution only.
- Keep `installer_ref` aligned with the reusable workflow ref in examples.
- Use `search_warm_strategy: staged` for large manifests or constrained
  runners.
- Use `build_timeout_minutes` for large dbt projects or remote publishes.
- Use `DBT_NOVA_SECRET_BUNDLE_JSON` for portable cross-owner secret mapping.

For `.github/workflows/nova-metadata-audit.yml`:

- Metadata-only audits should avoid full semantic startup unless explicitly
  required.
- `selection_mode=changed` should use immutable PR SHAs when available.

## Release Rules

- Do not use a `release/*` or `hotfix/*` branch name unless the user intends to
  trigger the auto-tag release flow after merge to `master`.
- Release tags are `vX.Y.Z`.
- `release.yml` validates `Cargo.toml` and `CHANGELOG.md` against the tag.
- If docs reference a new release tag before the tag exists, keep that change in
  the release PR and tag promptly after merge.
- Do not merge release PRs without explicit user approval.

## Security and Supply Chain

- Security reports belong in private disclosure, not public issues. See
  `SECURITY.md`.
- Keep `deny.toml` advisory ignores and `dependency-watchlist.toml` review dates
  current.
- Run `cargo deny check` after dependency or advisory changes.
- Never log credentials, tokens, private keys, or unsanitized artifact/manifest
  URIs.
- Preserve URI sanitization when touching manifest, artifact, or warehouse code.

## PR Checklist for Agents

Before opening or updating a PR:

- Run the narrowest meaningful tests for the touched code.
- Run formatting and lint checks when code changed.
- Run docs build when docs changed.
- Update `CHANGELOG.md` for user-facing behavior, release, CLI/MCP, workflow, or
  security changes.
- Include validation commands in the PR body.
- Call out any checks that were intentionally skipped and why.

