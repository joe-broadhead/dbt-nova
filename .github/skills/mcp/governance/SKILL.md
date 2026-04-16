---
name: mcp-governance
description: "Runs deterministic governance audits through Nova MCP tools. Use when enforcing metadata standards, scoring entity quality, extracting documentation/test gaps, and producing rerunnable pass/fail audit evidence."
license: MIT
allowed-tools: "mcp__nova__health mcp__nova__reload_manifest mcp__nova__list_entities mcp__nova__batch_get_entities mcp__nova__get_metadata_score mcp__nova__get_undocumented mcp__nova__get_test_coverage mcp__nova__get_entity mcp__nova__search Read"
metadata:
  owner: "dbt-nova"
  persona: "governance"
  version: "0.0.3"
---

# MCP Governance Skill (dbt-nova)

## Mission

Produce audit outputs that are deterministic, reproducible, and actionable:
- same scope -> same blockers
- explicit gates -> pass/fail decisions
- remediation queue with owners and retest criteria

## Execution contract (required)

1. Preflight
- Run `health`.
- If status is not `ready`, run `reload_manifest` and wait for readiness.
- Capture manifest identity from the response for audit evidence.

2. Freeze scope
- Define one explicit scope contract before scoring:
  - resource types
  - package / tag / path filters
  - explicit include/exclude sets if needed
- Reuse the same scope for reruns.

3. Deterministic inventory and scoring
- Use `list_entities` for scoped IDs.
- Use `get_metadata_score` for scoring:
  - project scope for broad summaries
  - entity scope for final pass/fail decisions
- Do not treat sampled project scoring as the final gate.

4. Extract blockers
- Use:
  - `get_undocumented`
  - `get_test_coverage`
  - `get_entity`
- Use governance `search` only as triage support, not as sole audit evidence.

5. Classify blockers
- Every blocker should be explicit and machine-checkable.
- Group blockers by:
  - documentation
  - tests
  - ownership / governance metadata

6. Recheck
- After fixes, run `reload_manifest` again.
- Re-run the same frozen scope.
- Compare blocker counts and gate outcomes, not just the overall score.

## Important boundary

- MCP currently does not expose the CLI-only `audit nova-meta` validator.
- Use MCP governance tools for metadata/test/documentation audits.
- Use the CLI validator separately when Nova schema/YAML validation is required.

## Output standard (required)

Always include:
- manifest identity
- frozen scope definition
- deterministic gate summary
- top blocking reasons with counts
- remediation queue with owner, priority, and retest condition

## Validation checklist (copy and complete)

[ ] Manifest ready and identity captured
[ ] Scope frozen before scoring
[ ] Inventory built from the same scope used for scoring
[ ] Entity-level gates computed for decisions
[ ] Documentation and test gaps captured
[ ] Blocking reasons categorized
[ ] Remediation queue includes retest criteria
[ ] Post-fix rerun uses the same scope

## References

- `references/metadata-rubric.md`
- `references/tool-recipes.md`
- `references/manifest-refresh.md`
