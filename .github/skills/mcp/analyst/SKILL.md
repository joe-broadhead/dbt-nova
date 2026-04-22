---
name: mcp-analyst
description: "Answers business questions through Nova MCP tools. Use when resolving KPIs, validating canonical indicators, choosing the right execution entity, running deterministic recipes, or executing bounded warehouse SQL with explicit evidence."
license: MIT
allowed-tools: "mcp__nova__show_metadata mcp__nova__health mcp__nova__search mcp__nova__search_indicator mcp__nova__indicator_inventory mcp__nova__search_recipes mcp__nova__get_recipe mcp__nova__run_recipe mcp__nova__get_entity mcp__nova__batch_get_entities mcp__nova__get_columns mcp__nova__search_columns mcp__nova__column_inventory mcp__nova__get_sql mcp__nova__get_lineage mcp__nova__get_column_lineage mcp__nova__get_context mcp__nova__get_test_coverage mcp__nova__get_metadata_score mcp__nova__find_by_path mcp__nova__compare_grains mcp__nova__diff_entities mcp__nova__list_entities mcp__nova__list_packages mcp__nova__list_tags mcp__nova__list_databases mcp__nova__execute_sql Read"
metadata:
  owner: "dbt-nova"
  persona: "analyst"
  version: "0.0.6"
---

# MCP Analyst Skill (dbt-nova)

## Transport contract

- Use `show_metadata` for fast project identity and manifest scope checks.
- Use `health` only when readiness is uncertain or a prior tool suggests startup, refresh, or cache issues.
- If `health.status` is not `ready` on a shared hosted endpoint, stop and report the readiness state. Do not mutate server state from an analyst workflow.
- `run_recipe` uses the same warehouse execution path as `execute_sql`.
- A warehouse auth or provider failure is an execution blocker, not a discovery failure.

## First-principles contract (required)

Before using tools, decompose the question into:
- requested business outputs
- indicator(s), measure(s), or numerator/denominator components
- requested breakdown or grouping grain
- filters and candidate filter fields
- time window and comparison mode
- expected unit and result shape
- trust requirements: lineage, tests, metadata quality, or reproducibility

If one required part is materially ambiguous, ask one clarification question before warehouse execution.

A question is not ready for final SQL until you can name:
- the execution entity
- the metric or measure definition
- the grouping grain
- the time field
- the filter field(s)
- the exact validated filter value(s)

## Tool strategy

- `search_indicator`: primary KPI resolver
- `indicator_inventory`: deterministic KPI catalog when ranked search is too narrow
- `search`: supporting discovery for non-KPI asks, domain cues, and entity confirmation
- `search_recipes` / `get_recipe` / `run_recipe`: deterministic recurring workflows
- `get_entity` or `batch_get_entities`: compact execution-entity inspection
- `compare_grains` / `diff_entities`: tie-breakers when multiple parents look plausible
- `get_columns`: final field verification on the chosen execution entity
- `search_columns` / `column_inventory`: help find filter fields or semantic column families
- `get_sql`: inspect metric logic or joins when SQL confirmation is required
- `get_context`: one-shot context bundle when you need columns, tests, lineage, and docs together
- `get_lineage` / `get_column_lineage`: provenance and trust
- `get_test_coverage` / `get_metadata_score`: quality and documentation signals
- `find_by_path` / `list_entities` / `list_packages` / `list_tags` / `list_databases`: scoped fallback discovery
- `execute_sql`: filter validation, preflight, and final warehouse execution

## Deterministic flow

1. Check endpoint context only when needed.
   - Use `show_metadata` for identity and scope.
   - Use `health` when readiness is uncertain.
2. Decide whether the ask is a recurring workflow.
   - If yes, try `search_recipes` before ad-hoc SQL.
   - If targeted recipe search returns zero but the ask still looks recurring, run `search_recipes {}` and narrow from that result set.
3. Resolve requested indicators one at a time.
   - Use `search_indicator` first.
   - Use `indicator_inventory` when comparing KPI families or repeated definitions.
   - Use `search` only for supporting discovery, not as the primary KPI resolver.
4. Choose one execution entity.
   - Prefer a common parent across requested indicators.
   - If no credible shared parent exists, do not force one query. Either answer in separate entity sections or ask for clarification.
   - Shortlist 1-3 parents only.
   - Use `get_entity` or `batch_get_entities`, then `compare_grains` / `diff_entities` if needed.
5. Confirm the semantic contract on the winning entity.
   - Verify grain, relevant metrics or measures, and available filter fields.
6. Verify execution fields only after choosing the entity.
   - Use `get_columns` first.
   - Use `search_columns` or `column_inventory` only when the likely filter field is still unclear.
7. Escalate trust checks only when useful.
   - Use `get_context` for bundled context.
   - Use lineage, test coverage, and metadata score for high-stakes or ambiguous answers.
8. Validate filter values with bounded SQL before final aggregation.
   - Never assume warehouse values such as `UK`, `GB`, or `United Kingdom` without checking.
9. Run final SQL or the chosen recipe.
   - Prefer recipe execution when the recipe fully covers the ask.
   - Use bounded execution and parameterized SQL.
10. Report with explicit evidence.

## Recipe rules

- Use `get_recipe include_queries=true include_sql=false` before `run_recipe`.
- When a recipe has an inventory or diagnostic query with no required parameters, run that first.
- Do not guess `recipe_id` values when discovery tools are available.
- If a recipe only partially covers the ask, use it as the scaffold and continue on the same execution entity.

## SQL execution guardrails

- Run `execute_sql preflight_only=true` when the environment or provider is uncertain.
- Use bounded limits on exploratory queries.
- Use `parameters` for dynamic user values instead of string interpolation.
- For rate metrics:
  - use the canonical metric expression when defined
  - otherwise compute from validated numerator and denominator components
- Default weekly standard: Sunday-Saturday.
- Default YoY alignment: 364-day day-of-week alignment unless the user requests otherwise.

## Output standard

Every final answer must include:
- selected indicator definition(s)
- selected execution entity
- selected grain
- selected time field
- selected filter field(s) and validated value(s)
- final SQL or recipe id/query names
- any trust caveat that materially affects interpretation

## Load order

- Read `../../shared/analyst/references/workflow.md` first.
- Load `references/tool-recipes.md` only when you need exact call shapes.
- Load the shared assets only when writing the final answer:
  - `../../shared/analyst/assets/evidence-block.md`
  - `../../shared/analyst/assets/report-template.md`

## References

- `../../shared/analyst/references/workflow.md`
- `../../shared/analyst/assets/evidence-block.md`
- `../../shared/analyst/assets/report-template.md`
- `references/tool-recipes.md`
