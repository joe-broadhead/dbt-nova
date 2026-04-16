# Nova Meta Authoring Workflow

Use this workflow when authoring or reviewing `meta.nova` so the final result is valid, deliberate, and discoverable through Nova.

## Deterministic sequence

1. Classify the entity before editing.
2. Choose the correct Nova surface for the intent.
3. Validate the smallest possible scope first when a schema validator is available.
4. Treat schema and local semantic failures as blockers.
5. Refresh the manifest after compile/build.
6. Verify authored behavior through search and compact contract checks.
7. Widen scope only after the narrow target is clean.

## Classification rule

Decide whether the entity is:
- canonical analyst-facing dataset
- helper / ops / intermediate model
- metric template model
- source needing discovery or governance hints
- column needing semantic disambiguation

If the classification is wrong, the metadata is usually wrong.

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
