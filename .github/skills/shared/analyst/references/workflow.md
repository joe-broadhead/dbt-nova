# Analyst Workflow

Use this workflow for business questions that must end in a reproducible answer with explicit evidence.

## Decompose the question first

Extract:
- indicator(s)
- time window
- filter(s)
- breakdown
- comparison mode

Ask one clarification question only if a required element is missing or genuinely ambiguous.

Use this prompt when needed:

`To confirm: I will compute <indicator list> from <candidate entity> at <grain> filtered by <filters> over <time window>. If this should use a different entity, grain, or filter mapping, tell me before I run SQL.`

## Deterministic sequence

1. Check session readiness.
2. Look for a recipe first when the request resembles a recurring workflow.
3. Resolve indicators directly, one requested indicator at a time.
4. Choose one execution entity from the top shared parent, not isolated indicator rows.
5. Confirm the compact semantic contract on that entity.
6. Verify execution fields only after the winning entity is chosen.
7. Validate filter values with bounded SQL before aggregation.
8. Run the final SQL or recipe.
9. Report the answer with explicit evidence.

Do not skip filter-value validation when the question includes geography, market, segment, or channel constraints.

## Recipe-first rule

Use recipes for deterministic recurring workflows such as:
- weekly reports
- reference packs
- reconciliations
- standard KPI decks

If a recipe fully covers the ask, prefer it.
If it only partially covers the ask, use it as the domain scaffold and continue discovery on the same execution entity.

## Entity selection rubric

Prefer the candidate that satisfies the most checks:
- explicit measure or metric definition
- explicit grain
- explicit time field
- required filter fields
- acceptable tests and metadata

If two candidates tie, prefer the one with clearer definitions and fewer assumptions.

## Compact contract rule

Treat `get_entity detail=standard` as the default contract check.

Use it to confirm:
- `nova_summary.grain`
- `nova_summary.measures`
- `nova_summary.metrics`
- `relation_name`
- `domains`
- `synonyms`

Use `get_context` only when you need lineage, tests, and docs bundled together.

## SQL inspection rule

Use `get_columns` after choosing the entity to confirm:
- time field
- filter fields
- numerator / denominator fields for rate metrics

Use `get_sql` only when SQL inspection is required.
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

## Output requirement

Every final answer must include:
- selected indicator definition(s)
- selected execution entity
- selected time field
- selected filter fields and validated values
- final SQL or recipe id/query names
- exact execution blocker if warehouse execution could not complete

Use the shared report template asset when the answer needs a formal handoff.
