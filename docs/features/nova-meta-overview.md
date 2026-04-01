# Nova Meta Overview

`meta.nova` is a lightweight metadata namespace inside dbt that powers Nova’s
search, scoring, and governance. It is intentionally small and stable: you add
only **human intent** (business meaning, defaults, and compliance), while Nova
derives everything else from the manifest.

Nova reads `meta.nova` at two levels:

- **Entity-level**: models, sources, metrics (the model itself).
- **Column-level**: inside `columns[].meta.nova`.

## Quick Examples

### Model-level (canonical dataset)

```yaml
models:
  - name: base__example_activity
    meta:
      nova:
        canonical: true
        tier: alpha
        domains: ["digital", "product"]
        use_cases: ["weekly_report", "product_analytics"]
        synonyms: ["activity", "sessions", "user activity"]
        grain:
          primary_key: ["activity_id"]
          time_field: activity_date
          dimensions: ["country_code", "platform_name"]
        measures:
          - name: active_users
            type: count_distinct
            expression: "count(distinct user_id)"
            description: "Distinct active users."
            field: user_id
            synonyms: ["dau", "active users"]
        search:
          candidates:
            analyst: false
        governance:
          sensitivity: medium
          pii: possible
          compliance: ["gdpr"]
```

### Column-level (semantic hints)

```yaml
columns:
  - name: country_code
    meta:
      nova:
        role: dimension
        semantic_type: country_code
        synonyms: ["market", "country"]
        example_values: ["GB", "FR", "DE"]
        governance:
          pii: false
```

## Field Map (What Nova Reads)

### Common fields (entity + column)

| Field | Type | Notes |
| --- | --- | --- |
| `role` | enum | `dimension`, `measure`, `metric`, `identifier`, `time` |
| `semantic_type` | string | Organization-specific semantic label |
| `synonyms` | array<string> | Business search terms |
| `example_values` | array<string> | Lightweight discovery hints |
| `governance` | object | `sensitivity`, `pii`, `compliance` |

### Entity fields (models & sources)

| Field | Type | Notes |
| --- | --- | --- |
| `canonical` | boolean | Preferred dataset for a concept |
| `tier` | enum | `alpha`, `beta`, `gamma`, `gold`, `silver`, `bronze` |
| `domains` | array<string> | Business routing |
| `use_cases` | array<string> | Common analyst questions |
| `grain` | object | `primary_key`, `time_field`, `dimensions` |
| `measures` | array<object> | Reusable aggregations |
| `search.candidates` | object | Persona-specific ranking hints (`analyst`, `engineer`, `governance`) |

`measures[]` fields:
- `name` (required), `type` (required), `expression`, `description`, `field`, `synonyms`

### Metric fields (metric models)

Use **either** `metric` (single KPI) or `metrics` (array).

`metric`/`metrics[]` fields:
- `name` (required), `description`, `expression`, `synonyms`, `template`
- `grain`: `time_field`, `dimensions`, and optional `primary_key`
- `recommended_filters`: `field`, `operator`, `values`, `label`

Allowed `recommended_filters.operator` values:
`in`, `not_in`, `=`, `!=`, `>`, `>=`, `<`, `<=`, `between`, `is_null`, `is_not_null`.

## Governance Fields (Entity + Column)

| Field | Type | Notes |
| --- | --- | --- |
| `sensitivity` | enum | `none`, `public`, `internal`, `confidential`, `low`, `medium`, `high`, `restricted` |
| `pii` | bool / string / array<string> | Recommended string levels: `none`, `possible`, `confirmed` |
| `compliance` | array<string> | e.g., `gdpr`, `soc2`, `hipaa` |

If you use `pii` as a list, prefer explicit classes (`["email", "phone"]`).

## How Nova Uses Meta

- **Search ranking**: boosts matches on `synonyms`, `domains`, `use_cases`,
  `measures`, and `metric(s)` fields, and can de-boost persona-specific helper models via
  `search.candidates`.
- **Metadata scoring**: evaluates coverage and quality of key Nova fields.
- **Governance filters**: enables queries like “show restricted” or “pii”.
- **Agent workflows**: persona skills rely on Nova meta for reliable outputs.

`search.candidates` is a ranking hint:
- Missing keys default to `true`
- `false` lowers rank for that persona
- entities remain searchable

## Governance Mode: Build and Validate Nova Meta

Governance workflows don’t just **audit** metadata—they help teams **improve**
it with clear, actionable feedback:

- **Score coverage and quality** with `get_metadata_score` (entity or project).
- **Find gaps** with `get_undocumented` (missing descriptions/columns).
- **Validate completeness** against A‑grade criteria (governance + semantic).
- **Generate remediation plans** by listing missing fields and owners.

In practice, governance agents use these outputs to **close the loop**:
score → identify gaps → fix meta → re‑score.

## Validation

The schema is versioned in `schemas/nova/v0.json`. Use it to validate
`meta.nova` blocks in CI or during review.

## Best Practices

- Keep meta **small and stable** (avoid high‑churn data).
- Use **canonical** for exactly one model per concept.
- Prefer 2–8 high‑signal synonyms over exhaustive lists.
- Use `grain.dimensions` only for **default** breakdowns.

## Related Guides

- [Nova Meta (Models)](nova-meta-models.md)
- [Nova Meta (Metrics)](nova-meta-metrics.md)
- [Search Ranking](search-ranking.md)
