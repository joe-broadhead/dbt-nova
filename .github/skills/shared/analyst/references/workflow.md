# Analyst Workflow

Use this workflow for business questions that must end in a reproducible answer with explicit evidence.

## Decompose the question first (required)

Extract:
- requested business output
- indicator(s), measure(s), or numerator/denominator components
- target grain
- time window
- filter(s)
- breakdown
- comparison mode
- requested unit or formatting
- trust requirements

Ask one clarification question only if a required element is missing or genuinely ambiguous.

Use this prompt when needed:

`To confirm: I will compute <indicator list> from <candidate entity> at <grain> filtered by <filters> over <time window>. If this should use a different entity, grain, or filter mapping, tell me before I run SQL.`

## Deterministic sequence

1. Confirm endpoint context only when needed.
   - Use `show_metadata` for project identity and manifest scope.
   - Use `health` when readiness is uncertain.
2. Classify the request.
   - recurring workflow
   - KPI answer
   - dimensional lookup
   - provenance or trust audit
3. Look for a recipe first when the request resembles a recurring workflow.
   - Try targeted `search_recipes`.
   - If that returns zero but the request still looks recurring, use `search_recipes {}` and narrow from the result set.
   - If recipe discovery is still inconclusive but the folder family is obvious, corroborate with `find_by_path`.
4. Resolve indicators directly, one requested indicator at a time.
   - Use `search_indicator` first.
   - Use `indicator_inventory` when comparing repeated definitions.
5. Choose one execution entity from the top shared parent, not isolated indicator rows.
   - If the requested indicators do not share a credible parent, do not force a synthetic combined query.
6. Confirm the compact semantic contract on that entity.
7. Verify execution fields only after the winning entity is chosen.
8. Validate filter values with bounded SQL before aggregation.
9. Run the final SQL or recipe.
10. Report the answer with explicit evidence.

Do not skip filter-value validation when the question includes geography, market, segment, or channel constraints.

## Recipe-first rule

Use recipes for deterministic recurring workflows such as:
- weekly reports
- reference packs
- reconciliations
- standard KPI decks

If a recipe fully covers the ask, prefer it.
If it only partially covers the ask, use it as the domain scaffold and continue discovery on the same execution entity.

Inspect recipe metadata first with `get_recipe include_queries=true include_sql=false`.
If a recipe contains an inventory or diagnostic query with no required parameters, run that first before the heavier steps.

## Entity selection rubric

Prefer the candidate that satisfies the most checks:
- explicit measure or metric definition
- explicit grain
- explicit time field
- required filter fields
- acceptable tests and metadata
- fewer assumptions for the requested output

If two candidates tie, prefer the one with clearer definitions and fewer assumptions.

Use `search_indicator` first for direct KPI resolution.
Use `indicator_inventory` when the task is cataloging a KPI family, comparing repeated definitions, or choosing among several plausible indicators before execution.
Use `search` for supporting discovery when the ask is not yet KPI-shaped.

When `search_indicator` returns multiple rows from one parent, reason from `parent_groups` before choosing an execution entity.

## Compact contract rule

Treat the transport's compact entity summary as the default contract check.

If `get_entity detail=standard` is available, use that.
If you need to compare multiple shortlisted parents quickly, use `batch_get_entities` on the top 2-3 `parent_unique_id` values from `search_indicator`.
Use `compare_grains` and `diff_entities` only when two parents remain plausible after compact inspection.

Use the compact summary to confirm:
- `nova_summary.grain`
- `nova_summary.measures`
- `nova_summary.metrics`
- `relation_name`
- `domains`
- `synonyms`

Use `get_context` only when the deployment exposes it and you need lineage, tests, and docs bundled together.

## SQL inspection rule

Use `get_columns` after choosing the entity to confirm:
- time field
- filter fields
- numerator / denominator fields for rate metrics
- requested breakdown columns

If the likely filter field is still unclear after the compact entity check, use the transport's column-discovery tool:
- `search_columns` when the deployment exposes text-ranked column search
- `column_inventory` when the deployment exposes deterministic semantic column listing

Use `get_sql` only when the deployment exposes it and SQL inspection is required.
Default to raw SQL unless the manifest definitely contains compiled SQL.

## Time standards

- Week: Sunday-Saturday
- YoY: 364-day day-of-week alignment
- Use same-date YoY only when explicitly requested
- Always report exact dates when resolving relative windows such as `last week`

## Filter validation rule

Validate actual warehouse values before writing final filters.
Do not assume friendly labels map directly to raw warehouse codes without checking live values.

Use the chosen relation and actual filter column in a bounded validation query before final aggregation.

Examples:
- country label -> ISO code
- channel label -> transaction or session channel value
- segment name -> exact dimension member value

If the question asks for multiple filters, validate each one that is not already explicit in the metadata.

## Trust escalation rule

Use trust and provenance tools proportionally:
- `get_context` when you need bundled lineage, tests, docs, and columns
- `get_lineage` / `get_column_lineage` when provenance matters
- `get_test_coverage` when reliability matters
- `get_metadata_score` when documenting caveats or choosing between similar entities

Do not front-load every trust tool by default. Escalate only when the answer is high-stakes or the entity choice is ambiguous.

## Output requirement

Every final answer must include:
- selected indicator definition(s)
- selected execution entity
- selected grain
- selected time field
- selected filter fields and validated values
- final SQL or recipe id/query names
- exact execution blocker if warehouse execution could not complete

Use the shared report template asset when the answer needs a formal handoff.
