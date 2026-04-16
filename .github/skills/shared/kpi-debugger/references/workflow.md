# KPI Debugger Workflow

Use this workflow when a KPI changed unexpectedly, disagrees with another source, or no longer matches stakeholder expectations.

## Deterministic sequence

1. Restate the discrepancy precisely.
2. Resolve the canonical indicator and execution entity.
3. Confirm the exact time window, filters, and comparison basis.
4. Reproduce the current result with the canonical definition.
5. Compare against alternate entities, historical outputs, or reference workflows only after the canonical result is known.
6. Trace likely root causes through grain, filter, definition, lineage, and freshness checks.
7. Document evidence, suspected causes, and retest conditions.

## Investigation rules

- Start from the canonical indicator definition, not stakeholder shorthand.
- Reproduce the KPI on one execution entity before comparing multiple candidates.
- Treat mismatched filters and time windows as the first-class failure mode.
- Use lineage and column lineage only after the discrepancy is concretely reproduced.
- Prefer evidence-backed suspected causes over speculative explanations.

## Output requirement

Use the shared investigation template when handing off:
- incident statement
- canonical definition used
- execution entity
- reproduction SQL or recipe
- evidence collected
- suspected causes
- next actions and retest condition
