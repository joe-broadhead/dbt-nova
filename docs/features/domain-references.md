# Domain References

Domain references are curated Markdown guides that sit next to dbt models,
metadata, or project docs. They capture stable business context that is too
narrative for `meta.nova`: grain, scope, standard filters, canonical entities,
gotchas, cross-domain handoffs, and maintenance rules.

They are curated sources of truth for domain context. They complement Nova
metadata, and they do not replace `meta.nova`, eval suites, recipes, lineage,
tests, or warehouse validation.

## Why They Exist

Raw query corpora are noisy planning material. They often contain stale logic,
one-off filters, duplicated definitions, and bypasses around governed metrics.
Domain references give agents a curated layer between compact `meta.nova`
contracts and verbose project history.

Use a domain reference when a team needs to answer:
- Which model is canonical for this domain?
- What grain and time field should analysis use?
- Which filters are standard hygiene rules?
- Which measures or metrics are governed?
- Which gotchas should stop an agent before SQL execution?
- When should the question hand off to another domain?

## Placement

Recommended locations:

```text
models/<domain>/domain-reference.md
models/marts/<domain>/domain-reference.md
docs/domains/<domain>.md
```

Pick one convention per project and document it in your agent or skill setup.
Keep references close enough to model metadata that reviewers notice drift.

## Required Sections

Each domain reference should include:

| Section | Purpose | Nova metadata link |
| --- | --- | --- |
| Status | Owner, maintainers, review date, dbt project, related evals | owner, governance |
| Business context | Stable domain description and supported decisions | `domains`, `use_cases`, `synonyms` |
| Canonical entities | Preferred execution models, dimensions, and excluded helpers | `canonical`, `tier`, `grain` |
| Grain and scope | Primary key, time field, dimensions, inclusions, exclusions | `grain` |
| Standard hygiene filters | Required filters and exact coded values | columns, `recommended_filters` |
| Measures and metrics | Governed indicators and parent entities | `measures`, `metric`, `metrics` |
| Dimensions and values | High-signal filters and validation expectations | column `role`, `semantic_type`, `example_values` |
| Common workflows | Recurring recipes, evals, or output shapes | recipes, eval suites |
| Cross-domain handoffs | When to stay, ask, split, or hand off | lineage, domains |
| Gotchas | Rules that materially change answers | metadata, tests, lineage |
| Freshness and trust | Tests, freshness, caveats, score targets | governance, tests, freshness |
| Maintenance | Review cadence and companion update rules | PR/eval process |

The installable `meta-authoring` skill includes the copy-ready template at
`.github/skills/meta-authoring/references/domain-reference-template.md`.

## Authoring Rules

- Prefer `unique_id` over friendly model names.
- Keep prose short and durable. Put high-churn details in metadata, tests, or
  evals instead.
- Tie each canonical recommendation back to `meta.nova.domains`,
  `use_cases`, `grain`, `measures`, `metric(s)`, or `governance` when possible.
- Treat standard filter values as claims that need validation unless they are
  already enforced by metadata, tests, or accepted-values checks.
- Explain when to ask for clarification instead of joining incompatible grains.
- Include only gotchas that materially change answers.
- Do not use raw SQL history, query corpora, dashboards, or notebooks as the
  source of authority. They can be supporting evidence only.

## Synthetic Example

This example uses the synthetic `starter_eval` fixture names.

```markdown
# Starter Commerce Domain Reference

## Status

- Owner: analytics enablement
- Maintainers: dbt-nova example maintainers
- Last reviewed: 2026-06-23
- Applies to dbt project: `starter_eval`
- Applies to Nova domains: `commerce`
- Related eval suites: `evals/starter.yml`

## Business Context

The starter commerce domain covers synthetic order reporting for examples,
skills, and evals. Analysts use it to identify the canonical orders model,
resolve gross revenue, inspect customer geography, and run the weekly revenue
recipe. The domain is deliberately narrow: it proves Nova discovery, context,
lineage, recipes, metadata scoring, and agent tool-use behavior without relying
on a real warehouse or real business data.

## Canonical Entities

| Purpose | dbt unique_id | Relation | Why canonical |
| --- | --- | --- | --- |
| Primary execution dataset | `model.starter_eval.fct_orders` | `analytics.starter.fct_orders` | Owns order-grain reporting and the canonical gross revenue measure. |
| Customer geography dimension | `model.starter_eval.dim_customers` | `analytics.starter.dim_customers` | Supplies customer attributes and geography used by the order fact. |
| Raw sparse example | `model.starter_eval.raw_events_sparse` | `analytics.starter.raw_events_sparse` | Demonstrates sparse metadata and should not be the analyst landing page. |

## Grain And Scope

- Primary grain: one row per order
- Primary key: `order_id`
- Time field: `order_date`
- Default dimensions: `country_code`, `status`
- Supported breakdowns: order date, country code, status
- Unsupported breakdowns: session, product, shipment, and marketing attribution

## Standard Hygiene Filters

| Filter purpose | Field | Required values or rule | Applies by default? |
| --- | --- | --- | --- |
| Completed order reporting | `status` | validate accepted fixture values before filtering | no |
| Geography reporting | `country_code` | exact country code such as `GB` | no |

## Measures And Metrics

| Business term | Nova indicator | Type | Definition | Parent entity |
| --- | --- | --- | --- | --- |
| Gross revenue | `gross_revenue` | measure | Sum of `gross_amount` at order grain. | `model.starter_eval.fct_orders` |

## Cross-Domain Handoffs

- Stay in this domain for order revenue by date, status, or country.
- Hand off when the question asks about sessions, checkout conversion,
  marketing attribution, stock, margin, or fulfilment.
- Use `model.starter_eval.raw_events_sparse` only for sparse-metadata examples.
```

The full synthetic example is packaged with the `meta-authoring` skill at
`.github/skills/meta-authoring/references/domain-reference-example.md`.

## How Agents Should Use References

Analyst agents should:

1. Resolve governed indicators first with `search_indicator` for KPI-shaped
   questions.
2. Use the domain reference to confirm canonical entity, grain, filters,
   gotchas, and handoff boundaries.
3. Verify current fields, values, lineage, and trust evidence through Nova tools.
4. Report material caveats from the reference when they affect interpretation.

If the domain reference conflicts with current Nova tool evidence, treat the
reference as stale or ambiguous and prefer current manifest evidence.

Meta-authoring agents should:

1. Check repeated concepts with Nova search and indicator tools before writing.
2. Keep canonical facts in `meta.nova`; keep narrative guidance in the domain
   reference.
3. Update the relevant skill, reference doc, or eval when model metadata changes.
4. Validate docs and skills before handing off.

## Validation

For dbt-nova itself, validate packaged skills and docs:

```bash
bash scripts/install_skills.sh --all --skills-dir /tmp/dbt-nova-skills-test
uv run mkdocs build --strict
```

For downstream projects, add project-specific checks around the chosen reference
location. A future maintenance check can warn when dbt model or metadata changes
arrive without a matching skill, reference, or eval update.

## Related Guides

- [Nova Meta Overview](nova-meta-overview.md)
- [Nova Meta (Models)](nova-meta-models.md)
- [Nova Meta (Metrics)](nova-meta-metrics.md)
- [Agent Skills](../getting-started/skills.md)
