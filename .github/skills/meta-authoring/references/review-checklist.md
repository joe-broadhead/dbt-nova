# Metadata Review Checklist

Use this before final handoff.

## Classification

- The resource is classified as canonical dataset, helper/staging/intermediate, metric template, source, or column annotation.
- The chosen classification matches SQL, grain, lineage, and current search behavior.
- Helpers are not marked canonical unless they are truly the preferred surface.
- Canonical analyst resources are not de-ranked with `search.candidates.analyst: false`.

## Minimum Viable Metadata

- Canonical datasets have stable routing fields, grain, and reusable measures when they own aggregations.
- Metric templates use `metric` or `metrics`, include an expression and grain, and do not hardcode one-off report filters.
- Sources and staging resources stay sparse but include governance when sensitivity or PII is present.
- Column annotations are limited to identifiers, time fields, high-signal dimensions, measures, and genuine disambiguation.
- Synonyms are concise and business-facing.
- Domain references keep narrative guidance out of `meta.nova` while still
  linking back to `domains`, `use_cases`, `grain`, `measures`, `metric(s)`, and
  `governance`.

## Canonicality And Reuse

- Repeated terms were checked through indicator and column search.
- Only one preferred owner is canonical at the relevant level unless the domain/grain difference is explicit.
- Existing canonical definitions are reused instead of copied.
- Duplicate or helper variants remain searchable but rank lower for the intended persona.
- Queryable indicators have an explicit execution surface:
  `relation_name` for relation-backed rows or the configured Semantic Layer path
  for `metricflow` rows.
- Metadata-only cross-grain KPIs are not presented as executable surfaces.

## Field Integrity

- `grain.primary_key`, `grain.time_field`, and `grain.dimensions` exist on the model output.
- `measures[].field` exists when specified.
- `recommended_filters[].field` exists when specified.
- Measure types are one of: `sum`, `count`, `avg`, `min`, `max`, `count_distinct`, `ratio`.
- Column `role` values are one of: `dimension`, `measure`, `metric`, `identifier`, `time`.

## Governance

- Dataset-level `governance` reflects sensitivity, PII, and compliance posture.
- Nested PII is not ignored just because the sensitive fields are inside a struct.
- Column-level governance is added only where a specific exposed field needs it.
- Search-candidate hints are not treated as access control.

## Verification

- Local YAML/schema/audit validation was run when files were edited, or the blocker is reported.
- Search verification was run against a manifest that includes the change, or is explicitly marked pending deployment.
- `search_indicator` or `search` returns the expected canonical result for key terms.
- `get_entity`, `get_context`, or `get_columns` confirms the final contract.
- `get_metadata_score` improves or any non-score tradeoff is explained.
- `modelling_consistency_report` is clean or intentionally documented for
  repeated-indicator, canonical, or cross-grain changes.
- Domain reference changes were validated with skill installation and docs build
  when the packaged template or docs changed.
