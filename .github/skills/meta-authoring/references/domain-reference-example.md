# Starter Commerce Domain Reference Example

This synthetic example uses the `starter_eval` fixture naming from dbt-nova. It
is intentionally small and should not be treated as a production domain model.

## Status

- Owner: analytics enablement
- Maintainers: dbt-nova example maintainers
- Last reviewed: 2026-06-23
- Applies to dbt project: `starter_eval`
- Applies to Nova domains: `commerce`
- Related eval suites: `evals/starter.yml`

## Business Context

The starter commerce domain covers synthetic order reporting for examples,
skills, and evals. Analysts use it to identify the canonical orders model,
resolve gross revenue, inspect customer geography, and run the weekly revenue
recipe. The domain is deliberately narrow: it proves Nova discovery, context,
lineage, recipes, metadata scoring, and agent tool-use behavior without relying
on a real warehouse or real business data.

Nova metadata links:
- `meta.nova.domains`: `commerce`
- `meta.nova.use_cases`: `gross_revenue_reporting`, `order_reporting`
- `meta.nova.synonyms`: order and revenue vocabulary on `fct_orders`
- `meta.nova.governance`: low-sensitivity synthetic reporting data

## Canonical Entities

| Purpose | dbt unique_id | Relation | Why canonical | Nova metadata |
| --- | --- | --- | --- | --- |
| Primary execution dataset | `model.starter_eval.fct_orders` | `analytics.starter.fct_orders` | Owns order-grain reporting and the canonical gross revenue measure. | `canonical`, `grain`, `measures` |
| Customer geography dimension | `model.starter_eval.dim_customers` | `analytics.starter.dim_customers` | Supplies customer attributes and geography used by the order fact. | `domains`, `grain` |
| Raw sparse example | `model.starter_eval.raw_events_sparse` | `analytics.starter.raw_events_sparse` | Demonstrates low-quality sparse metadata and should not be the analyst landing page. | sparse metadata only |

## Grain And Scope

- Primary grain: one row per order
- Primary key: `order_id`
- Time field: `order_date`
- Default dimensions: `country_code`, `status`
- Supported breakdowns: order date, country code, status
- Unsupported breakdowns: session, product, shipment, and marketing attribution

Scope rules:
- Included: synthetic order revenue from the starter fixture.
- Excluded: real customer behavior, stock, fulfilment, margin, returns, and
  marketing channel attribution.
- Known partial coverage: `dim_customers` can support geography examples but is
  not a full customer analytics domain.

Nova metadata links:
- `meta.nova.grain.primary_key`: `order_id`
- `meta.nova.grain.time_field`: `order_date`
- `meta.nova.grain.dimensions`: `country_code`, `status`

## Standard Hygiene Filters

| Filter purpose | Field | Required values or rule | Applies by default? | Notes |
| --- | --- | --- | --- | --- |
| Completed order reporting | `status` | validate accepted fixture values before filtering | no | Use only when the question asks for completed orders. |
| Geography reporting | `country_code` | exact country code such as `GB` | no | Validate friendly country labels before final SQL. |

## Measures And Metrics

| Business term | Nova indicator | Type | Expression or definition | Parent entity |
| --- | --- | --- | --- | --- |
| Gross revenue | `gross_revenue` | measure | Sum of `gross_amount` at order grain. | `model.starter_eval.fct_orders` |

Nova metadata links:
- `meta.nova.measures[]`: `gross_revenue`
- `meta.nova.metric` or `meta.nova.metrics[]`: not used in this fixture
- `recommended_filters`: not used in this fixture

## Dimensions And Filter Values

| Business concept | Field | Semantic type | Common values | Validation requirement |
| --- | --- | --- | --- | --- |
| Order date | `order_date` | time | fixture dates | Confirm requested date window exists when executing SQL. |
| Country | `country_code` | country code | examples include `GB` | Validate friendly labels to exact codes before final SQL. |
| Order status | `status` | status | fixture-defined accepted values | Use only after confirming the target status value. |

## Common Workflows

| Workflow | Recommended path | Required checks | Output shape |
| --- | --- | --- | --- |
| Gross revenue lookup | `search_indicator` for `gross revenue`, then compact context on `model.starter_eval.fct_orders` | Confirm `gross_revenue` and order grain before analysis. | selected model and measure |
| Weekly revenue report | recipe `commerce/weekly_revenue` | Confirm recipe parameters and date range. | daily gross revenue and country rollup |

## Cross-Domain Handoffs

| When the question asks for... | Stay in this domain when... | Hand off to... | Handoff evidence |
| --- | --- | --- | --- |
| Customer geography | The question only needs order revenue by `country_code`. | customer domain reference, if available | `model.starter_eval.dim_customers` lineage |
| Sessions or conversion | The question asks about visits, checkout, or conversion rate. | ecommerce sessions domain, if available | no session metric exists in the starter fixture |
| Raw event provenance | The question is about sparse metadata behavior. | governance or metadata-audit workflow | `model.starter_eval.raw_events_sparse` metadata score |

## Gotchas And Anti-Patterns

- Do not use `model.starter_eval.raw_events_sparse` for revenue answers.
- Do not invent conversion or session KPIs in this fixture; they are outside
  starter commerce scope.
- Do not treat `dim_customers` as a full customer analytics mart; it is a
  supporting dimension for examples.
- Do not bypass `gross_revenue` with raw SQL against `gross_amount` unless
  semantic discovery failed and the fallback is documented.

## Freshness, Tests, And Trust

- Freshness signal: fixture manifest timestamp only.
- Key tests: not-null and uniqueness tests on `fct_orders` identifiers and
  selected columns.
- Metadata score target: high enough to serve starter evals.
- Known caveats: synthetic data, narrow entity set, no live warehouse freshness.
- Required caveats in final answers: mention fixture scope when a user asks for
  production interpretation.

## Maintenance

- Review cadence: whenever starter eval fixtures or packaged skills change.
- Update triggers:
  - `model.starter_eval.fct_orders` grain changes
  - `gross_revenue` definition changes
  - `commerce/weekly_revenue` recipe changes
  - `evals/starter.yml` changes expected tool order or evidence
- Required companion updates:
  - fixture manifest or source YAML
  - analyst or meta-authoring skill references
  - starter eval suite

## Evidence Log

- 2026-06-23: Added as the synthetic example for JOE-27 domain reference docs.
