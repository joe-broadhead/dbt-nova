# Release & Distribution

## Release Flow

Releases use a `master`-only flow:

1. Create `release/<version>` from `master`
2. Run final QA on the release branch
3. Merge release -> `master` and tag `v<version>`

Hotfixes follow the same pattern from `master` via `hotfix/<version>`.

Docs are deployed from release tags (`v*`).

The release workflow enforces this policy:

- Tags must point at a commit on `master`

## Release Checklist

Before tagging:

- [ ] `release/<version>` branch cut from `master`
- [ ] `CHANGELOG.md` updated for the version
- [ ] `Cargo.toml` version bumped
- [ ] `cargo test` passes
- [ ] `cargo clippy -- -W clippy::all -W clippy::pedantic` passes
- [ ] `cargo fmt --check` passes
- [ ] Docs build clean (`mkdocs build --strict`)
- [ ] Release PR approved and merged to `master`

After merge to `master`:

- [ ] Tagging happens automatically when the release PR is merged
- [ ] Verify release assets on GitHub

## CI & Automation

This repo uses four GitHub Actions workflows for releases and documentation:

1. **Prepare Release** (`.github/workflows/release-prepare.yml`)
   - Trigger: manual (`workflow_dispatch`)
   - Inputs: `version` (must be semantic `x.y.z`, e.g. `1.2.3`)
   - Actions: creates `release/<version>` off `master` and opens a PR to `master`

2. **Tag Release** (`.github/workflows/release-tag.yml`)
   - Trigger: PR merged into `master`
   - Gate: head branch name must be `release/<version>` or `hotfix/<version>`
   - Actions: creates and pushes tag `v<version>` on the merge commit
   - Requirement: repo secret `RELEASE_TAG_TOKEN` (PAT or GitHub App token with
     `contents:write`) must be configured. The default Actions `GITHUB_TOKEN`
     does not trigger downstream workflows on tag push.

3. **Release Build** (`.github/workflows/release.yml`)
   - Trigger: `v*` tag push
   - Manual fallback: `workflow_dispatch` with `tag` input (useful when a tag exists
     but was pushed in a way that did not trigger downstream workflows)
   - Actions:
     - validates tag is on `master`
     - runs one all-features test gate on Linux before packaging
     - builds and publishes slim assets for `linux-x86_64` (Cloud Run)
       and `macos-arm64` (standard Apple Silicon macOS)
     - builds, smokes, and publishes a `linux/amd64` OCI image to GHCR

4. **Docs Deploy** (`.github/workflows/docs.yml`)
   - Trigger: `v*` tag push
   - Actions: builds MkDocs and publishes to GitHub Pages

Release jobs now generate and publish checksum and cosign artifacts for every released platform,
emit GitHub artifact provenance attestations for release files, and publish an
OCI image to `ghcr.io/joe-broadhead/dbt-nova`.

Note: GitHub provenance attestations are only emitted when supported by the repository plan/type.
For user-owned private repositories, the attestation step is skipped.

- `release.yml` emits `dbt-nova-<asset>.sha256` for each platform tarball.
- The checksum files are uploaded with slim assets.
- Signature files are also uploaded:
  - `<tarball>.sig` / `<tarball>.crt`
  - `<checksum_file>.sig` / `<checksum_file>.crt` (for example: `dbt-nova-linux-x86_64.sha256.sig`)

The installer validates checksum signatures by default when `DBT_NOVA_VERIFY_SIGNATURE=1`.

Verify download integrity with:

```bash
gh release download <tag> --repo joe-broadhead/dbt-nova \
  --pattern dbt-nova-linux-x86_64.tar.gz --output dbt-nova-linux-x86_64.tar.gz
gh release download <tag> --repo joe-broadhead/dbt-nova \
  --pattern dbt-nova-linux-x86_64.sha256 --output dbt-nova-linux-x86_64.sha256
sha256sum -c dbt-nova-linux-x86_64.sha256
```

Verify provenance with Sigstore if `cosign` is installed:

```bash
gh release download <tag> --repo joe-broadhead/dbt-nova \
  --pattern dbt-nova-linux-x86_64.tar.gz.sig --output dbt-nova-linux-x86_64.tar.gz.sig
gh release download <tag> --repo joe-broadhead/dbt-nova \
  --pattern dbt-nova-linux-x86_64.tar.gz.crt --output dbt-nova-linux-x86_64.tar.gz.crt
cosign verify-blob \
  --signature dbt-nova-linux-x86_64.tar.gz.sig \
  --certificate dbt-nova-linux-x86_64.tar.gz.crt \
  dbt-nova-linux-x86_64.tar.gz
```

If your GitHub CLI is configured, you can also verify the GitHub release attestation:

```bash
gh attestation verify dbt-nova-linux-x86_64.tar.gz \
  --owner joe-broadhead
```

## Release Type

- **Slim**: binary only (downloads models on first run)
- **OCI image**: hosted/server runtime image for streamable HTTP deployments

Pull a released container image with:

```bash
docker pull ghcr.io/joe-broadhead/dbt-nova:v<version>
```

Every release also publishes an immutable commit tag:

```text
ghcr.io/joe-broadhead/dbt-nova:sha-<git-sha>
```

By default, releases include S3/GCS SDK support. If you need a minimal binary,
build with `--no-default-features` and enable only the features you need.

## Installer Script

```bash
DBT_NOVA_REPO=joe-broadhead/dbt-nova bash scripts/install.sh
```

The installer defaults to **slim**, supports bundled when available, and places
the binary in `~/.local/bin` by default.

Useful overrides:

- `DBT_NOVA_INSTALL_FLAVOR=bundled|slim`
- `DBT_NOVA_INSTALL_WARM_MODELS=1`
- `DBT_NOVA_EMBEDDINGS_CACHE_DIR=/path/to/models`
- `DBT_NOVA_WARMUP_REQUIRED_MODELS=3`
- `DBT_NOVA_INSTALL_SKILLS=1`
- `DBT_NOVA_SKILLS_DIR=/custom/skills/path`
- `DBT_NOVA_INSTALL_NONINTERACTIVE=1`
- `DBT_NOVA_INSTALL_DIR=/custom/path`
- `DBT_NOVA_VERIFY_CHECKSUM=1|0`
- `DBT_NOVA_VERIFY_SIGNATURE=1|0`
- `DBT_NOVA_COSIGN_BINARY=cosign`
- `--bundled`, `--slim`, `--warm-models`, `--install-skills`, `--skills-dir <path>`, `--non-interactive`, `--install-dir <path>`

## Packaging Notes

- Slim artifacts download models into the configured cache directory.
