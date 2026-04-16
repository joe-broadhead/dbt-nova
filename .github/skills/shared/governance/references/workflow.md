# Governance Workflow

Use this workflow for deterministic metadata and quality audits that need frozen scope, explicit blocker extraction, and repeatable reruns.

## Deterministic sequence

1. Check session readiness and capture manifest identity.
2. Freeze one explicit scope contract before scoring.
3. Run the main audit on that frozen scope.
4. Extract blockers with targeted detail calls.
5. Build a remediation queue with retest conditions.
6. Refresh and rerun the exact same scope after fixes.

## Scope freeze rule

Define scope before scoring and keep it stable through reruns:
- resource types
- package / tag / path filters
- include / exclude sets
- changed-file scope, if applicable

Do not change scope between the initial run and the rerun unless you are explicitly starting a new audit.

## Gate rule

Use entity-level evidence for final pass/fail decisions.
Project-level summaries are useful for baselines and triage, not for the final governance decision on a specific entity.

## Blocker rule

Every blocker must be:
- explicit
- machine-checkable
- grouped into a remediation bucket
- paired with a retest condition

Use blocker families such as:
- docs
- tests
- ownership / governance fields
- Nova metadata issues

## Output requirement

Every audit output should include:
- manifest identity
- frozen scope definition
- deterministic gate summary
- blocking reasons with counts
- remediation queue with owner, priority, and retest condition

Use the shared governance audit template asset when producing a formal audit handoff.
