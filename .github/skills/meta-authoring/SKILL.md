---
name: meta-authoring
description: "Builds and reviews high-signal `meta.nova` and curated domain references for dbt resources. Use when adding Nova metadata, choosing canonical datasets or indicators, defining model-bound measures or metric templates, annotating columns, de-ranking helper models, authoring domain reference docs, or improving Nova MCP search and metadata scores."
license: MIT
allowed-tools: "Bash Read Write mcp__nova__show_metadata mcp__nova__search mcp__nova__search_indicator mcp__nova__indicator_inventory mcp__nova__search_columns mcp__nova__column_inventory mcp__nova__find_by_path mcp__nova__get_entity mcp__nova__get_context mcp__nova__get_columns mcp__nova__get_metadata_score mcp__nova__compare_grains mcp__nova__find_entity_overlap mcp__nova__modelling_consistency_report mcp__nova__get_undocumented"
metadata:
  owner: "dbt-nova"
  persona: "meta-authoring"
  version: "0.0.14"
---

# Meta Authoring Skill (dbt-nova)

## Mission

Author or review `meta.nova` so Nova can route agents to the right dbt resource, business definition, grain, and governance signal without adding noisy metadata.

Good metadata is:
- structurally valid
- stable business intent, not one-off report logic
- attached to the execution entity that owns the data or KPI
- discoverable through Nova search
- minimal enough to stay maintainable

## Required Frame

Before editing, state:
- entity classification: canonical dataset, helper/staging/intermediate, metric template, source, or column annotation
- business concept: dataset, measure, metric, dimension, identifier, time field, or governance signal
- existing surface: current indicators, columns, overlapping entities, and canonical definitions
- intended surface: entity fields, `measures`, `metric` / `metrics`, column `meta.nova`, or `search.candidates`
- validation path: local schema/audit checks if editing files, then Nova search/score checks after manifest refresh

Do not author a new canonical measure, metric, or dataset until the repeated-concept surface has been checked.

## Tool Strategy

Prefer Nova MCP for evidence:
- `show_metadata` to confirm the loaded project and manifest timestamp
- `search_indicator` and `indicator_inventory` for measures and metrics
- `search_columns` and `column_inventory` for column semantics
- `search`, `get_entity`, `get_context`, and `get_columns` for current contracts
- `compare_grains`, `find_entity_overlap`, or `modelling_consistency_report` when placement is unclear
- `get_metadata_score` and `get_undocumented` for review and scoring evidence

Use the CLI only when you are validating local file edits or a local manifest build. A hosted MCP endpoint reflects its deployed manifest; it cannot prove uncompiled local YAML changes and should not be reloaded from this skill.

## Authoring Rules

- Use entity metadata for stable routing: `canonical`, `domains`, `use_cases`, `synonyms`, `tier`, `grain`, and `governance`.
- Use `measures` when an aggregation belongs on the execution model.
- Use `metric` or `metrics` for reusable KPI templates; never set both on the same entity.
- Use column metadata only for identifiers, time fields, high-signal dimensions, measure columns, and genuine semantic disambiguation.
- Use `search.candidates.analyst: false` only for helper, ops, staging, or intermediate resources that should remain searchable but not rank first for analysts.
- Use canonical flags sparingly. Repeated business terms should have one preferred definition at the relevant level.
- Keep synonyms concise: 2-8 business terms is usually enough.
- Verify referenced fields exist: `grain.time_field`, `grain.dimensions`, `measures[].field`, and `recommended_filters[].field`.
- Treat PII and compliance as first-class governance signals. Prefer entity-level `governance` for dataset sensitivity and column-level governance only where a specific exposed column needs it.
- Use curated domain references for narrative guidance that does not belong in
  `meta.nova`: business context, scope and exclusions, standard hygiene filters,
  canonical-entity rationale, gotchas, and cross-domain handoffs. Domain
  references must point back to Nova metadata rather than replacing it.
- Do not use raw query corpora, notebook archives, or dashboard history as the
  source of authority for domain references. They can be supporting evidence
  only after the governed metadata and eval evidence are clear.

## Workflow

1. Read `references/workflow.md`.
2. Read one transport reference: `references/transport-mcp.md` for live Nova evidence, or `references/transport-cli.md` for local validation.
3. Load `references/decision-rules.md` when choosing a Nova surface or canonical placement.
4. Load `references/patterns.md` only when writing YAML.
5. Load `references/domain-reference-template.md` when authoring or reviewing a
   domain reference document. Use `references/domain-reference-example.md` only
   when a concrete synthetic example is useful.
6. Load `references/review-checklist.md` before final handoff.

## Output Standard

Final handoffs must include:
- classification and intended Nova surface
- metadata added or changed
- domain reference sections added or changed, if any
- repeated-concept and canonicality evidence
- validation result
- search or score verification result
- remaining caveats or follow-up cleanup

Do not dump full YAML unless the user asks for it or the review requires exact patch text.
