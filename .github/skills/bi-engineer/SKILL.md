---
name: bi-engineer
description: "Designs dashboard-ready analytical products with Nova using first-principles BI engineering: start from the decision, audience, and consumer artifact; resolve canonical indicators; choose execution entities and grain; validate filters, comparisons, and bounded sample outputs; then produce dashboard specs, dataset contracts, metric cards, and viz QA handoffs through MCP or the dbt-nova CLI."
license: MIT
allowed-tools: "Bash Read Write mcp__nova__health mcp__nova__show_metadata mcp__nova__search mcp__nova__search_indicator mcp__nova__indicator_inventory mcp__nova__search_columns mcp__nova__column_inventory mcp__nova__find_by_path mcp__nova__search_recipes mcp__nova__get_recipe mcp__nova__run_recipe mcp__nova__get_entity mcp__nova__get_context mcp__nova__get_columns mcp__nova__get_sql mcp__nova__compare_grains mcp__nova__diff_entities mcp__nova__find_entity_overlap mcp__nova__get_lineage mcp__nova__get_column_lineage mcp__nova__get_test_coverage mcp__nova__get_metadata_score mcp__nova__get_impact mcp__nova__execute_sql"
metadata:
  owner: "dbt-nova"
  persona: "bi-engineer"
  version: "0.0.4"
---

# BI Engineer Skill (dbt-nova)

## Mission

Turn business reporting needs into dashboard-ready analytical contracts with explicit evidence for:
- business decision and audience
- consumer artifact and refresh expectation
- canonical indicator definitions
- chosen execution entity and grain
- supported filters and breakdowns
- comparison and formatting semantics
- chart or card rationale
- validation and caveats

## First-principles contract (required)

Before designing, decompose the request into:
- business decision
- primary audience
- consumer artifact
- indicator set
- target grain, refresh cadence, and comparison pattern
- required sections, cards, or views
- required filters and breakdowns
- validation plan

Do not finalize a design until you can name:
- the consumer artifact
- the execution entity for each section
- the grain and time field
- the supported filters and breakdowns
- the canonical indicator definitions
- the validation evidence

Default comparison basis:
- Weekly or multi-week dashboards: weekday-aligned YoY by default, normally 364-day offset.
- Monthly, quarterly, and yearly dashboards: calendar/date-aligned comparisons by default.
- Rate, ratio, and average cards: validate numerator, denominator, unit, and delta language before finalizing.

## Transport selection

- Prefer MCP for discovery, filter validation, and compact contract checks when `mcp__nova__*` tools are available.
  - Read `references/transport-mcp.md`
- Use the CLI through `Bash` when you need local compile/build validation or deterministic local tool execution.
  - Read `references/transport-cli.md`

## Deterministic flow

1. Start from the decision and audience.
2. Resolve the canonical indicators.
3. Choose one execution entity per section unless there is a strong reason to split.
4. Confirm grain, time field, filters, and breakdowns.
5. Validate likely filter values and comparison assumptions on the chosen entity.
6. Use recipes when a matching recurring report exists; otherwise design from entity contracts.
7. Run bounded sample validation only when it reduces ambiguity.
8. Choose chart and card patterns that fit the grain and audience decision.
9. Produce the dashboard spec, dataset contract, metric cards, and QA checklist.

Rules:
- Prefer canonical indicators over rederived KPI logic.
- Prefer one canonical execution entity per dashboard section.
- Do not design filters or breakdowns that are not supported by the chosen entity.
- Anchor column search to the selected entity before treating fields as supported.
- Use `compare_grains` before mixing entities in one section.
- Never run unbounded SQL, full-table scans, or full manifest warmups from this skill.
- Treat warehouse failures as validation blockers, not evidence that a dashboard design is correct.
- Treat the dashboard spec as an analytical contract, not a visual mockup.

## Output standard

Every final design handoff must include:
- objective and audience
- chosen indicators
- chosen execution entity and grain
- time field and comparison logic
- supported filters and breakdowns
- chart and card rationale
- validation evidence and known caveats
- unsupported requests or blocked validations

## Load order

- Read `references/workflow.md` first.
- Read the relevant transport file(s):
  - `references/transport-mcp.md`
  - `references/transport-cli.md`
- Load design references only when needed.
- Load assets only when producing the final handoff.
