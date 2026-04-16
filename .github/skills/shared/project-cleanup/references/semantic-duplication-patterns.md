# Semantic Duplication Patterns

## Repeated measure duplication

Signals:
- identical or near-identical measures repeated across many entities
- no single preferred execution entity for the concept
- canonical flags scattered or contradictory

## Repeated metric duplication

Signals:
- similar KPI templates implemented in several places
- search finds many plausible parents for the same analyst term
- downstream consumers re-choose the KPI source every time

## Wrapper duplication

Signals:
- thin semantic wrappers exist only to expose KPIs already owned by a base model
- the wrapper adds little or no new business meaning
- discovery becomes noisier rather than clearer

## What to do

- pick the canonical source for repeated business meaning
- demote duplicates that should remain searchable but not preferred
- remove or consolidate wrappers that add no real semantic value
