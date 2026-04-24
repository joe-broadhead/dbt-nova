# Metadata Decision Rules

## Entity Metadata

Use entity-level `meta.nova` for facts about the dataset or source:
- `canonical`
- `tier`
- `domains`
- `use_cases`
- `synonyms`
- `grain`
- `governance`

Entity metadata should answer: what is this resource, who should use it, at what grain, and with what governance constraints?

## Measures

Use `measures` when the aggregation belongs to the execution model and analysts should discover the formula from that dataset.

Good measure candidates:
- GMV, revenue, margin, orders, sessions, customers, units, product counts

A strong measure normally has:
- `name`, `type`, `expression`, `description`
- `field` when one physical output column is the source
- `synonyms` for real business vocabulary
- `canonical: true` only on the preferred repeated definition

Allowed types: `sum`, `count`, `avg`, `min`, `max`, `count_distinct`, `ratio`.

## Metrics

Use `metric` or `metrics` when the model represents a reusable KPI template that analysts adapt by time window, filters, and breakdowns.

Good metric candidates:
- average order value
- conversion rate
- margin rate
- revenue per session
- return rate

A strong metric normally has:
- `name`, `description`, `expression`, `template: true`
- `grain`
- optional `recommended_filters`
- `canonical: true` only on the preferred repeated KPI definition

Never set both `metric` and `metrics` on the same entity.

## Column Metadata

Use column-level `meta.nova` selectively for:
- identifiers
- time fields
- high-signal dimensions
- measure columns that need business naming
- ambiguous fields where synonyms or semantic type materially improve search
- exposed PII fields that need explicit governance

Do not annotate every column. Repeated low-signal annotations make search noisier and metadata harder to maintain.

Allowed `role` values: `dimension`, `measure`, `metric`, `identifier`, `time`.

## Search Candidates

Use `search.candidates.analyst: false` when a resource is useful for engineers or governance but should not be the analyst landing page.

Good candidates:
- staging models
- intermediate/helper models
- operational or debugging resources
- duplicate variants kept for lineage or migration

This is a ranking hint, not a security control or filter. Leave it absent unless there is a genuine audience exception.

## Canonicality

Use canonical flags to break ties, not to advertise importance everywhere.

- Entity-level canonical: preferred analyst-facing dataset.
- Measure-level canonical: preferred repeated aggregation.
- Metric-level canonical: preferred repeated KPI template.

If entity-level and indicator-level canonicality exist for the same concept, they should point in the same direction. A repeated term like `gmv` or `average_order_value` should not have multiple equally preferred owners unless the grains or domains are intentionally different and clearly named.

## Governance

Use entity-level `governance` for dataset sensitivity, PII posture, and compliance requirements. Use column-level governance when a specific exposed column is sensitive and the dataset-level signal is insufficient.

For nested structs containing customer names, emails, addresses, phone numbers, tax numbers, or national identifiers, record the dataset-level PII and compliance posture even if individual nested fields cannot be annotated cleanly.

## Search Verification

For authored repeated business terms:
1. search the term and its main synonyms with the intended persona
2. confirm the preferred entity or indicator ranks ahead of helper variants
3. inspect the surfaced contract with `get_entity` or `get_context`
4. confirm referenced fields exist with `get_columns`
5. confirm metadata score improves or the tradeoff is intentional
