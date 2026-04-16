# BI Engineer Workflow

Use this workflow when turning canonical Nova-backed models and indicators into dashboard-ready analytical products.

## Deterministic sequence

1. Clarify the business decision and primary audience.
2. Resolve the canonical indicators and execution entity.
3. Confirm grain, time field, and supported breakdowns.
4. Validate filter fields and likely filter values.
5. Choose chart and card patterns that match the grain and question shape.
6. Produce a dashboard spec, dataset contract, and QA checklist.
7. Capture the SQL or recipe references that back the design.

## Core design rules

- Prefer one canonical execution entity per dashboard section unless there is a clear reason to mix sources.
- Prefer canonical indicators over rederived KPI logic.
- Confirm breakdown compatibility before choosing a chart.
- Keep filter behavior explicit: defaults, required filters, optional filters, and unsupported slices.
- Treat the dashboard spec as an analytical contract, not only a mockup.

## Validation rule

Before finalizing a dashboard design, confirm:
- chosen indicators resolve on the intended execution entity
- grain supports the selected chart
- required dimensions exist on the entity output
- filter values or value families are real, not assumed
- any comparison pattern uses the same base grain

Use `indicator_inventory` when the design starts from a KPI family or dashboard section rather than one explicitly named metric.

Use `search_columns` or `column_inventory` when the design starts from a filter or breakdown need and the supporting field is not yet obvious.

## Output requirement

Use the shared BI assets when handing off a design:
- dashboard spec
- metric card template
- dataset contract
- viz QA checklist
