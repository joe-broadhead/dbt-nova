# Nova Meta Review Checklist

## Entity classification

Before editing, confirm which case applies:
- canonical analyst-facing dataset
- helper / ops / staging / intermediate model
- metric template model
- source needing discovery or governance hints
- column needing semantic disambiguation

If the classification is wrong, the metadata will usually be wrong.

## Minimum viable metadata

### Canonical analyst-facing model
Expected fields:
- `canonical: true`
- `domains`
- `use_cases`
- `synonyms`
- `grain.primary_key`
- `grain.time_field`
- optional `grain.dimensions`
- at least one meaningful `measure` when the model owns reusable aggregations

### Helper / ops / intermediate model
Expected fields:
- only the minimum stable intent
- `search.candidates.analyst: false` when analysts should not land here first
- no forced canonical flags unless the model truly is the preferred source

### Metric template model
Expected fields:
- model-level routing fields where useful
- `metric` or `metrics`
- `template: true`
- expression aligned with the SQL model output
- canonical metric flag only on the preferred repeated KPI definition

## Search and canonicality checks

- If the same business term appears in multiple models, choose one preferred definition.
- Use entity-level `canonical: true` for the preferred dataset.
- Use per-measure or per-metric `canonical: true` for the preferred repeated business term.
- Do not mark every duplicate as canonical.
- Do not set `search.candidates.analyst: false` on the canonical analyst-facing model.
- If both entity-level and measure/metric-level canonicality are present, they should point at the same preferred source.

## Search verification checks

When Nova tooling is available:
- run `search` with `persona: "analyst"` for the key business term
- confirm the canonical entity ranks above helper or duplicate variants
- confirm the result exposes `semantic_preview` for the matched measure or metric
- use `get_context` or `get_entity` to verify final grain and expression

## Content quality checks

- Synonyms are concise and high-signal.
- Descriptions explain business meaning, not only SQL mechanics.
- `grain.dimensions` reflect default analyst breakdowns.
- Governance metadata is present only when it changes routing, review, or compliance behavior.
- Column-level metadata is selective and useful.
- Native dbt metrics are not the only place where the business definition lives when the model should drive analyst discovery.

## Anti-patterns

Avoid these:
- exhaustive synonym lists
- report-specific time windows encoded as metric templates
- helper tables marked canonical just because they are upstream
- every column carrying low-signal `meta.nova`
- copying the same measure metadata into every related table
- using `search.candidates.analyst: false` as a substitute for choosing the real canonical model
- relying on a standalone dbt `metric` entity alone when a canonical execution model should carry the reusable Nova semantic definition

## Validation

If the repo supports validation, do all of the following:
- validate changed YAML against `schemas/nova/v0.json`
- run any repo metadata or lint checks
- if Nova tooling is available, inspect search behavior for repeated business terms and confirm the canonical definition surfaces first
- if metadata scoring is available, re-run it and verify the change improves or preserves score quality without adding noise
