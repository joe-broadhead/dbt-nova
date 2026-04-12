---
name: nova-meta-authoring
description: "Builds and reviews high-signal `meta.nova` blocks in dbt YAML. Use when adding or updating Nova metadata, choosing canonical datasets or measures, defining model-bound measures or metric templates, marking helper models with persona search hints, or preparing a dbt project for stronger Nova search and metadata scoring."
license: MIT
allowed-tools: "mcp__nova__search mcp__nova__get_entity mcp__nova__get_context mcp__nova__get_columns mcp__nova__get_sql mcp__nova__get_lineage mcp__nova__get_metadata_score mcp__nova__find_by_path Read"
metadata:
  owner: "dbt-nova"
  persona: "authoring"
  version: "0.0.2"
---

# Nova Meta Authoring

## When to use

Use this skill when you are creating, reviewing, or refactoring `meta.nova` in a dbt project.

Typical triggers:
- add Nova metadata to a new model or source
- improve weak or noisy metadata on an existing model
- choose the canonical dataset, measure, or metric definition for a business concept
- define helper models that should remain searchable for engineers but rank lower for analysts
- add model-bound `measures` or `metric` / `metrics` so Nova search can surface the right table and formula

## Core workflow (required)

1) Classify the entity before editing
- Read the SQL and schema YAML together.
- Decide whether the entity is:
  - canonical analyst-facing dataset
  - helper / ops / staging / intermediate model
  - metric template model
  - source needing semantic hints
  - column needing semantic disambiguation

2) Choose the right Nova surface
- Use base entity fields for stable routing and discovery:
  - `canonical`, `domains`, `use_cases`, `synonyms`, `grain`, `governance`
- Use `measures` when the model owns reusable aggregations on the execution dataset.
- Use `metric` or `metrics` when the model is a reusable KPI template that analysts adapt by time window, filters, and breakdowns.
- Use column-level `meta.nova` only for identifiers, time fields, high-signal dimensions, or semantic disambiguation.
- Use `search.candidates.analyst: false` for helper or ops models that should remain searchable but rank lower for analysts.

3) Add only stable intent
- Add metadata that dbt cannot derive.
- Do not encode report-specific time windows, one-off slices, or volatile operational details.
- Prefer model-bound Nova `measures` and `metric` / `metrics` on the canonical execution model for analyst discovery.
- If the repo also uses native dbt `metric` resources, they can coexist, but they should not be the only place where the business definition lives.

4) Choose canonical sources deliberately
- Use entity-level `canonical: true` for the preferred analyst-facing dataset.
- Use per-measure or per-metric `canonical: true` when the same business term appears in multiple places but one definition should rank first.
- Only one definition should be the preferred source for a repeated business term like `gmv`, `sessions`, or `average_order_value`.
- Do not mark every duplicate as canonical.

5) Validate structure and semantics
- If the repo has `schemas/nova/v0.json`, validate changed YAML against it.
- Review the change against `references/review-checklist.md`.
- Use `references/decision-rules.md` when deciding between `measures`, `metric`, `metrics`, column metadata, or search candidate hints.
- Use `references/patterns.md` for copyable model, metric, source, and helper patterns.

6) Verify search behavior when Nova tooling is available
- Run `search` with `persona: "analyst"` for the key business terms.
- Confirm the canonical entity ranks above helper variants and duplicate definitions.
- Confirm the search result exposes the matched measure or metric through `semantic_preview`, including the expression when present.
- Use `get_context` or `get_entity` to verify the final selected definition and grain.
- Use `get_metadata_score` to confirm the metadata quality improves without adding noise.

## Authoring rules

- Keep `meta.nova` small and stable.
- Use 2–8 high-signal `synonyms`; prefer business phrasing over technical noise.
- `grain.dimensions` should represent default analyst breakdowns, not every possible dimension.
- Put `measures` on the model where the data lives.
- Use `metric` or `metrics` for reusable KPI templates, not hardcoded business answers.
- Be selective with column-level Nova metadata; use it for identifiers, time fields, high-signal dimensions, and disambiguation.
- Preserve existing repo conventions unless they are clearly low-signal or internally inconsistent.

## Output expectations

When you finish, make the reasoning explicit:
- why this entity is or is not canonical
- why a repeated business term is canonical at the entity, measure, or metric level
- why a helper model was deboosted for analysts
- what validations were run
- which search terms were checked and whether the canonical definition surfaced first

## References

- `references/decision-rules.md`
- `references/patterns.md`
- `references/review-checklist.md`
