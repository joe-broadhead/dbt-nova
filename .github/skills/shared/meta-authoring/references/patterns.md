# Metadata Patterns

## Canonical analyst-facing model

Use this when the model is the preferred analyst-facing source for a business concept.

```yaml
version: 2
models:
  - name: fct_orders
    meta:
      nova:
        canonical: true
        tier: alpha
        domains: ["ecommerce", "revenue"]
        use_cases: ["weekly_report", "revenue_analysis"]
        synonyms: ["orders", "sales", "gmv"]
        grain:
          primary_key: ["order_id"]
          time_field: order_date
          dimensions: ["country_code", "channel", "device_type"]
        measures:
          - name: gmv
            canonical: true
            type: sum
            expression: "sum(gmv_amount)"
            description: "Gross merchandise value before returns."
            field: gmv_amount
            synonyms: ["gross merchandise value", "revenue"]
        governance:
          sensitivity: low
          pii: none
          compliance: ["gdpr"]
```

Use this when:
- the model is the preferred dataset for the concept
- analysts should discover it first
- the measure should surface directly in search with its expression

## Helper / ops / intermediate model

Use this when the model is important for engineering workflows but should not dominate analyst search.

```yaml
version: 2
models:
  - name: int_orders_enriched
    meta:
      nova:
        synonyms: ["orders enrichment"]
        search:
          candidates:
            analyst: false
        grain:
          primary_key: ["order_id"]
          time_field: order_date
```

Use this when:
- the model is operational, helper, staging, or intermediate
- engineers still need to find it
- analysts should land on a richer downstream model instead

## Metric template model

Use this when the dbt model represents a reusable KPI template that analysts adapt by time window, filters, and breakdowns.

`mart__...` is only an example naming convention. Nova does not have any special parser, ranking, or routing logic for model names that start with `mart__`. What matters is that the resource is a dbt `model` with `meta.nova.metric` or `meta.nova.metrics`.

```yaml
version: 2
models:
  - name: mart__average_order_value
    meta:
      nova:
        canonical: true
        tier: alpha
        domains: ["ecommerce", "revenue"]
        use_cases: ["revenue_analysis", "weekly_report"]
        metric:
          name: average_order_value
          canonical: true
          template: true
          description: "Gross merchandise value divided by order count."
          expression: "sum(gmv_amount) / nullif(count(distinct order_id), 0)"
          synonyms: ["aov", "average basket"]
          grain:
            time_field: order_date
            dimensions: ["country_code", "channel"]
          recommended_filters:
            - field: channel
              operator: in
              values: ["web", "app"]
              label: Digital channels
```

Use this when:
- the KPI is reused across many questions
- analysts should discover the formula directly in Nova search
- the SQL model cleanly owns the KPI definition

## Canonical source pattern

Use this when the upstream table is a raw or lightly curated source but still needs Nova routing hints.

```yaml
version: 2
sources:
  - name: commerce
    tables:
      - name: orders
        meta:
          nova:
            domains: ["ecommerce"]
            synonyms: ["raw orders"]
            grain:
              primary_key: ["order_id"]
              time_field: order_created_at
```

Keep source metadata sparse. Use it mainly for discovery and routing, not for rich analyst semantics.

## Column-level semantic hints

Use column-level `meta.nova` selectively for identifiers, time fields, or dimensions that need semantic disambiguation.

```yaml
columns:
  - name: country_code
    meta:
      nova:
        role: dimension
        semantic_type: country_code
        synonyms: ["country", "market"]
        example_values: ["FR", "DE", "GB"]

  - name: order_date
    meta:
      nova:
        role: time
        semantic_type: date
```

Good candidates:
- identifiers
- time fields
- high-signal business dimensions
- columns where synonyms materially improve discovery

## Canonicality rules

### Entity-level canonical
Mark `meta.nova.canonical: true` when the model or source is the preferred analyst-facing dataset for the concept.

### Measure-level canonical
Mark `measures[].canonical: true` when the same business term appears across multiple models but one definition should rank first for analyst search.

### Metric-level canonical
Mark `metric.canonical: true` or `metrics[].canonical: true` when the same KPI exists across multiple metric templates and one should be preferred.

Use canonical flags to break ties on repeated business terms such as `gmv`, `sessions`, or `average_order_value`.
