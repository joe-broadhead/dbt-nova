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

If the user does not specify a comparison mode and the ask is about a period rather than a point lookup, choose the default comparison basis from the time rules below before execution.

Ask one clarification question only if a required element is missing or genuinely ambiguous.

Use this prompt when needed:

`To confirm: I will compute <indicator list> from <candidate entity> at <grain> filtered by <filters> over <time window>. If this should use a different entity, grain, or filter mapping, tell me before I run SQL.`

## Deterministic sequence

1. Classify the request:
   - recurring workflow
   - KPI answer
   - dimensional lookup
   - provenance or trust audit
2. Look for a recipe first only when the request is for the recurring deliverable itself.
3. For KPI, metric, measure, rate, funnel, or conversion questions, resolve
   indicators directly before broad model search, one requested indicator at a
   time.
   - Use domain-specific query terms and compact indicator search defaults.
4. Choose one execution entity from the top shared parent, not isolated indicator rows.
   - If the requested indicators do not share a credible parent, do not force a synthetic combined query.
5. Confirm the compact semantic contract on that entity.
6. Verify execution fields only after the winning entity is chosen.
7. Validate filter values with bounded execution before aggregation.
8. Run the final SQL or recipe.
9. Report the answer with explicit evidence.

Do not skip filter-value validation when the question includes geography, market, segment, or channel constraints.

## Domain reference rule

Domain references are curated guides for durable domains or recurring workflow
families. Use them after semantic discovery when they can clarify canonical
entities, grain, standard hygiene filters, gotchas, or cross-domain handoffs.

Do not use domain references as raw query corpora or as replacements for
current Nova tool evidence. If a reference conflicts with `meta.nova`, lineage,
tests, or freshness evidence, prefer the current manifest evidence and report
the reference as stale or ambiguous.

When a domain reference materially changes the answer, capture:
- reference path or title
- canonical entity and grain it recommends
- required filters or exclusions it adds
- gotcha, caveat, or handoff rule applied

## Semantic-first gate

For KPI-shaped asks, broad model search is allowed only after semantic
discovery is exhausted or shown to be irrelevant. Do not call broad `search`,
`get_context`, `get_sql`, or `execute_sql` before `search_indicator` unless the
ask is not KPI-shaped or a recipe already answers the recurring deliverable.

Semantic discovery evidence must include:
- the `search_indicator` query terms used
- whether you searched metrics, measures, or both
- the top relevant indicator names and parent entities when present
- why the top semantic result was accepted or rejected

Fallback to raw model search only when one of these is true:
- no relevant indicator is returned
- relevant indicators lack a credible execution parent for the ask
- requested outputs are dimensional, provenance, or entity-comparison work, not
  KPI work
- a recipe covers the deliverable and supplies the contract

If fallback is used, carry the fallback reason into the final evidence block.

## Recipe-first rule

Use recipes for deterministic recurring workflows such as:
- weekly reports
- reference packs
- reconciliations
- standard KPI decks

Do not force recipe discovery first when the user is really asking for:
- source selection
- provenance
- trust caveats
- entity comparison
- definition review

If a recipe fully covers the ask, prefer it.
If it only partially covers the ask, use it as the domain scaffold and continue discovery on the same execution entity.
Do not reopen broad entity discovery after a recipe already answers the question unless you need one missing contract detail or a material caveat check.

Inspect recipe metadata first. If a recipe contains an inventory or diagnostic query with no required parameters, run that first.

## Entity selection rubric

Prefer the candidate that satisfies the most checks:
- explicit measure or metric definition
- explicit grain
- explicit time field
- required filter fields
- acceptable tests and metadata
- fewer assumptions for the requested output

If two candidates tie, prefer the one with clearer definitions and fewer assumptions.

Use direct KPI resolution first.
Use supporting discovery only when the ask is not yet KPI-shaped or semantic
discovery produced an explicit fallback reason.

Low-token KPI discovery defaults:
- `search_indicator` with `limit: 3`
- `detail: compact`
- `group_mode: top`
- `indicator_types: ["metric"]` for rate, conversion, funnel, or ratio questions unless you are explicitly searching for raw measures
- `include_support_signals: true` when the question includes filter values
- `include_support_signals: false` only when definitions and parent evidence are already clear and no filter-value mapping is needed

When a metric row returns an `expression`, use that expression as the contract
for SQL. Do not replace it with a similarly named measure or a guessed
numerator/denominator.

When compact indicator rows include `relation_name`, `grain`, and metric
`expression`, treat them as sufficient execution evidence. Do not spend a tool
call on `DESCRIBE`, `information_schema`, or full context unless execution
actually fails because the contract is incomplete.

## Contract check rule

Treat the transport's compact entity summary as the default contract check.

Use the compact summary to confirm:
- grain
- measures
- metrics
- relation name
- domains
- synonyms

Use bundled context only when you need lineage, tests, and docs together.

## Field verification rule

After choosing the entity, confirm:
- time field
- filter fields
- numerator / denominator fields for rate metrics
- requested breakdown columns

If the likely filter field is still unclear after the compact entity check, use the transport's column-discovery tool.

## Time standards

- Week: Sunday-Saturday
- Closed weeks or periods expressed in full weeks:
  - default YoY is 364-day day-of-week alignment
- Months, quarters, years, or periods expressed in months or years:
  - default YoY is same-calendar-date prior-year comparison
- Use a different comparison basis only when explicitly requested or when the default would be misleading
- Always report exact dates when resolving relative windows such as `last week`
- Always report both the current period and the prior comparison period in final analysis output

## Filter validation rule

Validate actual warehouse values before writing final filters.
Do not assume friendly labels map directly to raw warehouse codes without checking live values.
Use the smallest practical validation slice near the target period:
- target period itself when possible
- otherwise a short adjacent lookback such as the last 7-30 days
- avoid year-plus scans for simple member validation

Examples:
- country label -> ISO code
- channel label -> transaction or session channel value
- segment name -> exact dimension member value

If the question asks for multiple filters, validate each one that is not already explicit in the metadata.

## Trust escalation rule

Use trust and provenance tools proportionally:
- bundled context when you need columns, lineage, tests, and docs together
- lineage when provenance matters
- test coverage when reliability matters
- metadata score when documenting caveats or choosing between similar entities

Do not front-load every trust tool by default. Escalate only when the answer is high-stakes or the entity choice is ambiguous.

## Output requirement

Every final answer must include:
- selected indicator definition(s)
- selected execution entity
- selected grain
- selected time field
- selected filter fields and explicit validated values, including coded values such as `country_code = 'GB'`
- comparison basis and exact comparison period when applicable
- recipe id/query names when used, or a concise calculation-method summary when direct execution was used
- exact execution blocker if warehouse execution could not complete

Keep interpretation separate from measurement:
- first state what the data says
- then state how it was computed
- only then add business interpretation if the user wants it

Default period-analysis answer shape:
- current value
- prior-year comparator
- absolute delta
- percentage delta
- short note on the comparison basis
