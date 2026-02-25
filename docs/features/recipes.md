# Analysis Recipes

Analysis recipes provide a deterministic way to run recurring reporting workflows
as ordered SQL runs.

They are designed for repeated, high-value analyses where analysts want:

- stable execution order
- reusable KPI bundles
- predictable parameters
- separation between discovery (`search`) and execution (`run_recipe`)

In dbt-nova, a recipe is a directory-style identifier (for example
`marketing/weekly_report`) containing one or more SQL steps, discovered from
manifest `analysis` nodes.

## Where to store recipes

By default, recipe discovery uses this manifest `original_file_path` prefix:

- `analyses/recipes`

Each folder path in `original_file_path` forms a
recipe ID (for example `analyses/recipes/marketing/weekly_report` becomes
`marketing/weekly_report`).

Example:

```text
analyses/recipes/
  weekly_country_kpi_report/
    analysis__weekly_country_kpi_report__weekly_headline_kpis__01.sql
    analysis__weekly_country_kpi_report__weekly_funnel_health__09.sql
  marketing/
    retention/
      analysis__marketing_retention__cohort_summary__01.sql
```

Recipe IDs:

- `weekly_country_kpi_report`
- `marketing/retention`

You can fetch by full path (`marketing/retention`) or basename (`retention`).

## SQL Source

Recipes are manifest-only. For each discovered recipe query, Nova loads SQL from
the corresponding dbt `analysis` node in the manifest:

- prefer non-empty `compiled_code`
- fallback to non-empty `raw_code` when compiled SQL is unavailable

Execution and SQL rendering guardrail:

- if fallback `raw_code` contains dbt/Jinja markers (`{{`, `{%`, or `{#`), Nova rejects
  `run_recipe` and `get_recipe(include_sql: true)` with `INVALID_PARAMS`
  details so failures are actionable before warehouse SQL parsing.

## Runtime parameter replacement

After a query is resolved from manifest analysis, Nova performs
placeholder substitution for tokens in the form:

- `__TOKEN__`

You can pass values with `run_recipe` through `parameters`:

```json
{
  "name":"run_recipe",
  "arguments":{
    "recipe_id":"weekly_country_kpi_report",
    "parameters":{"COUNTRY_CODE":"US","START_DATE":"2026-02-01"},
    "placeholder_types":{"COUNTRY_CODE":"string","START_DATE":"string"},
    "sql_parameter_types":{"bind_start_date":"DATE"}
  }
}
```

Placeholder coercion hints in `placeholder_types` support `identifier`, `number`,
`boolean`, `raw`, and default string coercion rules.

SQL bind hints in `sql_parameter_types` are passed to warehouse execution for
named SQL bind parameters (for example `:bind_start_date`).

Legacy `parameter_types` remains accepted as a fallback for both maps.

If a resolved query contains placeholders and `parameters` are not supplied, `run_recipe`
and `get_recipe(include_sql: true)` return a parameter error.

## Parameter Contract Surface

Recipes now return parameter contracts directly in tool responses:

- `required_parameters`
- `optional_parameters`
- `parameter_defaults`
- `query_parameters` (per-query parameter specs)

When parameters are partially supplied, responses include:

- `missing_parameters`
- `unused_parameters`
- `type_mismatches`

`run_recipe` validates all selected queries before execution and returns
structured preflight validation details (including `by_query`) on failure.

## Weekly Country KPI Report Example

`analyses/recipes/weekly_country_kpi_report/*.sql`:

```text
analysis__weekly_country_kpi_report__weekly_headline_kpis__01.sql
analysis__weekly_country_kpi_report__monthly_headline_kpis__02.sql
analysis__weekly_country_kpi_report__ytd_headline_kpis__03.sql
analysis__weekly_country_kpi_report__period_variance_kpis__04.sql
analysis__weekly_country_kpi_report__weekly_channel_mix__05.sql
analysis__weekly_country_kpi_report__weekly_platform_mix__06.sql
analysis__weekly_country_kpi_report__weekly_campaign_mix__07.sql
analysis__weekly_country_kpi_report__weekly_promo_mix__08.sql
analysis__weekly_country_kpi_report__weekly_funnel_health__09.sql
analysis__weekly_country_kpi_report__weekly_paid_media_efficiency__10.sql
analysis__weekly_country_kpi_report__weekly_customer_segment_mix__11.sql
analysis__weekly_country_kpi_report__weekly_product_revenue_mix__12.sql
```

Recommended convention:

- `analysis__<recipe_id>__<step_name>__<step_number>.sql`
- Keep `step_number` zero-padded (`01`, `02`, ...).

This keeps execution order explicit and query intent readable.

This repository contains manifest-linked recipe examples in test fixtures.

These analyses should return the KPI set you need for the report, for example:

- `sessions`, `users`, `page_views`, `conversions`, `revenue`
- `conversion_rate`, `avg_order_value`, `revenue_per_1k_sessions`
- period metadata (`period`, `period_start`, `period_end`)

Because the SQL is compiled by dbt, you can use dbt `vars`, `source`, and `ref`
logic in the analysis definition itself.

## Tool Workflow

### 1) Discover recipes

Use `search_recipes` with topic/query filters.

### 2) Inspect a recipe

Use `get_recipe` to see query order and optional SQL.

### 3) Execute

Use `run_recipe` with optional `query_names` or `query_indexes`.

`query_indexes` are 1-based using resolved execution order.

## Execution Order

Query ordering is stable:

1. Last `__<n>` suffix (recommended convention)
2. Last `_<n>` suffix
3. Any remaining names, sorted lexicographically

Examples:

- `analysis__weekly_country_kpi_report__weekly_headline_kpis__01.sql` → 1
- `analysis_weekly_headline_2.sql` → 2
- `analysis__weekly_headline_kpis.sql` → last (no explicit order)

If multiple files share the same order, Nova sorts by filename.

## API Notes

- `get_recipe` includes per-query metadata in the payload:
  - `source` (`manifest_analysis`)
  - `analysis_id`
- `parameters` and `placeholder_types` are consumed during placeholder SQL resolution.
- `sql_parameter_types` is consumed during warehouse execution.

## Configuration

- `DBT_NOVA_RECIPES_DIR`
- Value is used as the `original_file_path` prefix matcher for recipe analyses.

This feature is documented together with all tool docs in:

- [Tools Reference](../api/tools.md)
- [Response Format](../api/response-format.md)
