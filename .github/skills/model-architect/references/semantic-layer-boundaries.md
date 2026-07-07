# Semantic Layer Boundaries

## Principle

Nova semantics should live as close as possible to the real execution model that owns the business meaning.

## Good boundary

- canonical measures live on the execution entity
- reusable KPI templates live on the execution entity or a clearly justified semantic model
- helper models expose only the minimal metadata needed for routing or engineering workflows
- specialized marts expose semantics only for their explicit grain and scope
- cross-grain KPIs point to a deterministic dbt relation, configured Semantic
  Layer path, saved query, or recipe before becoming analyst-facing

## Bad boundary

- business meaning lives only in thin wrappers
- search relies on naming conventions instead of stable semantics
- repeated metrics are copied across many sibling models without a canonical source
- reporting datasets become the only place where reusable KPI definitions exist
- metadata-only formulas or derivation notes are treated as queryable KPIs
- Semantic Layer indicators are treated as relation-backed SQL surfaces without
  the configured semantic execution path

## Boundary decisions

When choosing where semantics live, ask:
- where does the data really live?
- which model has the correct analyst-facing grain?
- which model should be the default answer for repeated business questions?
- which other entities should remain searchable but not preferred?
- which specialized marts are intentionally separate?
- which semantic definitions must move, be aliased, or be deprecated?

## Execution surface policy

`search_indicator` and `indicator_inventory` return execution metadata that
must shape architecture decisions:

- `execution_surface: "relation"` with `queryable_via: "relation_name"` means
  the indicator can be queried through the returned relation after grain and
  field checks.
- `execution_surface: "semantic_layer"` with `queryable_via: "metricflow"`
  belongs to the configured dbt Semantic Layer / MetricFlow execution path, not
  ad hoc SQL against a relation.
- `execution_surface: "metadata_only"` or `queryable: false` is context only.
  Move the KPI to a dbt model, semantic metric, saved query, or recipe before
  calling it executable.
