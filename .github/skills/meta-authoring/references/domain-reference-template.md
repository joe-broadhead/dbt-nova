# Domain Reference Template

Use this template when a project needs a curated domain reference document next
to dbt models, metadata, or project docs. Domain references explain business
context that is too narrative for `meta.nova`, while still pointing back to the
canonical Nova metadata fields that agents should verify.

Domain references are curated sources of truth for agents and reviewers. They
are not raw query dumps, historical notebook archives, or replacements for
governed metadata. Keep them short, stable, and explicit about the canonical
entities and filters an analyst should use.

## Placement

Preferred locations:
- `models/<domain>/domain-reference.md`
- `models/marts/<domain>/domain-reference.md`
- `docs/domains/<domain>.md`
- a colocated path chosen by the downstream project and documented in its skill
  or agent setup

Use one document per business domain or durable workflow family. Do not create a
new domain reference for a one-off question.

## Template

Copy the sections below and replace bracketed placeholders. Remove sections only
when they truly do not apply, and say `Not applicable` when omitting a section
would hide an important boundary.

```markdown
# [Domain Name] Domain Reference

## Status

- Owner:
- Maintainers:
- Last reviewed:
- Applies to dbt project:
- Applies to Nova domains:
- Related eval suites:

## Business Context

[Explain the domain in 3-6 sentences. Include the business process, the common
analyst questions, and the decisions this domain supports.]

Nova metadata links:
- `meta.nova.domains`:
- `meta.nova.use_cases`:
- `meta.nova.synonyms`:
- `meta.nova.governance`:

## Canonical Entities

| Purpose | dbt unique_id | Relation | Why canonical | Nova metadata |
| --- | --- | --- | --- | --- |
| Primary execution dataset | `model.pkg.model_name` | `catalog.schema.table` | [Reason] | `canonical`, `grain`, `measures` |
| Supporting dimension | `model.pkg.dimension_name` | `catalog.schema.table` | [Reason] | `domains`, `grain` |

Guidance:
- Name the canonical execution model first.
- Prefer `unique_id` over friendly names.
- Explain when a source, staging, helper, or intermediate model should not be
  used directly by analysts.

## Grain And Scope

- Primary grain:
- Primary key:
- Time field:
- Default dimensions:
- Supported breakdowns:
- Unsupported breakdowns:

Scope rules:
- Included:
- Excluded:
- Known partial coverage:

Nova metadata links:
- `meta.nova.grain.primary_key`:
- `meta.nova.grain.time_field`:
- `meta.nova.grain.dimensions`:

## Standard Hygiene Filters

| Filter purpose | Field | Required values or rule | Applies by default? | Notes |
| --- | --- | --- | --- | --- |
| [valid records] | `[field_name]` | `[rule]` | yes/no | [Reason] |
| [market] | `[field_name]` | `[value list]` | yes/no | [Validation note] |

Guidance:
- Include required filters that protect correctness, such as valid statuses,
  canonical channels, or non-test records.
- Use exact coded values when known.
- Say which filter values must be validated in the warehouse before final SQL.

## Measures And Metrics

| Business term | Nova indicator | Type | Expression or definition | Parent entity |
| --- | --- | --- | --- | --- |
| [term] | `[measure_or_metric_name]` | measure/metric | `[definition]` | `model.pkg.model_name` |

Nova metadata links:
- `meta.nova.measures[]`:
- `meta.nova.metric` or `meta.nova.metrics[]`:
- `recommended_filters`:

Guidance:
- Prefer canonical Nova measures or metrics over prose formulas.
- If the domain reference explains a KPI formula, point to the metadata owner
  and do not duplicate a divergent expression.

## Dimensions And Filter Values

| Business concept | Field | Semantic type | Common values | Validation requirement |
| --- | --- | --- | --- | --- |
| [country] | `[field_name]` | `[semantic_type]` | `[coded values]` | [How to validate] |

Guidance:
- Include high-signal dimensions only.
- Map friendly labels to exact warehouse values when stable.
- Mark values that are examples, not authoritative enumerations.

## Common Workflows

| Workflow | Recommended path | Required checks | Output shape |
| --- | --- | --- | --- |
| [weekly report] | [recipe, metric, or entity] | [filters, dates, eval gate] | [tables, KPIs] |

Guidance:
- Link recurring workflows to Nova recipes or eval suites when available.
- Keep one-off report logic out of this section unless it is becoming standard.

## Cross-Domain Handoffs

| When the question asks for... | Stay in this domain when... | Hand off to... | Handoff evidence |
| --- | --- | --- | --- |
| [concept] | [condition] | [domain/entity] | [metadata, lineage, owner] |

Guidance:
- Document joins or handoffs that are safe.
- Document boundaries where analysts should ask a clarifying question.
- Do not imply that incompatible grains can be joined without caveats.

## Gotchas And Anti-Patterns

- [Do not use `model.pkg.helper_model` for analyst answers because...]
- [Do not compare X to Y without...]
- [Raw source tables are for provenance only unless...]

Guidance:
- Include only pitfalls that materially change answers.
- Prefer short rules an agent can apply before execution.

## Freshness, Tests, And Trust

- Freshness signal:
- Key tests:
- Metadata score target:
- Known caveats:
- Required caveats in final answers:

Nova metadata links:
- `meta.nova.governance`:
- test coverage:
- source freshness:
- eval gate:

## Maintenance

- Review cadence:
- Update triggers:
  - canonical model changed
  - `meta.nova.grain` changed
  - measure or metric definition changed
  - standard filter value changed
  - eval suite changed
- Required companion updates:
  - dbt YAML or `meta.nova`
  - skill/reference docs
  - eval suite or golden question

## Evidence Log

- [YYYY-MM-DD] [Short note about the evidence, issue, PR, or eval run that
  changed this reference.]
```

## Review Checklist

Before handing off a domain reference:
- Every canonical entity has a `unique_id`.
- Grain, time field, required filters, gotchas, and handoffs are explicit.
- The document points back to `meta.nova.domains`, `use_cases`, `grain`,
  `measures`, `metric(s)`, and `governance` where relevant.
- No section relies on a raw query corpus as authority.
- Synthetic examples use synthetic project names only.
- The document says when to ask for clarification instead of forcing a join or
  metric proxy.
