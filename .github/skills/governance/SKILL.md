---
name: governance
description: "Runs deterministic governance audits with Nova using frozen scope, repeatable scoring, explicit blocker extraction, and remediation planning. Use when enforcing metadata standards, testing and documentation gates, ownership and compliance requirements, and rerunnable audit evidence through MCP or the dbt-nova CLI."
license: MIT
allowed-tools: "Bash Read Write mcp__nova__show_metadata mcp__nova__health mcp__nova__list_entities mcp__nova__list_tags mcp__nova__list_packages mcp__nova__batch_get_entities mcp__nova__find_by_path mcp__nova__search mcp__nova__search_columns mcp__nova__column_inventory mcp__nova__get_entity mcp__nova__get_columns mcp__nova__get_metadata_score mcp__nova__get_test_coverage"
metadata:
  owner: "dbt-nova"
  persona: "governance"
  version: "0.0.4"
---

# Governance Skill (dbt-nova)

## Mission

Produce repeatable governance audits with explicit evidence for:
- manifest identity
- frozen scope
- pass/fail gate policy
- blocker counts and reasons
- remediation queue
- rerun readiness

## First-principles contract (required)

Before auditing, define:
- audit purpose
- frozen scope
- scoring or gate policy
- blocker taxonomy
- rerun path

Do not score until you can name:
- manifest source
- scope contract
- pass/fail basis
- blocker categories
- remediation output format

## Transport selection

- Prefer MCP for entity scoring, blocker extraction, and rerun checks when `mcp__nova__*` tools are available.
  - Read `references/transport-mcp.md`
- Use the CLI through `Bash` when you need `audit metadata-score`, `audit nova-meta`, or local manifest validation.
  - Read `references/transport-cli.md`

## Deterministic flow

1. Capture manifest identity and freeze scope.
2. Run the baseline audit on that exact scope.
3. Extract blockers with targeted detail calls.
4. Group blockers into remediation buckets.
5. Produce a remediation queue with retest conditions.
6. Rerun the same scope after fixes.

Rules:
- Do not change scope between baseline and rerun unless explicitly starting a new audit.
- Use machine-checkable blocker labels, not vague prose.
- Use entity-level evidence for final pass/fail decisions.
- Prefer narrow, explicit scopes. Avoid broad inventory calls unless the audit question truly requires them.
- Do not mutate or reload shared hosted MCP servers from a governance audit.

## Default gate policy

Use the user's policy when provided. Otherwise use this default:
- entity metadata grade must be `A`
- owner/governance metadata must be present when surfaced by Nova
- key documentation coverage must be sufficient for governed use
- governed primary keys, time fields, and critical measures must have test coverage or an explicit gap
- suspected PII columns must have compliance tags or a review blocker

## Output standard

Every final audit output must include:
- manifest identity
- frozen scope
- scoring basis
- pass/fail counts
- blocker counts by type
- top failing entities
- remediation queue with priority and retest condition

For a single-entity audit, report one pass/fail decision and the specific remediation queue. Do not pad the answer with project-level rollups.

## Load order

- Read `references/workflow.md` first.
- Read the relevant transport file(s):
  - `references/transport-mcp.md`
  - `references/transport-cli.md`
- Load assets only when producing the formal audit handoff.
