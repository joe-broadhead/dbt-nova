# Dependency Watchlist

Nova tracks a small set of dependency constraints that are known and intentional.
These are not unowned TODOs: each entry has an owner, review date, and explicit
upgrade trigger.

## Source of Truth

Machine-readable watchlist:

- `dependency-watchlist.toml`

Validation script:

- `scripts/check_dependency_watchlist.sh`

CI and monthly automation fail if:

- `review_by` is expired
- required metadata is missing
- tracked state no longer matches dependency reality

## Active Entries

### `ort-sys` exact RC pin

- **Current state:** `Cargo.toml` pins `ort-sys = "=2.0.0-rc.4"`.
- **Why:** current `fastembed` compatibility requires this pin.
- **Upgrade trigger:** stable (non-RC) compatibility is available across the
  `fastembed`/`ort` stack.
- **Upgrade criteria:**
  - remove exact RC pin
  - update lockfile
  - run full CI and release build checks

### `reqwest` transitive split (`0.11` + `0.12` + `0.13`)

- **Current state:** all three versions are present in `Cargo.lock`.
- **Why:** `google-cloud-storage` transitively pulls `reqwest 0.11` while Nova
  directly uses `reqwest 0.12`; `jsonschema` currently pulls `reqwest 0.13`.
- **Upgrade trigger:** upstream GCS and `jsonschema` stacks converge on
  compatible `reqwest` major versions.
- **Upgrade criteria:**
  - upgrade GCS/jsonschema dependencies where possible
  - keep `cargo deny` green
  - pass provider integration tests
  - collapse to the smallest supported `reqwest` version set in lockfile

## Local Verification

```bash
scripts/check_dependency_watchlist.sh
```

If the script fails because state changed, update dependencies and refresh
`dependency-watchlist.toml` in the same PR.
