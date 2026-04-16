---
name: cli-analyst
description: "Answers business questions through the dbt-nova CLI. Use when you have terminal access to `dbt-nova` but not direct MCP bindings. Optimized for recipe-first recurring workflows, canonical indicator discovery, compact semantic contract checks, bounded SQL execution, and evidence-first reporting."
license: MIT
allowed-tools: "Bash Read Write"
metadata:
  owner: "dbt-nova"
  persona: "analyst"
  transport: "cli"
  version: "0.0.3"
---

# CLI Analyst Skill (dbt-nova)

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

## Session contract

- Always pass `--manifest-path`.
- Reuse one `--storage-instance-id` for the whole analysis session.
- Always use `--json`.
- Prefer `--params-file` over long inline JSON.
- Treat `health check` with `ready_for_traffic=false` as a blocker for reliable search and SQL-driven answers.

## Required workflow

1. Preflight
- Run:
  - `dbt-nova health check --manifest-path /path/to/manifest.json --json`
- If not ready, reload first:
  - `dbt-nova manifest reload --manifest-path /path/to/manifest.json --json`
- If using `tool call`, `reload_manifest` is the only tool that does not require a manifest load first.

2. Parse the question
- Extract indicators, time window, filters, breakdown, and comparison mode.
- Ask one clarification question only if a required element is missing or ambiguous.

3. Check for a recipe first
- Recipes are for deterministic recurring workflows such as weekly reports, reference packs, reconciliations, and standard KPI decks.
- Run:
  - `tool call search_recipes`
  - `tool call get_recipe`
- Default `get_recipe` mode:
  - `include_queries=true`
  - `include_sql=false`
- Only request `include_sql=true` when you specifically need SQL text and the recipe is renderable from the manifest.
- If a recipe fully covers the ask, prefer `run_recipe`.
- If it only partially covers the ask, use it as the domain scaffold and continue discovery on the same execution model.

4. Resolve indicators directly
- Prefer `tool call search_indicator`.
- Search one requested indicator at a time before combining them into final SQL.
- Prefer the top shared parent entity in `parent_groups` over isolated indicator rows.
- Use analyst `search` as supporting evidence when:
  - the indicator is ambiguous
  - you need broader entity context
  - you want to confirm the best execution model

5. Confirm the execution entity
- Use `tool call get_entity` with `detail: "standard"`.
- Treat `get_entity detail=standard` as the compact semantic contract:
  - `nova_summary.grain`
  - `nova_summary.measures`
  - `nova_summary.metrics`
  - `relation_name`
  - `domains`
  - `synonyms`
- Prefer entities that have:
  - canonical indicator definitions
  - explicit grain
  - explicit time field
  - usable dimensions

6. Verify execution fields
- Use `tool call get_columns` only after the winning entity is chosen.
- Confirm:
  - time field
  - filter fields
  - numerator / denominator fields for rate metrics
- Use `tool call get_sql` when you need the model SQL:
  - default to `compiled=false`
  - use `compiled=true` only when the manifest actually contains compiled SQL

7. Validate filters with bounded SQL
- Use `tool call execute_sql` for distinct-value or range checks before aggregation.
- Bound exploratory SQL with `row_limit` and `byte_limit`.
- Never assume mappings like `UK -> GB` without validating actual warehouse values.
- `execute_sql` and `run_recipe` require valid warehouse env vars.

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

## Command patterns

### Recipe inventory

```bash
dbt-nova tool call search_recipes \
  --manifest-path /path/to/manifest.json \
  --storage-instance-id analyst-session \
  --params-file recipe-search.json \
  --json
```

Example `recipe-search.json`:

```json
{"query":"amplitude sessions","include_queries":true,"limit":10}
```

### Indicator discovery

```bash
dbt-nova tool call search_indicator \
  --manifest-path /path/to/manifest.json \
  --storage-instance-id analyst-session \
  --params-file indicator.json \
  --json
```

Example `indicator.json`:

```json
{"query":"checkout completion rate","persona":"analyst","limit":5}
```

### Compact semantic contract

```bash
dbt-nova tool call get_entity \
  --manifest-path /path/to/manifest.json \
  --storage-instance-id analyst-session \
  --params-file entity.json \
  --json
```

Example `entity.json`:

```json
{"id_or_name":"model.package.base__amplitude_sessions_sql","detail":"standard"}
```

### Filter validation SQL

```bash
dbt-nova tool call execute_sql \
  --manifest-path /path/to/manifest.json \
  --storage-instance-id analyst-session \
  --params-file sql.json \
  --json
```

Example `sql.json`:

```json
{
  "statement": "select country_code, count(*) as rows from analytics.base__amplitude_sessions group by 1 order by 2 desc",
  "row_limit": 50,
  "byte_limit": 50000
}
```

## Worked patterns

### Conversion and checkout completion

Question:
- `What was the conversion and checkout completion for the UK last week?`

Use this flow:
1. `search_recipes` for amplitude or weekly-report recipes.
2. `get_recipe` to inspect any matching recipe before deciding whether to run it.
3. `search_indicator` for `conversion rate`.
4. `search_indicator` for `checkout completion rate`.
5. Choose the shared parent from `parent_groups`.
6. `get_entity detail=standard` on that parent.
7. `get_columns` to confirm execution fields.
8. `execute_sql` to validate the country code.
9. `execute_sql` final query for the exact week window.

### Generic GMV ask

Question:
- `What was GMV for Spain last week?`

Use this flow:
1. `search_indicator` for the full question first, not only `gmv`.
2. Confirm the canonical parent entity from `parent_groups`.
3. `get_entity detail=standard` to confirm the canonical `gmv` measure and grain.
4. `get_columns` to verify time and country fields.
5. `execute_sql` to validate the Spain filter value.
6. Final SQL from the canonical base measure.

## Guardrails

- Prefer `search_indicator` for KPI resolution. Do not rely on broad `search` alone.
- Prefer `get_entity detail=standard` over `get_context` for routine analyst work.
- Use `get_context` only when you need lineage, tests, and docs bundled together.
- Use `get_recipe include_sql=false` by default. SQL rendering is stricter than metadata inspection.
- Use `get_sql compiled=false` by default. Compiled SQL may not be present in the manifest.
- If warehouse execution is unavailable, still return:
  - chosen indicators
  - chosen execution entity
  - chosen time/filter fields
  - final SQL
  - exact execution blocker

## References

- `docs/getting-started/cli.md`
- `docs/api/tools.md`
- `docs/features/recipes.md`
