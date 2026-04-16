# Grain Decision Tree

## Start with the analytical question

Ask:
- what is the unit of analysis?
- what breakdowns must remain valid?
- what comparisons must remain valid?

## Decision path

1. If the model must support row-level drill or entity-level joins, prefer primary-key grain.
2. If the model is a stable reporting layer, prefer a clear aggregated grain with explicit dimensions.
3. If the KPI is non-additive, confirm the grain supports the intended rollup before exposing it as canonical.
4. If multiple grains exist for the same concept, separate canonical execution from helper or derived views instead of pretending they are interchangeable.

## Warnings

- Do not treat a pre-aggregated model as canonical for row-level analysis.
- Do not treat a row-level model as the only analyst-facing model if every common question requires repeated aggregation boilerplate.
- Grain ambiguity is usually a reason to split responsibilities, not to broaden one model indefinitely.
