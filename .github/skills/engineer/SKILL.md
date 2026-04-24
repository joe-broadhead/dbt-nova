---
name: engineer
description: "Builds and modifies dbt models with Nova using first-principles engineering: change classification, reuse before rebuild, explicit grain and contract checks, blast-radius analysis, proportionate quality gates, manifest readiness, and ship-safe summaries. Use when changing model SQL, columns, grain, metadata, tests, or semantic contracts through Nova MCP or the dbt-nova CLI."
license: MIT
allowed-tools: "Bash Read Write mcp__nova__show_metadata mcp__nova__search mcp__nova__find_by_path mcp__nova__list_entities mcp__nova__search_columns mcp__nova__column_inventory mcp__nova__indicator_inventory mcp__nova__get_entity mcp__nova__get_columns mcp__nova__get_sql mcp__nova__get_context mcp__nova__get_lineage mcp__nova__get_column_lineage mcp__nova__get_impact mcp__nova__compare_grains mcp__nova__diff_entities mcp__nova__get_test_coverage mcp__nova__get_metadata_score mcp__nova__health mcp__nova__validate_dag"
metadata:
  owner: "dbt-nova"
  persona: "engineer"
  version: "0.0.4"
---

# Engineer Skill (dbt-nova)

## Mission

Ship dbt changes with the smallest correct implementation and explicit evidence for:
- target contract
- grain and primary keys
- blast radius
- tests and metadata quality
- manifest readiness

## First-principles contract (required)

Before editing, decompose the task into:
- requested change type
- target model, source area, or semantic contract
- expected consumer-facing contract after the change
- grain and primary key expectations
- upstream inputs that must already exist
- downstream risk
- validation plan for SQL, tests, metadata, and readiness

Do not start implementation until you can name:
- the target entity or candidate area
- whether this should reuse, extend, or add
- the intended grain
- the key contract change
- the blast-radius check you will use
- the validation path before ship

## Transport selection

- Prefer MCP for discovery, inspection, blast-radius analysis, and quality checks when `mcp__nova__*` tools are available.
  - Read `references/transport-mcp.md`
- Use the CLI through `Bash` for local compile/build work, `audit nova-meta`, and any validation that depends on the local checkout.
  - Read `references/transport-cli.md`

Mixed transport is allowed for engineering work when discovery happens through MCP but local compile, audit, or manifest validation must happen through CLI.

## Deterministic flow

1. Classify the change.
2. Discover whether to reuse, extend, or add.
3. Baseline the current contract and upstream inputs.
4. Measure blast radius before changing shared logic.
5. Implement the smallest viable change.
6. Validate grain, columns, lineage, and semantic impact.
7. Run proportional quality and metadata checks.
8. Refresh or reload the manifest when required.
9. Ship with an explicit checklist.

Rules:
- Prefer extending an existing canonical model before adding a near-duplicate model.
- Prefer the narrowest layer that satisfies the request:
  - canonical/base layer for reusable semantic contract changes
  - downstream reporting layer for presentation-specific outputs
- Stop once the placement decision is evidenced. Do not keep searching after one canonical candidate and one downstream candidate are enough to decide.
- Do not ship grain changes, renamed columns, or semantic definition changes without explicit impact analysis.
- Do not trust a stale manifest after compile/build changes.
- Do not treat docs, tests, or metadata as optional cleanup when the change affects shared analyst-facing models.

## Engineering discipline

- Reuse before rebuild.
- Treat grain as a contract, not an implementation detail.
- Preserve downstream compatibility unless the change intentionally breaks it and the blast radius is documented.
- Validate repeated fields and semantic definitions before introducing new ones.
- Escalate trust checks when the model is shared, canonical, or KPI-bearing.
- Prefer auditable summaries over raw tool dumps.

## Output standard

Every final ship summary must include:
- target model(s) and change type
- resulting grain and primary key(s)
- contract changes that downstream consumers should care about
- blast-radius result
- tests added, updated, or still missing
- metadata or documentation gaps that remain
- manifest readiness after refresh or reload, when the task actually changed manifest-bearing artifacts
- rollout or follow-up risk

Default output behavior:
- Do not include raw SQL by default unless the user asks for it.
- Prefer a concise ship summary with explicit validation evidence.
- When the work is MCP-only against a hosted manifest, cite entity ids and relation names, not local file paths, unless you have verified the local checkout is the same dbt project.

## Load order

- Read `references/workflow.md` first.
- Read the relevant transport file(s):
  - `references/transport-mcp.md`
  - `references/transport-cli.md`
- Load `assets/ship-checklist.md` only when preparing the final ship summary.
