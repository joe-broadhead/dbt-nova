# Nova Meta: Model Guide (for Agents)

This guide explains how to add and maintain `meta.nova` on dbt models so Nova search + tooling stay high‑signal and low‑maintenance.
For the complete field map and governance conventions, see
[Nova Meta Overview](nova-meta-overview.md).

## Goals

- Make canonical datasets easy to find.
- Encode only non‑derivable intent (avoid metadata bloat).
- Keep changes small and repeatable.

## Where to Put Nova Meta

Add a `meta.nova` block in the model’s YAML file:

```yaml
version: 2
models:
  - name: base__example_activity
    meta:
      nova:
        canonical: true
        tier: alpha
        domains: ["digital", "product"]
        use_cases: ["weekly_report", "product_analytics"]
        synonyms: ["activity", "user activity", "sessions"]
        grain:
          primary_key: ["activity_id"]
          time_field: activity_date
          dimensions: ["country_code", "platform_name"]
        measures:
          - name: active_users
            canonical: true
            expression: "count(distinct user_id)"
            description: "Distinct active users."
            type: count_distinct
            field: user_id
            synonyms: ["dau", "active users"]
        governance:
          sensitivity: medium
          pii: possible
          compliance: ["gdpr"]
```

## Recommended Fields (Model‑Level)

- `canonical` (bool): True for the preferred dataset for a business concept.
- `tier` (`alpha`, `beta`, `gamma`, `gold`, `silver`, or `bronze`): Quality signal for discovery.
- `domains` (list): Broad business domain(s) used for routing.
- `use_cases` (list): Typical analyst questions (e.g., `weekly_report`).
- `synonyms` (list): Business names analysts will search for.
- `grain.primary_key` (list): The row‑level identifier(s).
- `grain.time_field` (string): Primary time dimension.
- `grain.dimensions` (list, optional): Default breakdowns for analysis.
- `measures` (list): Minimal, reusable measures from this model.
- `governance` (object, optional): Compliance signal for governance discovery.

## Current Ecommerce Nova Conventions (Implemented)

This section reflects what is **already in the manifest** for ecommerce models today.
Use it as a reference when extending coverage; update it when you expand Nova meta
to additional domains.

- `canonical`: all current ecommerce models are `true`.
- `tier`: `alpha` and `beta` are used.
- `domains`: `ecommerce`, `web`, `stock`, `app`.
- `use_cases` (current set): `app_performance`, `app_sales`, `buy_to_detail_funnel`,
  `campaign_analysis`, `cart_and_reserve`, `category_navigation`, `category_reporting`,
  `inventory_health`, `product_performance`, `promo_feature_engagement`,
  `revenue_opportunity`, `stock_availability`, `stockout_analysis`, `weekly_report`,
  `web_analytics`.
- `grain`: both `primary_key` and `time_field` are populated.
- `measures`: each measure uses `name`, `expression`, `description`, `type`, `field`,
  `synonyms`, and can optionally set `canonical: true` when the same business
  term exists on multiple models. Use `canonical_scope: grain` only when a
  measure is intentionally canonical for that model grain instead of globally
  canonical for the measure name.
- `governance`: `sensitivity` is `low` or `medium`; `pii` is `none` or `possible`;
  `compliance` includes `gdpr`.

## Reference Schema (Copy/Paste)

```yaml
meta:
  nova:
    canonical: true
    tier: alpha
    domains: ["digital", "product"]
    use_cases: ["weekly_report", "product_analytics"]
    synonyms: ["activity", "user activity", "sessions"]
    grain:
      primary_key: ["activity_id"]
      time_field: activity_date
      dimensions: ["country_code", "platform_name"]
    measures:
      - name: active_users
        canonical: true
        description: "Distinct active users."
        expression: "count(distinct user_id)"
        type: count_distinct
        field: user_id
        synonyms: ["dau", "active users"]
    governance:
      sensitivity: medium
      pii: possible
      compliance: ["gdpr"]
```

## Synonyms Conventions (Model‑Level)

Use consistent formatting so search behavior is predictable.

- Prefer **lowercase phrases** for business terms (e.g., `"web sessions"`, `"site sessions"`).
- Include **snake_case** only when it is a common technical alias (e.g., `"session_id"`).
- Avoid punctuation and overly broad words (e.g., `"data"`, `"table"`).
- Keep 2–8 high‑signal entries; do not duplicate close variants.

## Measures (Model‑Bound)

Measures live on the model where the data exists.

Use this minimal shape:

```yaml
measures:
  - name: sessions
    canonical: true
    expression: "count(distinct new_session_id)"
    description: "Total sessions."
    type: count_distinct
    field: new_session_id
    synonyms: ["visits"]
```

### What Is a Measure?

A **measure** is a reusable aggregation defined at the model level (e.g., `count(distinct new_session_id)`).
Measures are **model‑bound**: they belong to the model where the underlying data lives.

Use `canonical: true` on a measure when the same business term exists on multiple
models but one definition should surface first in analyst search.

If the same measure name is intentionally canonical at multiple grains, add
`canonical_scope: grain` on those measure entries. This keeps project validation
from treating the definitions as competing global canonicals while preserving the
metadata-bridge signal that each parent owns the measure at its declared grain.

### How to Detect Measures in a Model

Use these signals:

- **Aggregation logic** in SQL (`count`, `sum`, `avg`, `min`, `max`) tied to a column.
- **Reusable KPI** you expect analysts to compute repeatedly (sessions, orders, revenue).
- **Stable definition** that shouldn’t change per report.

If a value is row‑level (dimension) or a one‑off calculation for a single report, it is **not** a measure.

## Metric Templates (Optional)

If the model is a **metric template**, define a `metric` (single) or `metrics` (multiple)
block under `meta.nova`. See the Metric Guide for full structure and conventions.

## Governance Meta (Recommended)

Governance metadata powers compliance search (e.g., queries like “pii”, “gdpr”, “restricted”).
Keep it **minimal** and **deterministic**.

```yaml
governance:
  sensitivity: high
  pii: confirmed
  compliance: ["gdpr", "ccpa"]
```

### `governance.sensitivity` (enum)
- `none`
- `public`
- `internal`
- `confidential`
- `low`
- `medium`
- `high`
- `restricted`

### `governance.pii` (recommended values)
- `none`
- `possible`
- `confirmed`

Also accepted:
- **boolean** (coarse classification)
- **array of tags** (e.g., `["email", "phone"]`)

### `governance.compliance` (recommended values)
Use one or more of:
- `gdpr`
- `ccpa`
- `hipaa`
- `pci`
- `sox`
- `soc2`
- `internal_only`

## Deterministic Enums (Use These Exact Values)

These fields should use fixed enums for consistency:

### `tier` (recommended values)
- `alpha`
- `beta`
- `gamma`
- `gold`
- `silver`
- `bronze`

If your organization uses a different tiering scheme, keep it consistent and
document it in a single shared place.

### `measures[].type`
- `count`
- `count_distinct`
- `sum`
- `avg`
- `min`
- `max`
- `ratio` (use only if the measure itself is a ratio)

### `governance.sensitivity`
- `none`
- `public`
- `internal`
- `confidential`
- `low`
- `medium`
- `high`
- `restricted`

### `governance.pii` (recommended string values)
- `none`
- `possible`
- `confirmed`

Also accepted: boolean or array of tags (e.g., `["email", "phone"]`).

### `governance.compliance` (recommended values)
- `gdpr`
- `ccpa`
- `hipaa`
- `pci`
- `sox`
- `soc2`
- `internal_only`

### `columns[].meta.nova.role` (if used)
- `dimension`
- `measure`
- `metric`
- `identifier`
- `time`

### `columns[].meta.nova.semantic_type` (recommended enums)
Prefer a small, consistent set:
- `country_code`
- `country_name`
- `region`
- `platform`
- `device`
- `channel`
- `date`
- `timestamp`
- `session_id`
- `order_id`
- `user_id`
- `event_name`
- `revenue`
- `quantity`
- `boolean_flag`
- `marketing_campaign`
- `marketing_source`
- `marketing_medium`
- `marketing_term`

### `domains` / `use_cases` (controlled vocabulary)

These should be treated as enums within **your organization**. Maintain a short, curated list
in one place and reuse it consistently (avoid ad‑hoc values).

### `columns[].meta.nova.role` (if used)
- `dimension`
- `measure`
- `metric`
- `identifier`
- `time`

### Best Practices

- Define 1–3 **high‑value** measures per canonical model.
- Keep expressions simple (aggregations only).
- Avoid duplicating measures across multiple models unless only one is canonical.

## Column‑Level Nova (Optional)

Only add column meta when it improves search or interpretation.

```yaml
columns:
  - name: country_code
    meta:
      nova:
        role: dimension
        semantic_type: country_code
        synonyms: ["market", "country"]
```

Use for:
- ambiguous names (e.g., `id`, `status`, `type`)
- key filters or breakdowns
- metrics foundation columns

### Optional: `example_values` (Low‑Maintenance Hints)

For key filter columns (country, platform, device), you may add a short list of
example values to speed analyst discovery. This is **not authoritative** and
should not replace analyst validation queries.

```yaml
columns:
  - name: country_code
    meta:
      nova:
        role: dimension
        semantic_type: country_code
        synonyms: ["market", "country"]
        example_values: ["GB", "FR", "DE", "ES"]
```

## What NOT to Encode

Avoid high‑churn or easily queryable metadata:

- freshness cadence
- row counts
- lineage edges
- test results
- performance details

## Quality Checklist

- Canonical model has `tier`, `use_cases`, `synonyms`, and `grain`.
- Measures are defined only on canonical models.
- Synonyms align with business terminology.
- No duplicated measure names across canonical models.

## Common Pitfalls

- Adding too many measures (noise).
- Using synonyms that are too broad (pollutes search).
- Marking multiple models as canonical for the same concept.

Here is a full example for a canonical activity model:

```yaml
version: 2

models:
  - name: base__example_activity
    description: |
      {{ doc("technical__base__example_activity") }}
      ---
      {{ doc("semantic__base__example_activity") }}
    group: product_analytics

    meta:
      nova:
        canonical: true
        tier: alpha
        domains: ["digital", "product"]
        use_cases: ["weekly_report", "product_analytics"]
        synonyms: ["activity", "user activity", "sessions"]
        grain:
          primary_key: ["activity_id"]
          time_field: activity_date
        measures:
          - name: active_users
            expression: "count(distinct user_id)"
            description: "Distinct active users."
            type: count_distinct
            field: user_id
            synonyms: ["dau", "active users"]
          - name: sessions
            expression: "count(distinct session_id)"
            description: "Distinct sessions."
            type: count_distinct
            field: session_id
            synonyms: ["visits", "sessions"]
          - name: conversions
            expression: "sum(is_converted)"
            description: "Conversions (boolean flag sum)."
            type: sum
            field: is_converted
            synonyms: ["conversions", "converted sessions"]
          - name: revenue
            expression: "sum(revenue_amount)"
            description: "Total revenue across activity."
            type: sum
            field: revenue_amount
            synonyms: ["sales", "gmv"]

    columns:
      - name: activity_id
        data_type: string
        description: "Primary key for each activity row."
        meta:
          nova:
            role: identifier
            semantic_type: session_id
            synonyms: ["activity_pk"]
        data_tests:
          - unique
          - not_null

      - name: user_id
        data_type: string
        description: "User identifier."
        meta:
          nova:
            role: identifier
            semantic_type: user_id
            synonyms: ["customer_id", "account_id"]

      - name: session_id
        data_type: string
        description: "Session identifier."
        meta:
          nova:
            role: identifier
            semantic_type: session_id
            synonyms: ["session_id"]

      - name: activity_date
        data_type: date
        description: "Date of the activity."
        meta:
          nova:
            role: time
            semantic_type: date
            synonyms: ["activity_day"]

      - name: activity_ts
        data_type: timestamp
        description: "Timestamp of the activity event."
        meta:
          nova:
            role: time
            semantic_type: timestamp
            synonyms: ["event_ts"]

      - name: country_code
        data_type: string
        description: "Country code for the activity."
        meta:
          nova:
            role: dimension
            semantic_type: country_code
            synonyms: ["market", "country"]
            example_values: ["GB", "FR", "DE", "ES"]

      - name: platform_name
        data_type: string
        description: "Platform for the activity (e.g., web, app)."
        meta:
          nova:
            role: dimension
            semantic_type: platform
            synonyms: ["platform", "channel_platform"]
            example_values: ["web", "app"]

      - name: device_type
        data_type: string
        description: "Device category for the activity."
        meta:
          nova:
            role: dimension
            semantic_type: device
            synonyms: ["device", "device_class"]

      - name: event_name
        data_type: string
        description: "Business event name."
        meta:
          nova:
            role: dimension
            semantic_type: event_name
            synonyms: ["event", "action"]

      - name: is_converted
        data_type: int
        description: "Conversion flag (0/1)."
        meta:
          nova:
            role: measure
            semantic_type: boolean_flag
            synonyms: ["converted", "conversion_flag"]

      - name: revenue_amount
        data_type: double
        description: "Revenue amount attributed to the activity."
        meta:
          nova:
            role: measure
            semantic_type: revenue
            synonyms: ["sales", "gmv"]
```
