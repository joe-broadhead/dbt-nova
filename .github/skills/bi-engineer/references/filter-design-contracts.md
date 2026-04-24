# Filter Design Contracts

## Treat filters as part of the analytical contract

For each filter, document:
- field name
- label shown to users
- required vs optional
- default behavior
- whether the field was validated on the chosen execution entity
- allowed values or value family
- global vs chart-local scope

## Design rules

- Use only filters that exist on the execution entity output.
- Prefer filters that match the entity grain and intended audience vocabulary.
- Validate likely values before finalizing labels or defaults.
- Distinguish global filters from chart-local filters.
- Explicitly document unsupported filters rather than silently omitting them.
- For multi-entity dashboards, either use only filters shared by all affected entities or declare per-section filter behavior.
- Avoid exposing raw identifiers as filters unless the audience explicitly needs them.
