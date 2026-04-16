# Filter Design Contracts

## Treat filters as part of the analytical contract

For each filter, document:
- field name
- label shown to users
- required vs optional
- default behavior
- whether the field was validated on the chosen execution entity

## Design rules

- Use only filters that exist on the execution entity output.
- Prefer filters that match the entity grain and intended audience vocabulary.
- Validate likely values before finalizing labels or defaults.
- Distinguish global filters from chart-local filters.
- Explicitly document unsupported filters rather than silently omitting them.

## Common filter classes

- time
- geography / market
- channel
- platform / device
- product / category
- customer segment
