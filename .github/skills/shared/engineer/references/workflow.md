# Engineer Workflow

Use this workflow for dbt model changes that must ship with explicit blast-radius, quality, and readiness checks.

## Design checklist

- define grain and primary keys
- confirm required dimensions exist upstream
- reuse or extend before adding a new model
- document the model and key columns
- add or update tests for critical fields

## Deterministic sequence

1. Check session readiness.
2. Discover the target model or source area.
3. Validate upstream inputs and current SQL behavior.
4. Run blast-radius analysis before changing logic or grain.
5. Run quality gates.
6. Validate Nova metadata when YAML or `meta.nova` changes and the transport supports it.
7. Refresh the manifest after compile/build.
8. Complete the ship checklist.

## Required discovery rule

Prefer:
- direct reuse of an existing model
- extension of an existing canonical execution model
- adding a new model only when the current project shape clearly cannot support the use case cleanly

## Quality expectations

- docs on key columns and measures
- tests for primary keys, not-null, and relationships
- acceptable metadata completeness for domains, measures, and use cases
- clear downstream impact statement before shipping

## Blast-radius rule

Do not skip blast-radius analysis for:
- grain changes
- renamed columns
- semantic definition changes
- changes to shared execution models

Use column lineage for critical columns when a metric or KPI depends on them.

## Output requirement

Every ship summary should include:
- target model and grain
- change summary
- downstream impact
- tests added or still missing
- metadata score / doc gaps
- manifest readiness after refresh
