# BI Engineer Workflow

Use this workflow when turning Nova-backed indicators and execution entities into dashboard-ready analytical products.

## Deterministic sequence

1. Clarify the business decision and primary audience.
2. Resolve the canonical indicators and the likely execution entity.
3. Confirm grain, time field, and supported breakdowns.
4. Validate filter fields and likely filter values on the selected entity.
5. Select recipes when an existing recurring report matches the requested artifact.
6. Choose chart and card patterns that match the question, grain, and audience.
7. Produce a dashboard spec, dataset contract, metric cards, and QA checklist.
8. Capture validation evidence, blocked checks, and known caveats.

## Core design rules

- Prefer one canonical execution entity per dashboard section unless there is a clear reason to mix sources.
- Prefer canonical indicators over rederived KPI logic.
- Confirm breakdown compatibility before choosing a chart.
- Keep filter behavior explicit: defaults, required filters, optional filters, and unsupported slices.
- Default weekly and multi-week comparisons to weekday-aligned YoY; default month, quarter, and year comparisons to calendar/date-aligned.
- Validate numerator, denominator, unit, and delta type for every rate, ratio, and average.
- Use `compare_grains` before mixing entities in one section.
- Do not use broad column search results as proof of support; confirm fields on the selected entity.
- Avoid unbounded warehouse execution. Use bounded samples only to validate shape, values, or examples.
- Treat the dashboard spec as an analytical contract, not only a mockup.

## Validation rule

Before finalizing a design, confirm:
- chosen indicators resolve on the intended execution entity
- grain supports the selected chart
- required dimensions exist on the entity output
- filter values or value families are real, not assumed
- any comparison pattern uses the same base grain
- rate and average rollups use the canonical expression, not summed precomputed rates
- unsupported filters or slices are explicitly listed

Use indicator inventory when the design starts from a KPI family rather than one explicitly named metric.
Use column discovery when the design starts from a filter or breakdown need and the supporting field is not yet obvious.
