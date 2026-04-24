# Semantic Duplication Patterns

## Repeated measure duplication

Signals:
- identical or near-identical measures repeated across many entities
- no single preferred execution entity for the concept
- canonical flags scattered or contradictory
- duplicate indicators have incompatible grains

## Repeated metric duplication

Signals:
- similar KPI templates implemented in several places
- search finds many plausible parents for the same analyst term
- downstream consumers re-choose the KPI source every time
- metrics differ only by hidden filters or wrapper logic

## Wrapper duplication

Signals:
- thin semantic wrappers exist only to expose KPIs already owned by a base model
- the wrapper adds little or no new business meaning
- discovery becomes noisier rather than clearer

## Staging-family duplication

Signals:
- many source-specific or country-specific staging models have identical columns
- downstream impact is low or absent
- consolidation would require source-ingestion architecture changes

Treat as lower-priority hygiene unless it creates real discovery, governance, or maintenance pain.

## What to do

- pick the canonical source for repeated business meaning
- demote duplicates that should remain searchable but not preferred
- remove or consolidate wrappers that add no real semantic value
- document intentional specialization and non-actions
