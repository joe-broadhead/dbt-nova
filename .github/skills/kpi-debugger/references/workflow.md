# KPI Debugger Workflow

Use this workflow when a KPI changed unexpectedly, disagrees with another source, or no longer matches stakeholder expectations.

## Deterministic sequence

1. Restate the discrepancy precisely.
2. Resolve the canonical indicator and execution entity.
3. Confirm the exact time window, filters, and comparison basis.
4. Map every stakeholder filter to concrete fields and allowed values.
5. Reproduce one bounded current result with the canonical definition.
6. Reproduce the comparison result with the same contract.
7. Compare alternate entities, reports, or recipes only after the canonical result is stable.
8. Trace likely root causes through definition, filters, time, grain, lineage, freshness, and trust checks.
9. Document observed facts, hypotheses, blockers, and retest conditions.

## Investigation rules

- Start from the canonical indicator definition, not stakeholder shorthand.
- If stakeholder shorthand maps to several KPIs, use `indicator_inventory` before picking one to reproduce.
- Reproduce the KPI on one execution entity before comparing multiple candidates.
- Treat mismatched filters and time windows as the first-class failure mode.
- Use `search_columns` or `get_columns` to make the filter mapping explicit before blaming the metric definition.
- Use lineage and column lineage only after the discrepancy is concretely reproduced.
- For rates or averages, reproduce the numerator, denominator, and final metric separately.
- For weekly or multi-week windows, default to weekday-aligned YoY. For month, quarter, or year windows, default to calendar/date-aligned YoY.
- Treat failed SQL execution, preflight failures, and missing access as blockers, not root-cause evidence.
- Avoid unbounded SQL, broad exploratory scans, and full-project warmups.
- Prefer evidence-backed suspected causes over speculative explanations.

## Output requirement

Use the investigation template when handing off:
- incident statement
- canonical definition used
- execution entity
- comparison basis
- reproduction method
- evidence collected
- suspected causes
- blockers
- next actions and retest condition
