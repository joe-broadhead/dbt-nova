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

## Reproduce before diagnosing

Always reproduce the KPI with:
- the canonical definition
- the selected execution entity
- explicit time boundaries
- explicit filters

Do not compare alternate sources until the canonical result is reproduced.

## Comparison order

1. Canonical query today
2. Canonical query for the prior comparison window
3. Same KPI from alternate entities or reports
4. Upstream field or lineage checks

## Typical discrepancy families

- definition mismatch
- filter mismatch
- time-window mismatch
- grain mismatch
- stale or partial upstream data
- semantic routing to the wrong entity
