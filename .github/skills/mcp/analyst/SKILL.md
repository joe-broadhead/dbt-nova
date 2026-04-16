---
name: mcp-analyst
description: "Answers business questions through Nova MCP tools. Use when resolving KPIs, validating canonical indicators, choosing the right execution entity, running deterministic recipes, or executing bounded warehouse SQL with explicit evidence."
license: MIT
allowed-tools: "mcp__nova__search mcp__nova__search_indicator mcp__nova__search_recipes mcp__nova__get_recipe mcp__nova__run_recipe mcp__nova__get_entity mcp__nova__get_columns mcp__nova__get_sql mcp__nova__get_lineage mcp__nova__get_column_lineage mcp__nova__get_context mcp__nova__get_test_coverage mcp__nova__get_metadata_score mcp__nova__find_by_path mcp__nova__execute_sql mcp__nova__health mcp__nova__reload_manifest Read"
metadata:
  owner: "dbt-nova"
  persona: "analyst"
  version: "0.0.3"
---

# MCP Analyst Skill (dbt-nova)

## Mission

Turn business questions into reproducible answers with explicit evidence:
- which indicator definition was used
- which execution entity was selected
- which time and filter fields were validated
- which SQL or recipe produced the answer

Decompose every question into:
- indicator(s)
- time window
- filter(s)
- breakdown
- comparison

## Execution contract (required)

1. Preflight
- Run `health`.
- If status is not `ready`, run `reload_manifest` and wait for readiness.

2. Parse the question
- Extract indicators, time window, filters, breakdown, and comparison mode.
- Ask one clarification question only if a required element is missing or ambiguous.

3. Check for a recipe first
- For recurring workflows such as weekly reports, reference packs, and reconciliations:
  - run `search_recipes`
  - inspect with `get_recipe`
- Default `get_recipe` mode:
  - `include_queries: true`
  - `include_sql: false`
- Only request SQL text when you specifically need it and the recipe is renderable from the manifest.
- If a recipe fully covers the ask, prefer `run_recipe`.
- If it only partially covers the ask, use it as the domain scaffold and continue discovery on the same execution entity.

4. Resolve indicators directly
- Prefer `search_indicator`.
- Search one requested indicator at a time before combining them into a final answer.
- Prefer the top shared parent in `parent_groups` over isolated indicator rows.
- Use analyst `search` as supporting evidence when:
  - the indicator is ambiguous
  - you need broader entity context
  - you want to confirm the best execution entity

5. Confirm the execution entity
- Use `get_entity` with `detail: "standard"`.
- Treat `detail: "standard"` as the compact semantic contract:
  - `nova_summary.grain`
  - `nova_summary.measures`
  - `nova_summary.metrics`
  - `relation_name`
  - `domains`
  - `synonyms`
- Prefer entities with:
  - canonical indicator definitions
  - explicit grain
  - explicit time field
  - usable dimensions

6. Verify execution fields
- Use `get_columns` only after the winning entity is chosen.
- Confirm:
  - time field
  - filter fields
  - numerator / denominator fields for rate metrics
- Use `get_sql` only when SQL inspection is needed:
  - default to `compiled: false`
  - use `compiled: true` only when the manifest actually contains compiled SQL

7. Validate filter values before aggregation
- Use bounded `execute_sql` checks for distinct values or date coverage.
- Never assume mappings like `UK -> GB` without validating actual warehouse values.

8. Run final SQL or recipe
- Use recipe output when the recipe fully answers the question.
- Otherwise write final SQL from the canonical measure/metric definitions on the selected execution entity.
- Default weekly convention: Sunday-Saturday.
- Default YoY alignment: 364-day day-of-week alignment.

9. Report with evidence
- Always include:
  - selected indicator definitions
  - selected execution entity
  - selected time field
  - selected filter fields and validated values
  - final SQL or recipe id/query names
  - exact reason execution could not complete, if blocked

## Tool usage (quick map)

- `search_indicator`: primary KPI resolver
- `search`: supporting discovery and entity confirmation
- `search_recipes` / `get_recipe` / `run_recipe`: deterministic recurring workflows
- `get_entity` / `get_columns`: compact contract + field verification
- `get_sql`: model SQL inspection
- `get_lineage` / `get_column_lineage`: provenance checks when needed
- `get_test_coverage` / `get_metadata_score`: trust and quality signals
- `execute_sql`: bounded validation and final execution
- `health` / `reload_manifest`: session readiness

## SQL execution guardrails (required)

- For unfamiliar environments, run `execute_sql` preflight first.
- Read the provider from the preflight response before writing provider-specific SQL.
- Bound exploratory queries with `row_limit`, `byte_limit`, and related limits.
- `run_recipe` uses the same warehouse execution path as `execute_sql`.
- A warehouse auth or provider error is not a discovery failure; still return the chosen indicators, entity, fields, and final SQL.

## Output standard (required)

- State assumptions and grain explicitly.
- Include exact dates when resolving relative windows like “last week”.
- For rates, report deltas in percentage points when comparisons are requested.

## Validation checklist (copy and complete)

[ ] `health` checked
[ ] Recipe lookup performed for recurring workflow requests
[ ] Indicator resolution run through `search_indicator`
[ ] Execution entity selected and justified
[ ] Time column selected
[ ] Filter fields selected
[ ] Filter values validated with SQL
[ ] Time window specified explicitly
[ ] Measure or metric definitions verified
[ ] Final SQL or recipe result captured

## References

- `references/analysis-workflow.md`
- `references/tool-recipes.md`
