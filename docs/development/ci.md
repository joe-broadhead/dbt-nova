# CI & Automation

This repository uses GitHub Actions for CI, releases, and documentation.

## Workflows

### CI

- **File:** `.github/workflows/ci.yml`
- **Trigger:** push on `master` and all pull requests
- **Action:** `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings`,
  `cargo test --locked`, `cargo check --locked --no-default-features --all-targets`,
  `DBT_NOVA_EVAL_ENABLE_HYBRID=0 DBT_NOVA_EVAL_ENABLE_LIFECYCLE=0 DBT_NOVA_EVAL_ALLOW_EMBEDDING_DOWNLOAD=0 cargo test --locked --test search_eval compare_lexical_vs_hybrid_search_quality -- --ignored`,
  `cargo llvm-cov --locked --all-features --workspace --summary-only`,
  `mkdocs build --strict` (with `docs/requirements.txt`), `scripts/check_advisory_ignores.sh`,
  `scripts/check_config_reference.sh`, and `cargo deny check advisories licenses sources`
- **Note:** sets `DBT_NOVA_STRICT_SCHEMA=1` so schema parsing failures break the build

### Prepare Release

- **File:** `.github/workflows/release-prepare.yml`
- **Trigger:** manual (`workflow_dispatch`)
- **Input:** `version` (must be semantic `x.y.z`, e.g. `1.2.3`)
- **Action:** creates `release/<version>` from `master` and opens a PR to `master`

### Tag Release

- **File:** `.github/workflows/release-tag.yml`
- **Trigger:** PR merged into `master`
- **Gate:** head branch is `release/<version>` or `hotfix/<version>`
- **Action:** creates and pushes tag `v<version>` on the merge commit
- **Requirement:** `RELEASE_TAG_TOKEN` secret (PAT or GitHub App token with
  `contents:write`) so tag pushes trigger downstream workflows

### Release Build

- **File:** `.github/workflows/release.yml`
- **Trigger:** `v*` tag push (or manual `workflow_dispatch` with `tag` input)
- **Action:**
  - validates tag is on `master`
  - runs one all-features Linux test gate
  - builds and publishes **slim** + **bundled** assets for `linux-x86_64` and `macos-arm64`

### Docs Deploy

- **File:** `.github/workflows/docs.yml`
- **Trigger:** `v*` tag push
- **Action:** builds MkDocs and publishes to GitHub Pages

## Required Permissions

These workflows use `GITHUB_TOKEN` with:

- `contents: write` for tagging/releases
- `pull-requests: write` for release PR creation
- `pages: write` and `id-token: write` for docs deploy
- `attestations: write` for provenance attestations when supported

Additional secret required:

- `RELEASE_TAG_TOKEN` for `.github/workflows/release-tag.yml`

## Nightly Jobs

- **File:** `.github/workflows/nightly.yml`
- **Trigger:** scheduled + manual
- **Action:** short fuzz run (`cargo fuzz`) and `cargo deny` checks
- **Security guard:** advisory ignore metadata/expiry check (`scripts/check_advisory_ignores.sh`)

## Branch Expectations

- Default and release branch: `master`
- Release/hotfix branches are cut from `master`
- Release tags (`v*`) drive both binary artifacts and docs deployment

## Local Checks (Suggested)

Run these before opening a release PR:

```bash
cargo test --locked
cargo check --locked --no-default-features --all-targets
cargo clippy --locked --all-targets -- -W clippy::all -W clippy::pedantic
cargo fmt --check
scripts/check_config_reference.sh
pip install -r docs/requirements.txt
mkdocs build --strict
cargo deny check
```

## Release Flow Diagram

```mermaid
flowchart TD
  A[master] -->|workflow_dispatch: Prepare Release| B[release/<version>]
  B -->|PR to master| C[master]
  H[hotfix/<version>] -->|PR to master| C
  C -->|merge PR| D[Tag Release v<version>]
  D -->|push v* tag| E[Release Build: artifacts]
  D -->|push v* tag| F[Docs Deploy: GitHub Pages]
```

## Hotfix Checklist

- [ ] Create `hotfix/<version>` from `master`
- [ ] Add fix and update tests/docs as needed
- [ ] Ensure `cargo test` passes
- [ ] Open PR to `master` and merge
- [ ] Tag auto-created (`v<version>`)
- [ ] Verify release assets and docs deploy
