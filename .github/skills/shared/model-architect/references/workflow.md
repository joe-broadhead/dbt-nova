# Model Architect Workflow

Use this workflow when improving the structure of a dbt project itself: choosing canonical execution models, reducing overlap, clarifying grains, and tightening semantic-layer boundaries.

## Deterministic sequence

1. Define the business concept or modeling problem to be cleaned up.
2. Inventory the candidate entities in scope.
3. Compare grains, dimensions, and semantic definitions across those candidates.
4. Identify the canonical execution model and the helper / intermediate models around it.
5. Document modelling anti-patterns and cleanup risks.
6. Produce a refactor plan with migration steps, validation, and rollback notes.

## Core rules

- Canonicality is a project-shape decision, not a naming convention.
- Grain comes before semantics: a canonical model with the wrong grain is not canonical.
- Prefer one clear analyst-facing execution model per repeated business concept.
- Keep helper models useful for engineering without letting them dominate discovery.
- Treat overlap as a design smell until proven otherwise.

## Output requirement

Use the shared refactor-plan template when handing off architecture work:
- current-state overlap
- canonical target
- migration steps
- impact / rollback
- validation plan
