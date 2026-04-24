---
name: project-cleanup
description: "Finds and plans structural cleanup in dbt projects with Nova using first-principles triage: freeze scope, detect overlap clusters, duplicate indicators, naming drift, and canonical conflicts, separate accidental duplication from legitimate specialization, quantify impact, and produce prioritized cleanup queues through Nova MCP or the dbt-nova CLI."
license: MIT
allowed-tools: "Bash Read Write mcp__nova__health mcp__nova__show_metadata mcp__nova__search mcp__nova__find_by_path mcp__nova__list_entities mcp__nova__search_indicator mcp__nova__indicator_inventory mcp__nova__search_columns mcp__nova__column_inventory mcp__nova__get_entity mcp__nova__get_columns mcp__nova__get_context mcp__nova__find_entity_overlap mcp__nova__modelling_consistency_report mcp__nova__compare_grains mcp__nova__diff_entities mcp__nova__get_lineage mcp__nova__get_impact mcp__nova__get_column_lineage mcp__nova__get_metadata_score"
metadata:
  owner: "dbt-nova"
  persona: "project-cleanup"
  version: "0.0.4"
---

# Project Cleanup Skill (dbt-nova)

## Mission

Turn structural mess into a prioritized cleanup plan with explicit evidence for:
- cleanup scope
- overlap clusters
- inconsistent naming or semantic duplication
- canonical conflicts or candidates
- cleanup priority
- downstream and column-level risk
- immediate versus later cleanup

## First-principles contract (required)

Before recommending cleanup, define:
- cleanup scope
- repeated concepts in scope
- likely overlap clusters
- canonicality questions
- downstream risk
- cleanup success criteria

Do not recommend cleanup work until you can name:
- the entities in scope
- the repeated concept or inconsistency
- grain differences between candidates
- whether the overlap is accidental duplication, legitimate specialization, country/source partitioning, or reporting derivation
- the likely canonical target, if one exists
- the impact and migration risk
- what can be cleaned immediately, what needs compatibility, and what should be left alone

## Transport selection

- Prefer MCP for scope inventory, overlap comparison, and cleanup prioritization when `mcp__nova__*` tools are available.
  - Read `references/transport-mcp.md`
- Use the CLI through `Bash` for deterministic exported artifacts and local project inspection.
  - Read `references/transport-cli.md`

Mixed transport is expected when discovery happens through MCP but exported artifacts or repo-local checks need CLI.

## Deterministic flow

1. Define the cleanup scope.
2. Baseline candidate entities in that scope.
3. Identify overlap clusters, duplicate indicators, and repeated column families.
4. Classify each cluster before proposing action.
5. Compare grains, columns, and semantic contracts across shortlisted clusters.
6. Separate canonical targets from helper, specialized, staging, or reporting entities.
7. Quantify downstream risk and choose cleanup priority.
8. Produce an overlap audit and cleanup queue.

Rules:
- Cleanup work is structural, not cosmetic.
- Repeated concepts are a design problem until a clear canonical target is justified.
- Column naming inconsistencies matter when they force downstream translation logic.
- Prefer a small number of clear analyst-facing execution models over many near-peers.
- Do not collapse country/source-partitioned staging tables unless a unification pattern and migration path are explicit.
- Do not prioritize low-impact duplication ahead of high-risk semantic ambiguity.
- Every cleanup recommendation should be backed by evidence, not taste.

## Output standard

Every final cleanup handoff must include:
- frozen scope
- overlap clusters
- canonical candidates
- key inconsistencies
- cleanup queue with priority
- downstream impact, compatibility, and validation notes

Default output behavior:
- Do not include raw tool dumps by default.
- Prefer a concise overlap audit with prioritized next actions and clear non-actions.

## Load order

- Read `references/workflow.md` first.
- Read the relevant transport file(s):
  - `references/transport-mcp.md`
  - `references/transport-cli.md`
- Load the deeper cleanup references only when needed.
- Load `assets/overlap-audit-template.md` only when preparing the final handoff.
