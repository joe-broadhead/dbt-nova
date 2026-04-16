# Grain and Aggregation Rules

## Grain comes first

Before designing a visualization, confirm:
- the execution entity grain
- the time field
- the supported dimensions
- whether the selected indicator is additive, semi-additive, or non-additive

## Practical rules

- Do not sum precomputed rates unless the definition explicitly supports rollup.
- Use numerator and denominator logic from the canonical indicator definition for rate charts.
- Avoid mixing daily and weekly grains in the same view unless the relationship is explicit.
- Prefer one base grain per dashboard section.
- When a view requires a finer grain than the canonical dashboard section, call that out as a drill or secondary view.

## Breakdown rule

A breakdown is valid only if:
- the dimension exists on the execution entity
- the indicator definition is meaningful at that breakdown
- the resulting cardinality is still usable in the intended chart
