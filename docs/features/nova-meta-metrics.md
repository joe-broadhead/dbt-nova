# Nova Meta: Metric Guide (for Agents)

This guide explains how to define Nova‑ready metric models with high‑signal `meta.nova` metadata.
For the full field map and governance conventions, see
[Nova Meta Overview](nova-meta-overview.md).

## Goals

- Make KPIs discoverable by name and synonym.
- Keep metrics authoritative and consistent.
- Avoid duplicating dependency metadata (dbt already provides it).
- Treat metrics as **canonical templates** that analysts adapt (time window, filters)
  based on the business question.

## Metric Model Pattern

Each metric is a dbt model (SQL file) plus a YAML schema entry.

Metrics are **templates**, not hardcoded answers. Analysts adapt:
- time window (weekly/monthly/YoY)
- filters (country/platform)
- breakdowns (dimensions)

### Single vs Multiple Metrics

Use `metric` when the model represents **one** KPI. If a model intentionally
bundles multiple KPIs, use `metrics` (array) instead of `metric`.

```yaml
meta:
  nova:
    metrics:
      - name: conversion_rate
        canonical: true
        template: true
        description: "Conversions per session."
        expression: "sum(is_converted) / nullif(count(distinct session_id), 0)"
        synonyms: ["conversion rate", "cvr"]
        grain:
          time_field: activity_date
          dimensions: ["country_code", "platform_name"]
      - name: revenue_per_session
        canonical: true
        template: true
        description: "Revenue per session."
        expression: "sum(revenue) / nullif(count(distinct session_id), 0)"
        synonyms: ["rps", "gmv per session"]
        grain:
          time_field: activity_date
          dimensions: ["country_code", "platform_name"]
```

## Reference Schema (Copy/Paste)

```yaml
meta:
  nova:
    canonical: true
    tier: alpha
    domains: ["digital", "product"]
    use_cases: ["weekly_report", "product_analytics"]
    metric:
      name: conversion_rate
      canonical: true
      template: true
      description: "Conversions per session."
      expression: "sum(is_converted) / nullif(count(distinct session_id), 0)"
      synonyms: ["conversion rate", "conversion", "cvr"]
      grain:
        time_field: activity_date
        dimensions: ["country_code", "platform_name", "device_type"]
      recommended_filters:
        - field: platform_name
          operator: "in"
          values: ["web", "app"]
          label: "Digital platforms (overall)"
```

## SQL Conventions (Follow These)

- **Single canonical source**: the metric should reference one base model via `ref()`.
- **Grain alignment**: output columns must match `meta.nova.metric(s).grain` (time + dimensions).
- **Name alignment**: the metric column name should match the model name and `metric.name`.
- **Expression fidelity**: SQL expression must match `metric.expression`.
- **Null‑safe ratios**: use `nullif(denominator, 0)` to avoid divide‑by‑zero.
- **Deterministic grouping**: group by all grain fields.
- **No joins/windowing**: keep metrics simple and owned by a single canonical model.

### SQL Example

`models/metric/product/metric__conversion_rate/metric__conversion_rate.sql`

```sql
with base as (
  select *
  from {{ ref('base__example_activity') }}
)
select
  activity_date,
  country_code,
  platform_name,
  device_type,
  sum(is_converted) / nullif(count(distinct session_id), 0) as conversion_rate
from base
group by 1, 2, 3, 4
```

### YAML Example

```yaml
version: 2
models:
  - name: metric__conversion_rate
    description: "Conversions per session."
    group: product_analytics

    meta:
      nova:
        canonical: true
        tier: alpha
        domains: ["digital", "product"]
        use_cases: ["weekly_report", "product_analytics"]
        metric:
          name: conversion_rate
          canonical: true
          template: true
          description: "Conversions per session."
          expression: "sum(is_converted) / nullif(count(distinct session_id), 0)"
          synonyms: ["conversion rate", "conversion", "cvr"]
          grain:
            time_field: activity_date
            dimensions: ["country_code", "platform_name", "device_type"]
          recommended_filters:
            - field: platform_name
              operator: "in"
              values: ["web", "app"]
              label: "Digital platforms (overall)"
        governance:
          sensitivity: low
          pii: none
          compliance: ["gdpr"]
```

## Recommended Fields (Nova‑Level)

- `canonical` (bool): True for the preferred definition of the KPI.
- `tier` (alpha|beta|gamma): Quality signal for discovery.
- `domains` (list): Broad business domain(s) used for routing.
- `use_cases` (list): Typical analyst questions (e.g., `weekly_report`).

## Recommended Fields (Metric‑Level)

- `name`: Canonical metric name.
- `canonical`: Mark the preferred definition when the same KPI exists on multiple models.
- `template`: Set `true` to signal this model is a reusable template (analysts adapt time/filters).
- `description`: Business definition.
- `expression`: Plain‑text formula (not parsed).
- `synonyms`: Alternate names analysts will search for.
- `grain.time_field`: Primary time column.
- `grain.dimensions`: Default breakdowns.
- `grain.primary_key` (optional): Rarely needed for metrics; use only if it
  clarifies row identity for the metric output.

## Current Ecommerce Nova Conventions (Implemented)

This section reflects what is **already in the manifest** for ecommerce metric models today.
Use it as a reference when extending coverage; update it when you expand Nova meta
to additional domains.

- `canonical`: all current ecommerce metrics are `true`.
- `tier`: all current ecommerce metrics are `alpha`.
- `domains`: `ecommerce`, `web`, `stock`.
- `use_cases` (current set): `campaign_analysis`, `cart_and_reserve`, `checkout_funnel`,
  `consented_analytics`, `conversion_funnel`, `inventory_health`, `product_availability`,
  `product_performance`, `promo_feature_engagement`, `revenue`, `revenue_analysis`,
  `revenue_opportunity`, `stock_availability`, `stockout_analysis`, `weekly_report`,
  `web_analytics`.
- `metric.template`: `true` for all current metrics.
- `metric.canonical`: supported on individual metric definitions when the same KPI
  appears in multiple Nova metric templates and one should rank first.
- `recommended_filters`: present on many metrics; when used it is currently
  `platform_name in ["web", "app"]` with label “Digital platforms (overall)”.
- `governance`: `sensitivity: low`, `pii: none`, `compliance: ["gdpr"]`.

## Governance Meta (Optional, Recommended)

Use the same governance block as model meta when the metric is sensitive:

```yaml
governance:
  sensitivity: low
  pii: none
  compliance: ["gdpr"]
```

See [Nova Meta (Models)](nova-meta-models.md) and
[Nova Meta Overview](nova-meta-overview.md) for enums and definitions.

### Grain Clarification (Important)

`grain.dimensions` should include **only the default breakdowns** analysts are expected to use for the KPI.
Do **not** include every possible dimension in the source model.

Rule of thumb: 2–5 dimensions that define the standard view of the metric.

## Optional Fields (Metric‑Level)

### `recommended_filters` (optional)

Use this only when there is a **strong default slice** that is almost always used.
Prefer `operator: "in"` for the **overall** digital default (e.g., web + app),
then analysts can narrow to a subset (e.g., web only).
Avoid encoding every country/platform variation—filters should remain analyst‑driven.

Analyst agency reminder:
- Metrics are **not** hardcoded answers.
- Analysts should validate filter values via a quick `select distinct` query
  before final analysis.

## Synonyms Conventions (Metric‑Level)

- Prefer **lowercase phrases** for business names (e.g., `"conversion rate"`).
- Add common abbreviations (e.g., `"cvr"`) if used by analysts.
- Use **snake_case** only when it is a common technical alias (e.g., `session_id`).
- Avoid ambiguous synonyms that apply to many domains.

Shape:

```yaml
recommended_filters:
  - field: platform_name
    operator: "in"
    values: ["web", "app"]
    label: "Digital platforms (overall)"
```

## Deterministic Enums (Use These Exact Values)

### `tier` (recommended values)
- `alpha`
- `beta`
- `gamma`
- `gold`
- `silver`
- `bronze`

If your organization uses a different tiering scheme, keep it consistent and
document it in a single shared place.

### `metric(s).recommended_filters[].operator`
- `in`
- `not_in`
- `=`
- `!=`
- `>`
- `>=`
- `<`
- `<=`
- `between`
- `is_null`
- `is_not_null`

### `domains` / `use_cases` (controlled vocabulary)

Treat these as enums within your org. Keep a small, curated list and reuse it consistently.

## What NOT to Include

Do not duplicate:
- upstream dependencies (`depends_on` is in the manifest)
- tests / validation results
- freshness cadence
- materialization details

## Quality Checklist

- Metric SQL references a **canonical** base model.
- Metric name is unique and stable.
- Expression is clear and matches SQL.
- Synonyms cover common business terms.

## Common Pitfalls

- Defining the same metric in multiple models.
- Missing a time field in the output.
- Leaving `synonyms` empty (hurts search).

## Metric Documentation Standard (Required)

Metric descriptions should follow a consistent, structured pattern. These sections
are already used in the manifest and are indexed for search clarity:

- **Meaning**: What the metric represents in business terms.
- **Formula**: Plain‑text numerator/denominator (or aggregation) in words.
- **Grain**: Time field + default dimensions.
- **Caveats**: Known limitations or tracking gaps.

Example description (used by existing ecommerce metrics):

```
## Meaning
Cart view rate (cart views per session).

## Formula
Numerator is sum of cart_viewed. Denominator is count of distinct new_session_id.

## Grain
period by country_code, device_type, platform_name.

## Caveats
Assumes tracking completeness for cart_viewed.
```

## Metric Docblocks (Required)

Store each metric’s documentation in a **dedicated doc block file** (one metric = one file)
and reference it from the model YAML description. This keeps docs consistent and searchable
without polluting the YAML.

Recommended pattern:

```yaml
models:
  - name: metric__conversion_rate
    description: |
      {{ doc('metric__conversion_rate') }}
```

Doc block file example (path is project‑specific):

```
docs/metrics/ecommerce/metric__conversion_rate.md
```

Follow the **Metric Documentation Standard** sections (Meaning / Formula / Grain / Caveats)
in each doc block.

Here is a full example:

```yaml
version: 2

models:
  - name: metric__conversion_rate
    description: "Conversions per session."
    group: product_analytics

    meta:
      nova:
        canonical: true
        tier: alpha
        domains: ["digital", "product"]
        use_cases: ["weekly_report", "product_analytics"]
        metric:
          name: conversion_rate
          template: true
          description: "Conversions per session."
          expression: "sum(is_converted) / nullif(count(distinct session_id), 0)"
          synonyms: ["conversion rate", "conversion", "cvr"]
          grain:
            time_field: activity_date
            dimensions: ["country_code", "platform_name", "device_type"]
          recommended_filters:
            - field: platform_name
              operator: "in"
              values: ["web", "app"]
              label: "Digital platforms (overall)"
```

```sql
with base as (
  select *
  from {{ ref('base__example_activity') }}
)
select
  activity_date,
  country_code,
  platform_name,
  device_type,
  sum(is_converted) / nullif(count(distinct session_id), 0) as conversion_rate
from base
group by 1, 2, 3, 4

```
