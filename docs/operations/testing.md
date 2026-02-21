# Testing & Cleanup

## Run Tests

```bash
cargo test
cargo test -- --nocapture
```

## Test Layout

- `src/tests/` – in‑crate unit tests
- `tests/` – integration/property tests and fixtures

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
cargo fuzz run manifest_entity -- -max_total_time=60
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
