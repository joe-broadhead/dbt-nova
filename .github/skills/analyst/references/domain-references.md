# Analyst Domain References

Domain references are curated, human-authored guides for durable business
domains or recurring workflow families. Use them as supporting context after
semantic discovery, not as a substitute for Nova metadata or live validation.

## When To Load

Load a domain reference when:
- the user names a business domain, standard workflow, or known reference doc
- a recipe or semantic indicator points to a domain with documented gotchas
- the question crosses domains or asks which source should be trusted
- the answer is high-stakes and domain caveats could change interpretation

Do not spend tokens looking for domain references for simple point lookups when
the semantic contract is already complete.

## How To Use

1. Resolve governed indicators first with `search_indicator` for KPI-shaped
   questions.
2. Use the domain reference to confirm canonical entity choice, grain, required
   hygiene filters, gotchas, and cross-domain handoffs.
3. Verify current fields and filter values through Nova tools before execution.
4. Carry material caveats from the domain reference into the final answer.

If the domain reference conflicts with current Nova tool evidence, prefer the
current manifest/tool evidence and report the reference as stale or ambiguous.

## Evidence To Capture

When a domain reference materially affects the answer, cite:
- reference path or title
- canonical entity and grain it recommends
- required filters or exclusions it adds
- gotcha, caveat, or handoff rule applied
- any conflict with current `meta.nova`, lineage, tests, or freshness evidence

## Boundaries

- Domain references are curated sources of truth, not raw query dumps.
- Historical SQL, notebooks, and dashboards can provide examples but do not
  override `meta.nova.measures`, `meta.nova.metric(s)`, lineage, or tests.
- Do not force joins across incompatible grains because a domain reference names
  both entities. Ask for clarification or split the answer.
- Do not copy formulas from a domain reference when a governed Nova indicator
  already owns the expression.

## Authoring Template

The public convention is documented in `docs/features/domain-references.md`.
When authoring or reviewing a domain reference, switch to the `meta-authoring`
skill and load its `references/domain-reference-template.md`. The same skill
carries a synthetic starter example at `references/domain-reference-example.md`.
