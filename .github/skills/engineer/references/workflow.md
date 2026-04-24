# Engineer Workflow

Use this workflow for dbt changes that must ship with explicit contract, blast-radius, and readiness checks.

## Decompose the change first (required)

Extract:
- change type
- target artifact or search area
- expected contract after the change
- grain and primary key expectations
- upstream requirements
- downstream risk
- validation plan

If the requested outcome is materially ambiguous, ask one clarification question before implementation.

## Change classification

Classify the work as one of:
- local SQL fix
- contract extension
- semantic definition change
- grain change
- new model
- metadata or test hardening
- refactor without intended contract change

The stricter the contract impact, the stronger the required blast-radius and validation evidence.

## Deterministic sequence

1. Discover whether the project already has the right place for the change.
2. Prefer reuse or extension before adding a new model.
3. Inspect the current contract on the candidate entity.
4. Validate required upstream inputs before editing.
5. Measure blast radius before changing shared logic or grain.
6. Implement the smallest viable change.
7. Re-check contract, lineage, and grain after the edit.
8. Run quality gates proportional to the risk.
9. Refresh or reload the manifest after compile/build changes.
10. Produce a ship summary with explicit residual risk.

## Reuse-before-add rule

Prefer:
- direct reuse of an existing model
- extension of an existing canonical execution model
- adding a new model only when the current project shape cannot support the use case cleanly

Shortlist aggressively:
- keep at most 2-3 plausible candidates
- usually one canonical upstream candidate and one downstream presentation candidate are enough
- stop searching once the reuse-versus-add decision is evidenced

If you add a new model, explain why extension or reuse was not sufficient.

## Layer-placement rule

Choose the narrowest correct layer for the change:
- canonical or base model when the business definition, semantic contract, or reusable KPI surface must change
- downstream reporting model when the request is layout-, audience-, or presentation-specific

Do not move presentation logic upstream into a high-blast-radius canonical model unless the semantic contract itself must change.
Do not create a new middle-layer model when an existing downstream model can be extended cleanly.

## Contract check rule

Before and after implementation, confirm:
- grain
- primary keys
- key dimensions and measures
- relation or materialization target
- downstream-facing column names

Do not treat a model as “unchanged” if any of those moved materially.

## Blast-radius rule

Do not skip blast-radius analysis for:
- grain changes
- renamed or removed columns
- semantic definition changes
- changes to shared execution models
- refactors that alter joins or filter logic

Use:
- impact analysis for downstream scope
- lineage for data-flow confirmation
- column lineage for critical fields
- grain comparison when replacing or refactoring execution entities
- diffing when comparing old and new targets

Do not over-prove low-risk questions:
- if the task is only placement or reuse-versus-add, stop after contract plus one blast-radius check
- if the task is only quality hardening, prioritize test coverage and metadata score before deeper lineage work
- reserve full context or column-lineage dives for high-risk contract changes

## Quality gates

Choose gates proportional to the change:
- docs on key columns and measures
- tests for primary keys, not-null, and relationships where appropriate
- metadata completeness for shared models
- DAG validation after meaningful topology changes

If schema YAML or `meta.nova` changes, run `audit nova-meta` when the local transport supports it.

## Manifest readiness rule

If compile/build changes the manifest, do not trust stale discovery results.

After local compile/build:
- refresh or reload the manifest
- check readiness
- only then trust follow-up search, scoring, or validation

Do not front-load manifest lifecycle work on a shared hosted MCP endpoint when the task is only discovery, design, or risk assessment.

## Output requirement

Every ship summary should include:
- target model and change type
- resulting grain and primary key(s)
- change summary
- downstream impact
- tests added or still missing
- metadata score or doc gaps when relevant
- manifest readiness after refresh, when the task actually changed the manifest
- rollout notes or remaining risks

When the answer is MCP-only discovery, design, or risk assessment against a hosted manifest:
- cite Nova entity ids and relation names
- do not cite local file paths unless you have verified the local checkout is the same dbt project
