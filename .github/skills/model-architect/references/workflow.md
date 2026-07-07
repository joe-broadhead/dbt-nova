# Model Architect Workflow

Use this workflow when improving the structure of a dbt project: choosing canonical execution models, reducing overlap, clarifying grains, and tightening semantic-layer boundaries.

## Deterministic sequence

1. Define the business concept or modeling problem to be cleaned up.
2. Baseline the project area and candidate entities in scope.
3. Use targeted overlap and repeated-concept tools before broad project reports.
4. Compare grains, dimensions, columns, SQL shape, and semantic definitions across shortlisted candidates.
5. Identify the canonical execution model and any legitimate helper, specialized mart, or reporting surfaces.
6. Review downstream, column-level, and metadata-score impact.
7. Produce a refactor plan with migration steps, validation, and rollback notes.

## Core rules

- Canonicality is a project-shape decision, not a naming convention.
- Grain comes before semantics: a canonical model with the wrong grain is not canonical.
- Prefer one clear analyst-facing execution model per repeated business concept and grain.
- Keep helper models useful for engineering without letting them dominate discovery.
- Preserve specialized marts when they have distinct grain, performance, or business-scope reasons.
- Treat high downstream impact as a migration constraint, not a reason to avoid cleanup.
- Treat overlap as a design smell until proven otherwise.

Use:
- `show_metadata`, `search`, `find_by_path`, and `list_entities` for the first baseline
- `search_indicator`, `indicator_inventory`, `search_columns`, and `column_inventory` for repeated semantic and column surfaces
- `find_entity_overlap` for targeted overlap around a focus entity
- `modelling_consistency_report` when project-wide duplicate/canonical drift,
  grain drift, or agent-modelling blocker/high findings are needed
- `compare_grains` and `diff_entities` for shortlisted candidate pairs
- `get_lineage`, `get_impact`, and `get_column_lineage` for migration blast radius
- `get_metadata_score` to check whether the proposed canonical target has enough documentation, tests, and semantics
- helper scripts in `scripts/` when you need exported inventories or overlap artifacts

## Output requirement

Use the refactor-plan template when handing off architecture work:
- current-state overlap
- canonical target
- helper and specialized surfaces to retain
- migration steps
- impact / rollback
- validation plan
