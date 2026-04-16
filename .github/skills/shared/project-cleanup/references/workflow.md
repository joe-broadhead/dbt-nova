# Project Cleanup Workflow

Use this workflow when a dbt project has overlapping entities, inconsistent naming, repeated semantics, or unclear canonical execution models.

## Deterministic sequence

1. Define the cleanup scope.
2. Run a broad consistency baseline when the scope is project-wide.
3. Inventory the candidate entities in that scope.
4. Identify overlap clusters and repeated concepts.
5. Compare grains, columns, and semantic contracts across the cluster.
6. Separate canonical targets from helper or specialized entities.
7. Document inconsistencies and cleanup priorities.
8. Produce an overlap audit and cleanup queue.

## Core rules

- Cleanup work is structural, not cosmetic.
- Repeated concepts are a design problem until a clear canonical target is justified.
- Column naming inconsistencies matter when they force downstream translation logic.
- Prefer a small number of clear analyst-facing execution models over many near-peers.
- Every cleanup recommendation should be backed by evidence, not taste.

Use:
- `modelling_consistency_report` for the first project-wide baseline
- `find_entity_overlap` to form overlap clusters
- `compare_grains` for narrowed entity pairs
- `indicator_inventory`, `column_inventory`, and `search_columns` to inspect repeated terms and columns

## Output requirement

Use the shared overlap audit template when handing off cleanup work:
- overlap clusters
- inconsistent patterns
- canonical candidates
- cleanup queue with priority

When terminal access is available, prefer the helper scripts in `scripts/` for deterministic artifact generation:
- `python3 scripts/export_entity_inventory.py ...`
- `python3 scripts/export_column_inventory.py ...`
- `python3 scripts/build_overlap_report.py ...`
