---
name: mcp-engineer
description: "Builds and modifies dbt models through Nova MCP tools with production-quality gates. Use when changing model SQL, grain, columns, or semantic contracts and you need impact analysis, SQL inspection, metadata/test gates, and manifest readiness checks."
license: MIT
allowed-tools: "mcp__nova__search mcp__nova__find_by_path mcp__nova__get_entity mcp__nova__get_columns mcp__nova__get_sql mcp__nova__get_lineage mcp__nova__get_column_lineage mcp__nova__get_context mcp__nova__get_impact mcp__nova__get_test_coverage mcp__nova__get_metadata_score mcp__nova__get_undocumented mcp__nova__reload_manifest mcp__nova__health mcp__nova__diff_entities mcp__nova__validate_dag Read"
metadata:
  owner: "dbt-nova"
  persona: "engineer"
  version: "0.0.3"
---

# MCP Engineer Skill (dbt-nova)

## Mission

Ship production-safe dbt changes with explicit blast-radius, quality, and readiness checks.

## Core workflow (required)

1. Preflight
- Run `health`.
- If status is not `ready`, run `reload_manifest` and wait for readiness.

2. Discover the implementation target
- Prefer:
  - `search` with `persona: "engineer"`
  - `find_by_path` when the file area is already known
- Prefer reuse or extension before adding new models.

3. Validate upstream inputs
- Use:
  - `get_entity`
  - `get_columns`
  - `get_sql`
  - `get_context` with `context_mode: "engineer"` for fast triage
- Default `get_sql` mode:
  - `compiled: false`
- Use `compiled: true` only when you need rendered SQL and the manifest actually contains it.

4. Run blast-radius analysis
- Use:
  - `get_lineage`
  - `get_impact`
  - `get_column_lineage` for critical fields
- Filter lineage to models only when tests would add noise.

5. Run quality gates
- Use:
  - `get_test_coverage`
  - `get_metadata_score`
  - `get_undocumented`
  - `validate_dag`
- Use `validate_dag` with `detail: "summary"` unless you are actively debugging graph defects.

6. Refresh after changes
- After dbt compile/build updates the manifest, run `reload_manifest` again and re-check `health`.

7. Report the ship checklist
- Include:
  - target model and grain
  - changed columns or logic
  - downstream impact
  - test gaps
  - metadata score
  - manifest readiness

## Tool usage (quick map)

- `search` / `find_by_path`: target discovery
- `get_entity` / `get_columns` / `get_sql`: contract and SQL inspection
- `get_context`: fast triage bundle
- `get_lineage` / `get_impact` / `get_column_lineage`: blast radius
- `get_test_coverage` / `get_metadata_score` / `get_undocumented`: quality gates
- `diff_entities` / `validate_dag`: change verification
- `reload_manifest` / `health`: lifecycle and readiness

## Output standard (required)

Provide a ship checklist:
- model name + grain
- selection rationale
- columns added or changed
- tests added or required
- downstream impact summary
- metadata score and missing fields
- manifest readiness

## Validation checklist (copy and complete)

[ ] Manifest ready before analysis
[ ] Target entity selected and justified
[ ] Upstream inputs validated
[ ] Impact analysis reviewed
[ ] Tests checked
[ ] Documentation / metadata score checked
[ ] Manifest reloaded after changes
[ ] Ship checklist completed

## References

- `references/engineering-workflow.md`
- `references/tool-recipes.md`
- `references/manifest-refresh.md`
