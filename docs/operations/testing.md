# Testing & Cleanup

## Run Tests

```bash
cargo test
cargo test -- --nocapture
```

## Test Layout

- `src/tests/` – in‑crate unit tests
- `tests/` – integration/property tests and fixtures

## No-Warm Release Smoke

Use the release smoke harness when you need to prove the built binary, CLI
surface, MCP stdio readiness, and a small set of agent-readiness checks against
a real manifest without building dense, sparse, or reranker semantic caches:

```bash
scripts/smoke_release_no_warm.sh \
  --manifest-path target/manifest.json \
  --output-dir out/nova-release-smoke \
  --storage-instance-id release-no-warm-smoke
```

The harness runs `cargo build --release --locked` unless `--skip-build` is
passed. It exports:

- `DBT_NOVA_SEARCH_ENABLE_VECTOR=false`
- `DBT_NOVA_SEARCH_ENABLE_SPARSE=false`
- `DBT_NOVA_SEARCH_ENABLE_RERANKER=false`

It intentionally does not run `dbt-nova manifest warm`. It also verifies that
`warm_manifest` remains disabled without
`DBT_NOVA_MCP_ENABLE_MANIFEST_WARM=1`, polls MCP `health` until
`ready_for_traffic=true`, checks `tools/list`, and writes `summary.json` plus
`report.md`.

Bridge and provider-backed evals are opt-in because production manifests,
provider credentials, and artifact policies differ by environment:

```bash
scripts/smoke_release_no_warm.sh \
  --manifest-path tests/fixtures/starter_eval_manifest.json \
  --bridge-suite evals/starter.yml \
  --agent-suite evals/starter.yml \
  --agent-providers opencode,claude,codex \
  --fail-under 1.0
```

Keep private manifests, provider stdout/stderr, raw traces, and credentials out
of public artifacts. The script captures only the Nova artifacts needed for a
local hardening or release review.

## Fuzzing and Scale Benches

Monthly maintenance runs fuzz targets for manifest streaming parse,
`meta.nova` YAML parsing, SQL validation, and artifact archive extraction.
Seed corpora live under `fuzz/corpus/`; crash artifacts are uploaded from
`fuzz/artifacts/` when a target fails. Keep seeds small, synthetic, and free of
private manifest data.

Search scale benches generate deterministic large manifests in-bench. Use
`DBT_NOVA_BENCH_LARGE_ENTITY_COUNT` to resize the fixture locally. The
benchmarks cover large build/search, full-store indicator inventory scans,
checked archived entity access, and concurrent hybrid search plus cheap entity
lookup. The archived-access bench is the guardrail for any future move away
from repeated checked `rkyv::access`; do not introduce unchecked archived access
without benchmark evidence and a local safety argument.

## MCP File Path Policy

MCP file tools resolve local paths under the server working directory. This
applies to eval suites, eval result comparisons, trace inspection/redaction, and
`validate_nova_meta` project scans. If a path is outside the server cwd, the
tool should reject it instead of reading arbitrary local files.

For local testing, start `dbt-nova server start` from the dbt project root when
you need `validate_nova_meta` to scan project YAML. For eval and trace tests,
write suites, result directories, and trace JSONL files under the server cwd and
pass relative paths where possible. Avoid symlink workarounds; they make audit
evidence harder to reproduce.

## Nova Metadata and Agent Evals

Use `dbt-nova eval run` to test that a manifest exposes the right search,
indicator, context, lineage, recipe, and metadata-score evidence to agents:

```bash
dbt-nova eval validate --suite evals/starter.yml

dbt-nova eval run \
  --suite evals/starter.yml \
  --manifest-path tests/fixtures/starter_eval_manifest.json \
  --output-dir out/nova-evals \
  --fail-under 0.95 \
  --json
```

Use `dbt-nova eval agent run` for slower provider-backed smoke tests that verify
how agents use Nova tools in practice:

```bash
dbt-nova eval agent run \
  --suite evals/starter.yml \
  --provider opencode \
  --manifest-path tests/fixtures/starter_eval_manifest.json \
  --timeout-secs 600
```

The agent provider must already be configured with dbt-nova MCP/CLI tools. Nova
scores required tool calls, forbidden tool calls, order constraints, selected
entities, and final-answer text checks from sanitized local trace rows. When a
remote hosted MCP endpoint cannot inherit `DBT_NOVA_TRACE_TOOL_CALLS_PATH`,
supported provider presets can also score MCP calls from JSON event streams.
Use `--case-id <ID>` on bridge or agent eval runs to iterate on one failing case
without re-running the whole suite.

Pass `--telemetry` on bridge or agent eval runs to append per-assertion JSONL
rows under `.nova/eval-runs/telemetry/<suite>-<hash>.jsonl`. Use
`dbt-nova eval history --suite <NAME> --since <YYYY-MM-DD>` to print matching
rows when comparing whether metadata or prompt changes improved a suite over
time. Add `--telemetry-retention <ROWS>` to keep only the newest rows for that
suite after each run.

For PR evidence on ranking, skill, or metadata changes, run the same suite into
two output directories and compare them with
`dbt-nova eval compare --before <DIR> --after <DIR>`. The comparison prints
Markdown with pass-rate movement, newly passing/failing cases, and agent
tool-call or token deltas when trace artifacts are available.

When a suite declares `gate.threshold`, run the full suite with `--telemetry`
and then run `dbt-nova eval gate <NAME> --json`. Configured gates reject latest
telemetry from stale suite files, filtered `--case-id` runs, or row-retention
truncation, because those runs do not prove current full-suite readiness. Gates
are advisory in v1: use blocked results to warn before high-stakes analysis or
launch-readiness claims, then inspect the reported failed case/assertion ids.

For release or PR evidence that combines metadata scores, readiness bands,
blocker categories, optional eval gate output, and JSON/Markdown artifacts, see
[Agent Readiness Audit](../features/agent-readiness.md).

For production GitHub Actions snippets that run bridge evals as PR gates and
provider-backed OpenCode evals as private or scheduled advisory checks, see
[Eval CI Templates](eval-ci-templates.md). Those templates include manifest
generation, telemetry retention, explicit `eval gate` failure checks, artifact
upload paths, and redacted trace handling.

## Test Storage Cleanup

Tests use temporary storage under `target/dbt-nova-tests` and clean up automatically.

Startup cleanup (one‑time per test run):
- Removes stale temp dirs older than **5 minutes**.
- Controlled by:
  - `DBT_NOVA_TEST_CLEANUP_AGE_SECS=0` → remove all temp dirs on start
  - `DBT_NOVA_TEST_CLEANUP_ALL=1` → force full cleanup

These variables are **test-only** and are ignored in production runs.

Manual cleanup:
```bash
rm -rf target/dbt-nova-tests
```

## Fuzzing (Monthly Schedule)

Short fuzz runs are scheduled in CI on the first day of each month. To run locally:

```bash
cargo install cargo-fuzz
cargo fuzz run manifest_entity -- -max_total_time=120
cargo fuzz run nova_meta_yaml -- -max_total_time=120
cargo fuzz run sql_validation -- -max_total_time=120
cargo fuzz run artifact_archive -- -max_total_time=120
```

## Coverage

Coverage uses `cargo llvm-cov` (runs in CI as a separate job).

```bash
cargo install cargo-llvm-cov
cargo llvm-cov --all-features --workspace --summary-only
```

## Snapshots

Snapshot tests live in `tests/snapshots.rs`. Update snapshots with:

```bash
cargo install cargo-insta
INSTA_UPDATE=always cargo test --test snapshots
```
