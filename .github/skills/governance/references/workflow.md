# Governance Workflow

Use this workflow for deterministic metadata and quality audits that need frozen scope, explicit blocker extraction, and repeatable reruns.

## Deterministic sequence

1. Capture manifest identity.
2. Freeze one explicit scope contract before scoring.
3. Run the main audit on that frozen scope.
4. Extract blockers with targeted detail calls.
5. Build a remediation queue with retest conditions.
6. Refresh and rerun the exact same scope after fixes.

## Scope freeze rule

Define scope before scoring and keep it stable through reruns:
- resource types
- package, tag, or path filters
- include and exclude sets
- changed-file scope, if applicable

Do not change scope between the initial run and the rerun unless you are explicitly starting a new audit.

Prefer the narrowest scope that answers the governance question:
- exact entity id for a model-level gate
- path or tag for a bounded family
- package plus resource type for project baselines

Avoid broad inventory calls when an exact entity, path, or tag scope is already known.
For small path or tag scopes, score each returned entity directly. Do not replace the frozen path scope with an unrelated project-level score page.

## Gate rule

Use entity-level evidence for final pass/fail decisions.
Project-level summaries are useful for baselines and triage, not for the final governance decision on a specific entity.

Default gate policy when the user has not provided one:
- `metadata_score_below_a_grade` when entity grade is below `A`
- `owner_missing` when owner or governance owner metadata is absent
- `documentation_coverage_below_threshold` when documented/semantic coverage is below the stated or inferred threshold
- `test_coverage_missing` when governed keys, time fields, or critical measures are untested
- `pii_without_compliance_tags` when likely PII columns lack compliance tagging or need explicit review

## Blocker rule

Every blocker must be:
- explicit
- machine-checkable
- grouped into a remediation bucket
- paired with a retest condition

Do not invent blockers that Nova did not expose. If the tool surface cannot confirm a compliance condition, report `needs_review` as the required action rather than marking a false pass.

## Compliance scan rule

For PII or compliance audits:
- use column search/inventory to find candidate sensitive fields
- inspect the parent entity governance score before deciding pass/fail
- fail only when the parent entity lacks required PII, sensitivity, or compliance metadata
- use `needs_review` when the column looks sensitive but the available metadata cannot prove classification requirements

## Rerun rule

After remediation, rerun the same frozen scope and gate policy.
For MCP hosted endpoints, do not reload or mutate the server unless the task is explicitly about a refreshed manifest lifecycle.
