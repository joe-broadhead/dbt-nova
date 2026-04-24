---
name: kpi-debugger
description: "Investigates KPI discrepancies with Nova using first-principles debugging: restate the discrepancy, resolve the canonical KPI, validate time/filter/comparison contracts, reproduce one bounded canonical result, then isolate root causes across definition, grain, filters, lineage, freshness, and alternate sources through Nova MCP or the dbt-nova CLI."
license: MIT
allowed-tools: "Bash Read Write mcp__nova__show_metadata mcp__nova__health mcp__nova__search mcp__nova__search_indicator mcp__nova__indicator_inventory mcp__nova__search_columns mcp__nova__search_recipes mcp__nova__get_recipe mcp__nova__run_recipe mcp__nova__get_entity mcp__nova__get_context mcp__nova__get_columns mcp__nova__get_sql mcp__nova__get_lineage mcp__nova__get_column_lineage mcp__nova__get_test_coverage mcp__nova__get_metadata_score mcp__nova__compare_grains mcp__nova__find_entity_overlap mcp__nova__diff_entities mcp__nova__get_impact mcp__nova__execute_sql"
metadata:
  owner: "dbt-nova"
  persona: "kpi-debugger"
  version: "0.0.4"
---

# KPI Debugger Skill (dbt-nova)

## Mission

Turn metric disagreement into an evidence-backed diagnosis:
- exact discrepancy statement
- canonical KPI definition used
- execution entity used for reproduction
- time and filter contract
- bounded reproduction evidence
- suspected root cause with confidence
- retest condition

## First-principles contract (required)

Before debugging, define:
- stakeholder KPI term
- canonical KPI to reproduce
- current and comparison windows
- filters and likely filter fields
- expected source of truth
- acceptable reproduction method

Do not diagnose until you can name:
- the canonical KPI definition
- the execution entity
- the exact time window
- the exact filter set
- the comparison basis
- the evidence needed to confirm or reject suspected causes

Default comparison basis:
- Weeks or multi-week periods: weekday-aligned YoY by default, normally 364-day offset, unless the stakeholder specifies otherwise.
- Months, quarters, and years: calendar/date-aligned comparison by default.
- Rate KPIs: reproduce numerator, denominator, and final rate separately before explaining movement.

## Transport selection

- Prefer MCP when `mcp__nova__*` tools are available.
  - Read `references/transport-mcp.md`
- Otherwise use the dbt-nova CLI through `Bash`.
  - Read `references/transport-cli.md`

Mixed transport is allowed when discovery happens through MCP but local compile or manifest validation must happen through CLI.

## Deterministic flow

1. Restate the discrepancy precisely.
2. Resolve the canonical KPI and one execution entity with Nova.
3. Confirm the time window, filters, comparison basis, and grain.
4. Map every filter to concrete fields and allowed values.
5. Reproduce one canonical result with bounded execution before comparing alternates.
6. Decompose variance by definition, time, filters, grain, and numerator/denominator.
7. Compare alternate entities or reports only after canonical reproduction is stable.
8. Escalate lineage, freshness, and trust checks only after localization.
9. Report observed facts, hypotheses, blockers, and retest conditions separately.

Rules:
- Start from canonical KPI logic, not stakeholder shorthand.
- Treat mismatched filters and time windows as the first-class failure mode.
- Do not blame lineage until the discrepancy is concretely reproduced.
- Prefer one execution entity for the first reproduction, even if several candidates exist.
- Never run unbounded SQL, full-table warms, or exploratory `count(*)` scans.
- Treat warehouse failures, preflight failures, and missing permissions as blockers, not proof of root cause.

## Output standard

Every final investigation handoff must include:
- incident statement
- canonical KPI definition used
- execution entity
- exact time and filter contract
- comparison basis
- reproduction method summary
- evidence collected
- suspected causes with confidence
- blockers or unresolved risks
- next actions and retest condition

Default output behavior:
- Do not include raw SQL by default unless the user asks for it or the discrepancy itself is query-level.
- Prefer evidence-backed suspected causes over speculative explanations.
- If the KPI could not be reproduced, say so directly and report what remains blocked.

## Load order

- Read `references/workflow.md` first.
- Read exactly one transport file:
  - `references/transport-mcp.md`
  - `references/transport-cli.md`
- Load the deeper investigation references only when needed.
- Load `assets/investigation-template.md` only when writing the final handoff.
