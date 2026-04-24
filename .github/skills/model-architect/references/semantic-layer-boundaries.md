# Semantic Layer Boundaries

## Principle

Nova semantics should live as close as possible to the real execution model that owns the business meaning.

## Good boundary

- canonical measures live on the execution entity
- reusable KPI templates live on the execution entity or a clearly justified semantic model
- helper models expose only the minimal metadata needed for routing or engineering workflows
- specialized marts expose semantics only for their explicit grain and scope

## Bad boundary

- business meaning lives only in thin wrappers
- search relies on naming conventions instead of stable semantics
- repeated metrics are copied across many sibling models without a canonical source
- reporting datasets become the only place where reusable KPI definitions exist

## Boundary decisions

When choosing where semantics live, ask:
- where does the data really live?
- which model has the correct analyst-facing grain?
- which model should be the default answer for repeated business questions?
- which other entities should remain searchable but not preferred?
- which specialized marts are intentionally separate?
- which semantic definitions must move, be aliased, or be deprecated?
