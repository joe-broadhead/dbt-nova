# Model Architect Workflow

Use this workflow when improving the structure of a dbt project itself: choosing canonical execution models, reducing overlap, clarifying grains, and tightening semantic-layer boundaries.

## Deterministic sequence

1. Define the business concept or modeling problem to be cleaned up.
2. Run a broad consistency baseline when the scope is project-wide.
3. Inventory the candidate entities in scope.
4. Compare grains, dimensions, columns, and semantic definitions across those candidates.
5. Identify the canonical execution model and the helper / intermediate models around it.
6. Document modelling anti-patterns and cleanup risks.
7. Produce a refactor plan with migration steps, validation, and rollback notes.

## Core rules

- Canonicality is a project-shape decision, not a naming convention.
- Grain comes before semantics: a canonical model with the wrong grain is not canonical.
- Prefer one clear analyst-facing execution model per repeated business concept.
- Keep helper models useful for engineering without letting them dominate discovery.
- Treat overlap as a design smell until proven otherwise.

Use:
- `modelling_consistency_report` for the first project-wide baseline
- `find_entity_overlap` to form overlap clusters
- `compare_grains` for shortlisted candidate pairs
- `indicator_inventory`, `column_inventory`, and `search_columns` to inspect repeated semantic and column surfaces

## Output requirement

Use the shared refactor-plan template when handing off architecture work:
- current-state overlap
- canonical target
- migration steps
- impact / rollback
- validation plan

When terminal access is available, prefer the helper scripts in `scripts/` for deterministic current-state artifacts:
- `python3 scripts/export_entity_inventory.py ...`
- `python3 scripts/export_column_inventory.py ...`
- `python3 scripts/build_overlap_report.py ...`
