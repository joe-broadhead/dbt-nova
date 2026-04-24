# Project Cleanup Workflow

Use this workflow when a dbt project has overlapping entities, inconsistent naming, repeated semantics, or unclear canonical execution models.

## Deterministic sequence

1. Define the cleanup scope.
2. Baseline the candidate entities in that scope.
3. Identify overlap clusters, repeated concepts, duplicate indicators, and repeated column families.
4. Classify each cluster: accidental duplication, canonical conflict, legitimate specialization, staging/source partitioning, reporting derivation, or naming drift.
5. Compare grains, columns, and semantic contracts across shortlisted clusters.
6. Separate canonical targets from helper, specialized, staging, or reporting entities.
7. Quantify downstream and column-level risk.
8. Document inconsistencies, non-actions, and cleanup priorities.
9. Produce an overlap audit and cleanup queue.

## Core rules

- Cleanup work is structural, not cosmetic.
- Repeated concepts are a design problem until a clear canonical target is justified.
- Column naming inconsistencies matter when they force downstream translation logic.
- Prefer a small number of clear analyst-facing execution models over many near-peers.
- Cleanup priority is a function of semantic risk, discovery impact, downstream blast radius, and migration complexity.
- High-overlap low-impact staging families are usually queued behind high-impact semantic ambiguity.
- Every cleanup recommendation should be backed by evidence, not taste.

Use:
- `show_metadata`, `search`, `find_by_path`, and `list_entities` for the first baseline
- `find_entity_overlap` for focused overlap clusters
- `modelling_consistency_report` for duplicate indicators, canonical conflicts, and project-wide grain drift
- `search_indicator`, `indicator_inventory`, `search_columns`, and `column_inventory` to inspect repeated terms and columns
- `compare_grains` and `diff_entities` for narrowed candidate pairs
- `get_lineage`, `get_impact`, and `get_column_lineage` for downstream risk
- `get_metadata_score` when weak metadata contributes to cleanup priority
- helper scripts in `scripts/` when you need exported inventories or overlap artifacts

## Output requirement

Use the overlap-audit template when handing off cleanup work:
- overlap clusters
- inconsistent patterns
- canonical candidates
- cleanup queue with priority
- explicit non-actions and rationale
