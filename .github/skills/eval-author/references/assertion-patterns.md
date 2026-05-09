# Assertion Patterns

## Bridge Eval Patterns

Use `search_rank` for canonical entity retrieval:

```yaml
- type: search_rank
  query: orders
  expected_unique_id: model.pkg.orders
  max_rank: 5
```

Use `search_indicator_rank` for canonical KPI discovery:

```yaml
- type: search_indicator_rank
  query: gross merchandise value
  expected: gross merchandise value
  max_rank: 3
  resource_types: [model]
```

Use `search_columns_rank` when the workflow depends on a specific filter, time, or measure field:

```yaml
- type: search_columns_rank
  query: order date
  expected_column: order_date
  expected_parent_unique_id: model.pkg.orders
  max_rank: 5
```

Use context assertions for required execution evidence:

```yaml
- type: context_has
  id_or_name: model.pkg.orders
  fields:
    - data.unique_id
    - data.entity.name
- type: context_contains
  id_or_name: model.pkg.orders
  field: data.entity.description
  expected: canonical orders
```

Use recipe assertions for recurring workflow discovery:

```yaml
- type: recipe_rank
  query: weekly country kpi report
  expected_recipe_id: marketing/weekly_country_kpi_report
  max_rank: 5
- type: recipe_has_queries
  recipe_id: marketing/weekly_country_kpi_report
  min_queries: 1
```

`search_recipes` returns recipe rows with `id`. In suites, use
`expected_recipe_id` and set it to the exact returned `id`; do not use a display
title, path fragment, or shortened basename unless that is the full returned id.

Use `tool_success` for endpoint coverage without overfitting to response shape:

```yaml
- type: tool_success
  tool: get_test_coverage
  params:
    id_or_name: model.pkg.orders
    include_full: false
```

## Anti-Patterns

- Do not assert rank 1 unless rank 1 is a durable product requirement.
- Do not assert on long descriptions that are likely to be edited.
- Do not encode raw warehouse relation names when `unique_id` is available.
- Do not mix many concepts into one case. Split cases so one failure has one likely cause.
- Do not use unscoped `search_columns_rank` for common fields. Provide
  `expected_parent_unique_id` and use a tolerant `max_rank`, or use
  `context_has` / `context_contains` once the execution entity is known.
- Do not use `execute_sql` in bridge smoke suites unless the SQL provider and data fixture are controlled.
