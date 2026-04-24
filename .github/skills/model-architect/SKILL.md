---
name: model-architect
description: "Improves dbt project structure with Nova using first-principles architecture work: define the repeated business concept, scope candidate clusters, compare overlap, grain, semantics, lineage, and blast radius, choose canonical execution surfaces, and produce migration-safe refactor plans through Nova MCP or the dbt-nova CLI."
license: MIT
allowed-tools: "Bash Read Write mcp__nova__health mcp__nova__show_metadata mcp__nova__search mcp__nova__find_by_path mcp__nova__list_entities mcp__nova__search_indicator mcp__nova__indicator_inventory mcp__nova__search_columns mcp__nova__column_inventory mcp__nova__get_entity mcp__nova__get_sql mcp__nova__get_columns mcp__nova__get_context mcp__nova__find_entity_overlap mcp__nova__modelling_consistency_report mcp__nova__compare_grains mcp__nova__diff_entities mcp__nova__get_lineage mcp__nova__get_impact mcp__nova__get_column_lineage mcp__nova__get_metadata_score"
metadata:
  owner: "dbt-nova"
  persona: "model-architect"
  version: "0.0.4"
---

# Model Architect Skill (dbt-nova)

## Mission

Improve project shape with explicit evidence for:
- canonical execution model choice
- grain decisions
- overlap clusters
- helper, specialized mart, and analyst-facing boundaries
- downstream and column-level impact
- migration, compatibility, and rollback plan

## First-principles contract (required)

Before proposing structure changes, define:
- business concept or modeling problem
- scope of candidates
- expected analyst-facing grain
- repeated semantic or column families
- blast-radius and compatibility concerns
- migration objective

Do not recommend a canonical target until you can name:
- the candidate cluster
- grain differences between candidates
- repeated semantics or repeated columns
- why one candidate should be preferred
- which helpers, specialized marts, and reporting models remain legitimate
- what downstream contracts are affected
- how the migration will be validated and rolled back

## Transport selection

- Prefer MCP for candidate discovery, side-by-side comparison, and impact analysis when `mcp__nova__*` tools are available.
  - Read `references/transport-mcp.md`
- Use the CLI through `Bash` for deterministic exported artifacts and local project inspection.
  - Read `references/transport-cli.md`

Mixed transport is expected when discovery happens through MCP but exported inventories or repo-local checks need CLI.

## Deterministic flow

1. Define the repeated business concept or modeling problem.
2. Baseline the project area and candidate cluster with scoped discovery.
3. Use targeted overlap and consistency evidence to shortlist candidates.
4. Compare grains, columns, SQL shape, and semantic contracts.
5. Choose the canonical execution surface and legitimate specialized surfaces.
6. Review downstream impact before proposing migration.
7. Produce a migration-safe refactor plan with validation and rollback.

Rules:
- Canonicality is a project-shape decision, not a naming convention.
- Grain comes before semantics.
- Prefer one clear analyst-facing execution model per repeated concept and grain.
- Keep helper models useful for engineering without letting them dominate analyst discovery.
- Preserve specialized marts when they answer a different grain, audience, or performance need.
- Do not merge or retire high-impact models without an expand-contract migration plan.
- Treat overlap as a design smell until proven otherwise.

## Output standard

Every final architecture handoff must include:
- business concept or modeling problem
- candidate entities reviewed
- chosen canonical target
- grain rationale
- helper, specialized, or reporting entities to retain
- overlap or duplication evidence
- downstream impact and rollback notes
- validation plan

Default output behavior:
- Do not include raw tool dumps by default.
- Prefer a concise refactor plan with explicit evidence.

## Load order

- Read `references/workflow.md` first.
- Read the relevant transport file(s):
  - `references/transport-mcp.md`
  - `references/transport-cli.md`
- Load the decision references only when needed.
- Load `assets/refactor-plan-template.md` only when preparing the final handoff.
