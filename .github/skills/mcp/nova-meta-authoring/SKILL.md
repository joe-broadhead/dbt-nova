---
name: mcp-nova-meta-authoring
description: "Builds and reviews high-signal `meta.nova` through Nova MCP tools. Use when choosing canonical datasets, measures, metrics, grain, search hints, and semantic disambiguation, then validating that those choices surface correctly through search and entity contracts."
license: MIT
allowed-tools: "mcp__nova__search mcp__nova__search_indicator mcp__nova__get_entity mcp__nova__get_context mcp__nova__get_columns mcp__nova__get_sql mcp__nova__get_lineage mcp__nova__get_metadata_score mcp__nova__find_by_path mcp__nova__reload_manifest mcp__nova__health Read"
metadata:
  owner: "dbt-nova"
  persona: "authoring"
  version: "0.0.3"
---

# MCP Nova Meta Authoring

## Mission

Author high-signal `meta.nova` and confirm that the resulting semantics surface correctly through Nova search and entity contracts.

## Core workflow (required)

1. Classify the entity before editing
- Read the SQL and schema YAML together.
- Decide whether the entity is:
  - canonical analyst-facing dataset
  - helper / ops / intermediate model
  - metric template model
  - source needing semantic hints
  - column needing semantic disambiguation

2. Choose the right Nova surface
- Use entity-level fields for stable routing and discovery:
  - `canonical`
  - `domains`
  - `use_cases`
  - `synonyms`
  - `grain`
  - `governance`
- Use `measures` when the model owns reusable aggregations on the execution dataset.
- Use `metric` / `metrics` for reusable KPI templates.
- Never set both `metric` and `metrics` on the same entity.
- Use column-level metadata only for identifiers, time fields, high-signal dimensions, or semantic disambiguation.
- Use search candidate hints only for exceptions that should remain searchable but rank lower for analysts.

3. Add only stable intent
- Add metadata that dbt cannot derive.
- Do not encode report-specific windows or one-off business slices.
- Prefer canonical measures and metrics on the real execution model.

4. Choose canonical definitions deliberately
- Use entity-level `canonical: true` for the preferred analyst-facing dataset.
- Use per-measure or per-metric `canonical: true` only when repeated business terms appear in multiple places and one should rank first.
- Do not mark every duplicate as canonical.

5. Refresh before validating behavior
- After dbt compile/build updates the manifest, run:
  - `reload_manifest`
  - `health`

6. Verify authored behavior through the MCP surface
- Use:
  - `search_indicator` for authored measures and metrics
  - `search` for broader entity discovery
  - `get_entity` with `detail: "standard"` for the compact semantic contract
  - `get_columns` for referenced field checks
  - `get_metadata_score` for metadata quality impact

## Important boundary

- MCP currently does not expose the CLI-only `audit nova-meta` validator.
- Use this skill to validate search and contract behavior through MCP.
- Use the CLI validator separately when you need schema and local semantic validation against `schemas/nova/v0.json`.

## Authoring rules

- Keep `meta.nova` small and stable.
- Prefer business phrasing over technical noise in `synonyms`.
- `grain.dimensions` should represent default analyst breakdowns, not every possible dimension.
- Put measures on the model where the data lives.
- Use metrics for reusable KPI templates, not hardcoded business answers.
- Be selective with column-level metadata; use it for identifiers, time fields, high-signal dimensions, and disambiguation.

## Output expectations

When you finish, make the reasoning explicit:
- why the entity is or is not canonical
- why a repeated business term is canonical at the entity, measure, or metric level
- which searches were run
- whether the intended canonical definition surfaced first
- which contract fields were verified

## References

- docs site: `https://joe-broadhead.github.io/dbt-nova/`
- overview: `https://joe-broadhead.github.io/dbt-nova/features/nova-meta-overview/`
- models: `https://joe-broadhead.github.io/dbt-nova/features/nova-meta-models/`
- metrics and measures: `https://joe-broadhead.github.io/dbt-nova/features/nova-meta-metrics/`
- search ranking: `https://joe-broadhead.github.io/dbt-nova/features/search-ranking/`
- persona summaries: `https://joe-broadhead.github.io/dbt-nova/features/persona-summaries/`
- `references/decision-rules.md`
- `references/patterns.md`
- `references/review-checklist.md`
