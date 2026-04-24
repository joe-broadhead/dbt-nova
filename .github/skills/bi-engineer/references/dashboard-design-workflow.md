# Dashboard Design Workflow

## Start from the decision

Capture:
- who will use the dashboard
- what decision it supports
- what artifact is being created
- refresh cadence and latency expectations
- which indicators must be trusted as canonical
- which filters and breakdowns are essential

## Build section by section

For each section:
1. define the question the section answers
2. choose the canonical indicator(s)
3. choose the execution entity
4. confirm grain and supported breakdowns
5. choose a chart or card pattern
6. define comparison and formatting semantics
7. document validation, blockers, and caveats

## Recommended section order

1. KPI cards
2. primary trend
3. main segmentation views
4. diagnostic or drill views

## Handoff expectation

Every section should be backed by:
- explicit indicator definitions
- one execution entity or an explicit grain comparison
- a time field and comparison basis
- supported filters and unsupported slices
- validation method, recipe, or bounded sample reference
