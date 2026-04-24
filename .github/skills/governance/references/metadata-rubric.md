# Governance Metadata Rubric

## Decision model

Governance decisions are binary per entity:
- `pass`
- `fail` with explicit blocking reasons

Use this rubric together with entity-level metadata scoring.
If the user provides stricter thresholds, use those and quote them in the audit output.

## Canonical blocking reasons

Use these exact labels in reports:
- `missing_required_nova_fields`
- `metadata_score_below_a_grade`
- `documentation_coverage_below_threshold`
- `test_coverage_missing`
- `owner_missing`
- `pii_without_compliance_tags`
- `needs_review`

## Priority policy

- P0: PII or compliance blockers
- P1: Missing required fields or owner missing
- P2: Documentation threshold failures
- P3: Test coverage gaps when other blockers are already clear

Use `needs_review` only when the available Nova evidence is insufficient to prove pass or fail for a governed condition. Pair it with the exact evidence needed on rerun.
