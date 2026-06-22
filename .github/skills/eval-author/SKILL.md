---
name: eval-author
description: "Designs, debugs, and operationalizes dbt-nova eval suites for manifest metadata quality and agent tool-use quality. Use when creating bridge evals, provider-backed agent evals, CI eval gates, regression suites for Nova metadata, or when interpreting dbt-nova eval artifacts and failures."
license: MIT
allowed-tools: "Bash Read Write mcp__nova__show_metadata mcp__nova__health mcp__nova__search mcp__nova__search_indicator mcp__nova__indicator_inventory mcp__nova__search_columns mcp__nova__column_inventory mcp__nova__get_entity mcp__nova__batch_get_entities mcp__nova__get_columns mcp__nova__get_context mcp__nova__get_lineage mcp__nova__get_column_lineage mcp__nova__get_test_coverage mcp__nova__get_metadata_score mcp__nova__search_recipes mcp__nova__get_recipe"
metadata:
  owner: "dbt-nova"
  persona: "eval-author"
  version: "0.0.4"
---

# Eval Author Skill (dbt-nova)

## Mission

Create Nova eval suites that prove two things separately:
- the manifest metadata bridge returns the right entities, indicators, context, lineage, recipes, and quality signals
- real agents use Nova tools in the intended order and cite the intended evidence

## First-principles contract (required)

Before writing evals, define:
- target persona or workflow
- manifest source and refresh cadence
- representative user questions or agent tasks
- expected canonical entities, indicators, fields, recipes, or lineage edges
- absolute time windows and comparison basis for any relative-date task
- whether the check belongs in a bridge eval, an agent eval, or both
- failure signal and owner action
- gate threshold and run cadence

Do not write assertions until you can name the ground-truth entity ids and explain why each assertion should be stable across normal manifest refreshes.

## Transport selection

- Use MCP for discovery when `mcp__nova__*` tools are available.
  - Read `references/workflow.md`
  - Read `references/assertion-patterns.md`
- Use the dbt-nova CLI through `Bash` for suite initialization, validation, bridge runs, provider-backed agent runs, and CI gates.
  - Read `references/provider-patterns.md` when authoring agent evals.

Eval execution is CLI-only. MCP is for discovering ground truth and debugging expected evidence before encoding it in a suite.

## Deterministic flow

1. Pick one persona workflow and one concrete question family.
2. Discover ground truth with Nova before writing YAML.
3. Write bridge evals first for deterministic metadata behavior.
4. Add agent evals only after bridge evals pass.
5. Keep each assertion tied to one failure reason.
6. Validate the suite shape with `dbt-nova eval validate`.
7. Run one case with `--case-id` before running the full suite.
8. Set a gate that matches the suite purpose.
9. Run the full suite with `--telemetry` before checking readiness gates.
10. Check readiness with `dbt-nova eval gate <suite_name> --json` for high-stakes, launch-readiness, or recurring production suites.
11. Read `card.md` for PR summaries, `results.json` for machines, `report.md` for assertion details, and tool traces for agent behavior.

## Design rules

- Prefer stable identifiers such as `unique_id`, `parent_unique_id`, recipe id, and column name.
- Use bridge evals for search rank, indicator rank, column rank, context fields, lineage edges, metadata score, recipe discovery, and generic tool success.
- Use agent evals for required tool calls, forbidden tools, ordering, selected entities, ranked entity evidence, safe parameter checks, and final-answer text.
- Do not use agent evals to compensate for weak metadata. Fix bridge failures first.
- Do not assert every possible tool call. Assert the minimum behavior that proves the workflow.
- Avoid brittle prose checks. Use `final_answer.must_contain` only for short, durable business terms.
- Freeze relative dates in agent tasks. Do not leave "last week", "this month", or "last 52 weeks" unresolved unless the eval is explicitly testing date interpretation.
- Keep smoke suites small and strict. Keep broader regression suites larger with realistic `--fail-under` thresholds.
- Put advisory launch-readiness thresholds in suite YAML as `gate: { threshold: <0.0-1.0> }`, then verify the latest full-suite telemetry with `dbt-nova eval gate <suite_name> --json`. Filtered `--case-id` runs are for iteration and do not satisfy configured gates.
- Never include secrets, credentials, raw SQL parameter maps, or private manifests in public eval artifacts.

## Output standard

When handing off an eval suite or eval fix, include:
- suite purpose and target persona
- manifest source assumption
- declared manifest scope and known gaps
- bridge cases and what each proves
- agent cases and what each proves
- gate threshold and intended run cadence
- command lines to validate and run
- expected artifacts and how to debug failures
- known gaps intentionally left out of scope

## Load order

- Read `references/workflow.md` first.
- Read `references/assertion-patterns.md` before writing or reviewing bridge cases.
- Read `references/provider-patterns.md` before writing or debugging agent cases.
- Use `assets/eval-suite-template.yml` only when creating a new suite skeleton.
