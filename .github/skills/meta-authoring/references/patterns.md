# Metadata Patterns

Use these as compact shapes, not copy/paste defaults. Preserve repo conventions and only add fields that are true for the target resource.

## Canonical Analyst Dataset

```yaml
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
          dimensions: ["country_code", "channel"]
        measures:
          - name: gmv
            canonical: true
            type: sum
            expression: "sum(gmv_amount)"
            field: gmv_amount
            description: "Gross merchandise value before returns."
            synonyms: ["gross merchandise value", "revenue"]
        governance:
          sensitivity: low
          pii: none
          compliance: ["gdpr"]
```

Use when analysts should land on this model first for the domain or metric family.

## Helper Or Intermediate Resource

```yaml
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

Use when the resource should remain findable but should not outrank the canonical analyst model.

## Metric Template

```yaml
models:
  - name: mart__average_order_value
    meta:
      nova:
        canonical: true
        domains: ["ecommerce", "revenue"]
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
```

Use when the KPI is a reusable definition, not a one-off answer. Model names like `mart__` are conventions only; Nova behavior comes from metadata.

## Source Or Staging Governance

```yaml
models:
  - name: stg__orders
    meta:
      nova:
        tier: bronze
        domains: ["ecommerce"]
        use_cases: ["source_traceability"]
        synonyms: ["raw orders", "order events"]
        search:
          candidates:
            analyst: false
        grain:
          primary_key: ["id"]
          time_field: creation_date_time
        governance:
          sensitivity: restricted
          pii: confirmed
          compliance: ["gdpr"]
```

Use sparse routing and governance metadata for sources/staging. Do not add rich analyst measures unless the source is intentionally the execution surface.

## Column Hints

```yaml
columns:
  - name: country_code
    meta:
      nova:
        role: dimension
        semantic_type: country_code
        synonyms: ["country", "market"]
        example_values: ["FR", "DE", "GB"]

  - name: customer_id
    meta:
      nova:
        role: identifier
        semantic_type: user_id
        synonyms: ["customer", "member"]
        governance:
          pii: confirmed
```

Use column hints only when they improve search, disambiguation, or governance behavior.
