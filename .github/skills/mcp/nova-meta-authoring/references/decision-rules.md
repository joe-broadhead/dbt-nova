# Nova Meta Decision Rules

## When to use each surface

### Base entity metadata
Use these on models or sources for stable routing and discovery:
- `canonical`
- `domains`
- `use_cases`
- `synonyms`
- `grain`
- `governance`

Use them when the information describes the dataset itself.

### `measures`
Use `measures` when:
- the aggregation belongs to the execution model
- analysts should discover the formula directly from the dataset
- the business term is reused across many questions

Good examples:
- `gmv`
- `sessions`
- `orders`
- `active_users`

A measure should normally include:
- `name`
- `type`
- `expression`
- `description`
- `field` when there is a single physical source column
- `synonyms` when business vocabulary varies
- `canonical: true` only on the preferred repeated definition

Allowed measure types are:
- `sum`
- `count`
- `avg`
- `min`
- `max`
- `count_distinct`
- `ratio`

### `metric` or `metrics`
Use `metric` or `metrics` when:
- the model is a KPI template
- analysts will adapt time window, filters, and breakdowns
- the logic is more than a simple aggregation on one field

Good examples:
- `average_order_value`
- `conversion_rate`
- `revenue_per_session`

A metric should normally include:
- `name`
- `description`
- `expression`
- `template: true`
- `grain`
- optional `recommended_filters`
- `canonical: true` only on the preferred repeated KPI definition

Never set both `metric` and `metrics` on the same entity.

### Column-level `meta.nova`
Use this only when the column needs semantic help:
- identifier
- time field
- high-signal business dimension
- business alias that search should understand

Do not annotate every column by default.

### `search.candidates`
Use `search.candidates.analyst: false` when:
- the model is useful for engineering or governance
- analysts should not land on it first
- the model is helper, ops, staging, or intermediate

This is a ranking hint, not a filter.
Leave `search.candidates` absent unless there is a real audience exception to encode.

## Canonicality hierarchy

### Entity-level canonical
Use when the dataset itself is the preferred analyst-facing source for the concept.

### Measure-level canonical
Use when the same business term appears as a measure in multiple datasets and one should rank first.

### Metric-level canonical
Use when the same KPI template appears in multiple models and one should rank first.

If both entity-level and measure/metric-level canonicality exist, they should point in the same direction.

## Nova semantics vs standalone dbt metrics

Nova supports both:
- model-bound Nova `measures` and `metric` / `metrics`
- standalone dbt `metric` entities

For analyst discovery, prefer putting the business definition on the canonical execution model with Nova metadata.

Reason:
- Nova search now boosts matched Nova measures and metrics strongly for analysts
- canonical matched Nova semantics can outrank standalone dbt `metric` entities
- search results can expose `semantic_preview` with the matched expression and source entity

Standalone dbt metrics can still coexist, but they should not be the only place where the KPI meaning lives.

Model names themselves do not carry Nova semantics. Prefixes like `mart__`, `fct__`, `dim__`, or `int__` are repo conventions only.

## Search verification rule

After editing metadata for a repeated business term:
1. run `search` with `persona: "analyst"`
2. search the main business terms and synonyms
3. confirm the preferred entity ranks first
4. confirm `semantic_preview` exposes the matched measure or metric definition
5. confirm helper variants remain searchable but rank lower
6. confirm the fields referenced in `grain`, `measure.field`, and `recommended_filters` are present on the model output
