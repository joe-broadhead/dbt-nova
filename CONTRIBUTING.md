# Contributing to dbt-nova

Thanks for investing time in dbt-nova. This guide describes the local workflow,
code standards, and the expectations for high-quality PRs.

## Scope

We welcome:
- Bug fixes and performance improvements
- New tools and metadata features
- Documentation improvements and examples
- Tests that increase coverage or catch regressions

If the change is large, open an issue first so we can align on direction.

## Development Setup

```bash
# Clone
git clone https://github.com/joe-broadhead/dbt-nova.git
cd dbt-nova

# Build
cargo build

# Tests
cargo test

# Lint + format
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --check
```

### Docs Preview (optional)

If you touch docs, validate the site:

```bash
mkdocs build --strict
```

## Branching Model

This repo uses a `master`-only release flow:

- **Default branch:** `master`
- **Feature branches:** `feature/<name>` off `master`, PRs target `master`
- **Release branches:** `release/<version>` off `master`
- **Hotfix branches:** `hotfix/<version>` off `master`

Release flow:

1. Cut `release/<version>` from `master`
2. QA on the release branch
3. Merge release -> `master`, tag `v<version>`

Hotfix flow:

1. Cut `hotfix/<version>` from `master`
2. Merge hotfix -> `master`, tag `v<version>`

Only `master` is tagged for releases. Docs deploy from release tags.

## Code Standards

This repo enforces strict quality rules:

- **Max file size:** 500 LOC (excluding generated code)
- **Max function size:** 50 LOC
- **No `.expect()` in production code** (use `?` or recover gracefully)
- **No silently ignored errors** (log or propagate)
- **Prefer ASCII** in source and docs unless non-ASCII is required

### Error Handling

Prefer returning a structured `DbtNovaError` over panic or silent fallback. If a
fallback is required, log at least a warning with context.

### Performance

Hot paths should avoid allocations when possible (zero-copy where available).
If you add a new hot-path function, consider a small benchmark or targeted test.

## Schema and Metadata

Nova metadata is defined in `schemas/nova/v0.json`. If you add or change fields:

- Update the schema file.
- Update docs in `docs/features/` and `docs/configuration/`.
- Add tests for metadata scoring or validation where relevant.

## Testing Guidance

All new behavior should be covered with tests. Avoid `unwrap_or(false)` or
implicit assertions. Prefer explicit checks and descriptive failure messages.

Recommended commands:

```bash
cargo test
cargo test --all-targets
```

## Versioned Artifacts

- **Cargo.lock** is committed (binary crate, reproducible builds).
- **benches/** is committed (performance baselines).

## Pull Request Checklist

Before submitting:

- [ ] Tests pass (`cargo test`)
- [ ] Lint passes (`cargo clippy --locked --all-targets -- -D warnings`)
- [ ] Formatting is clean (`cargo fmt --check`)
- [ ] Docs updated (if user-facing)
- [ ] `CHANGELOG.md` updated (if behavior or API changed)
- [ ] No new `.expect()` in production code
- [ ] No silently ignored errors

## Commit Messages

Use conventional commits:

- `feat:` New features
- `fix:` Bug fixes
- `docs:` Documentation changes
- `refactor:` Code refactoring
- `test:` Test additions/changes
- `chore:` Maintenance tasks

## Security

If you believe you found a security issue, avoid public issues. Use GitHub
Security Advisories if available for this repository.
