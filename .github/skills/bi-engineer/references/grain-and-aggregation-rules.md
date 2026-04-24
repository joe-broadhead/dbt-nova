# Grain and Aggregation Rules

## Grain comes first

Before designing a visualization, confirm:
- the execution entity grain
- the time field
- the supported dimensions
- whether the selected indicator is additive, semi-additive, or non-additive
- whether the view is detail, aggregate, or drill-level

## Practical rules

- Do not sum precomputed rates unless the definition explicitly supports rollup.
- Use numerator and denominator logic from the canonical indicator definition for rate charts.
- Avoid mixing daily and weekly grains in the same view unless the relationship is explicit.
- Prefer one base grain per dashboard section.
- When a view requires a finer grain than the canonical dashboard section, call that out as a drill or secondary view.
- If two entities are needed, compare their grains before designing a shared section.
- Declare null, zero, and missing-period handling for every chart that compares periods.
