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
4. If specialized marts exist for performance or narrow business scope, keep them distinct from the canonical execution surface.
5. If multiple grains exist for the same concept, separate canonical execution from helper, aggregate, or derived views instead of pretending they are interchangeable.

## Warnings

- Do not treat a pre-aggregated model as canonical for row-level analysis.
- Do not treat a row-level model as the only analyst-facing model if every common question requires repeated aggregation boilerplate.
- Do not collapse legitimate aggregated marts into a row-level canonical model if they protect performance or encode a different grain.
- Grain ambiguity is usually a reason to split responsibilities, not to broaden one model indefinitely.
