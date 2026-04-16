# Column Normalization Rules

## Goal

Reduce translation overhead caused by the same concept appearing under many different column names.

## Practical rules

- Prefer one stable business name per repeated concept where feasible.
- Keep physical compatibility constraints explicit when normalization is not possible.
- Normalize columns that drive:
  - filters
  - common breakdowns
  - canonical measures or metrics
  - dashboard joins or drill paths

## Anti-patterns

- same concept under multiple inconsistent business labels
- one canonical entity using different names than its near-peers for identical concepts
- dashboard logic compensating for inconsistent model naming

## Cleanup output

For each repeated concept, document:
- current names
- preferred normalized name
- canonical entity
- migration notes
