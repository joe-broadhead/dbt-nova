# Metadata Authoring Workflow

Use this workflow when authoring or reviewing `meta.nova` so the final result is valid, deliberate, and discoverable through Nova.

## Deterministic sequence

1. Classify the entity before editing.
2. Inventory the existing semantic and structural definitions when the concept is repeated.
3. Choose the correct Nova surface for the intent.
4. Validate the smallest possible scope first when a schema validator is available.
5. Treat schema and local semantic failures as blockers.
6. Refresh the manifest after compile/build.
7. Verify authored behavior through search and compact contract checks.
8. Widen scope only after the narrow target is clean.

## Classification rule

Decide whether the entity is:
- canonical analyst-facing dataset
- helper / ops / intermediate model
- metric template model
- source needing discovery or governance hints
- column needing semantic disambiguation

If the classification is wrong, the metadata is usually wrong.

## Repeated-concept rule

When the business concept already appears across multiple models or columns:
- use `indicator_inventory` to inspect existing measures and metrics
- use `search_columns` or `column_inventory` to inspect column-level semantics
- use `find_entity_overlap` or `compare_grains` when canonical placement is unclear

Do not add a new canonical definition until you understand the existing repeated surface.

## Surface selection rule

Use:
- entity-level metadata for stable routing and discovery
- `measures` when reusable aggregations belong to the execution model
- `metric` / `metrics` for reusable KPI templates
- column-level metadata only when the column needs real semantic help
- `search.candidates` only for genuine audience exceptions

Read the shared decision rules and patterns before editing repeated business terms or canonical definitions.

## Validation rule

When the transport supports `audit nova-meta`, start with the narrowest possible target:
- one file
- one resource
- one column

Do not continue to search validation until schema and local semantic validation are clean.

## Search verification rule

After validation and manifest refresh, verify authored behavior through:
- `search_indicator` for measures and metrics
- `search` for broader entity discovery
- `get_entity detail=standard` for the compact semantic contract
- `get_columns` for referenced field checks
- `get_metadata_score` for documentation and metadata quality impact

Use the shared review checklist before considering the work complete.
