# Project Cleanup Workflow

Use this workflow when a dbt project has overlapping entities, inconsistent naming, repeated semantics, or unclear canonical execution models.

## Deterministic sequence

1. Define the cleanup scope.
2. Inventory the candidate entities in that scope.
3. Identify overlap clusters and repeated concepts.
4. Compare grains, columns, and semantic contracts across the cluster.
5. Separate canonical targets from helper or specialized entities.
6. Document inconsistencies and cleanup priorities.
7. Produce an overlap audit and cleanup queue.

## Core rules

- Cleanup work is structural, not cosmetic.
- Repeated concepts are a design problem until a clear canonical target is justified.
- Column naming inconsistencies matter when they force downstream translation logic.
- Prefer a small number of clear analyst-facing execution models over many near-peers.
- Every cleanup recommendation should be backed by evidence, not taste.

## Output requirement

Use the shared overlap audit template when handing off cleanup work:
- overlap clusters
- inconsistent patterns
- canonical candidates
- cleanup queue with priority
