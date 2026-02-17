# Personas Overview

dbt-nova supports three personas that tune search ranking and tool behavior for different use cases.

## Which Persona Should You Use?

| If you need to... | Use persona | Guide |
|-------------------|-------------|-------|
| Find datasets, validate metrics, create reports | `analyst` | [Analyst Guide](analyst.md) |
| Debug models, assess impact, fix tests | `engineer` | [Engineer Guide](engineer.md) |
| Audit compliance, find gaps, track ownership | `governance` | [Governance Guide](governance.md) |

## Persona Effects

Each persona adjusts search ranking weights to prioritize different signals:

| Signal | Analyst | Engineer | Governance |
|--------|---------|----------|------------|
| Business terms (synonyms, measures) | High | Low | Medium |
| Exact name matches | Medium | High | Medium |
| Documentation coverage | High | Low | High |
| Test coverage | Medium | Medium | High |
| File paths | Low | High | Medium |
| Tags | Medium | Low | High |

Set the persona in search requests: `"persona": "analyst"`.

---

# Common Guidance

This section contains shared guidance for all persona workflows.

## Commands

- Format: `cargo fmt`
- Lint: `cargo clippy`
- Test: `cargo test`
- Build: `cargo build --release`

## Project Structure

- `src/tools/` MCP tool implementations.
- `src/manifest/` manifest loading, search, and storage.
- `docs/personas/` persona workflows and schemas.
- `tests/` integration and property tests.

## Code Style

- Use `detail: "standard"` for discovery.
- Use `detail: "full"` (or `get_entity`) only after disambiguation.
- Keep responses concise and include `unique_id` when asking follow-ups.

## Git Workflow

- Do not commit unless explicitly asked.
- Do not rewrite history.

## Boundaries

- Do not modify `manifest.json`.
- Do not use `detail: "full"` in broad searches.

---

## See Also

- [Tools Reference](../api/tools.md) - Complete tool documentation
- [Quick Reference](../api/quick-reference.md) - One-page tool cheatsheet
- [Search Ranking](../features/search-ranking.md) - How persona affects ranking
