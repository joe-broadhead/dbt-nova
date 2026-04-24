# Metric Discrepancy Playbook

## Clarify the discrepancy

Capture:
- KPI name as requested
- canonical KPI chosen
- expected value or source of truth
- observed value
- time window
- filters
- comparison target
- comparison basis
- exact grain expected by the stakeholder

## Reproduce before diagnosing

Always reproduce the KPI with:
- the canonical definition
- the selected execution entity
- explicit time boundaries
- explicit filters
- explicit grain
- bounded execution

Do not compare alternate sources until the canonical result is reproduced.

## Comparison order

1. Canonical query today
2. Canonical query for the prior comparison window
3. Numerator and denominator checks for rates or averages
4. Same KPI from alternate entities or reports
5. Grain and filter diffs across alternate entities
6. Upstream field or lineage checks

## Alignment defaults

- Weekly or multi-week periods: weekday-aligned YoY by default, usually 364 days.
- Monthly, quarterly, or yearly periods: calendar/date-aligned YoY by default.
- Constant-currency indicators: compare current constant-rate value against prior-year actual euro value when the metric definition says so.

## Typical discrepancy families

- definition mismatch
- filter mismatch
- time-window mismatch
- grain mismatch
- numerator / denominator mismatch
- currency or FX-rate mismatch
- stale or partial upstream data
- semantic routing to the wrong entity
