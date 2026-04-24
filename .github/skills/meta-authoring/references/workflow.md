# Metadata Authoring Workflow

Use this sequence for both authoring and review. The main failure mode is adding plausible YAML before proving where the business concept belongs.

## Sequence

1. Confirm the loaded project, manifest age, and resource scope.
2. Classify the target entity.
3. Inventory repeated concepts before adding new canonical metadata.
4. Choose the smallest correct Nova surface.
5. Validate local edits with schema/audit tooling when available.
6. Rebuild or refresh the manifest through the project workflow.
7. Verify search, grain, and score behavior against the refreshed manifest.
8. Widen scope only after the narrow target is clean.

## Classification

Classify the target as exactly one primary case:
- canonical analyst-facing dataset
- helper, ops, staging, or intermediate resource
- metric template model
- source needing sparse routing or governance hints
- column needing semantic disambiguation

If classification is ambiguous, inspect context and lineage before editing. Do not make a helper canonical just because it is upstream, and do not de-rank a resource that is the actual analyst entry point.

## Repeated-Concept Evidence

For business terms such as GMV, AOV, sessions, orders, customers, margin, conversion, or product counts:
- search indicators by term and synonym
- inventory canonical indicators
- inspect candidate entities and grains
- compare grains when two definitions look equivalent
- use overlap or consistency reports when the concept appears broadly

Only add a new canonical definition when there is no better existing owner, or when the new owner clearly supersedes the existing one and cleanup is planned.

## Surface Selection

Use:
- entity-level metadata for dataset routing, grain, and governance
- `measures` for reusable aggregations on the execution dataset
- `metric` / `metrics` for reusable KPI templates
- column metadata for identifiers, time, high-signal dimensions, and ambiguous business fields
- `search.candidates` for audience-specific de-ranking only

Prefer fewer high-signal fields over exhaustive metadata.

## Verification

After local validation and manifest refresh, verify:
- `search_indicator` for authored measures or metrics
- `search` for entity discovery and ranking
- `get_entity` or `get_context` for compact contract checks
- `get_columns` for referenced field existence
- `get_metadata_score` for quality impact

If the hosted MCP manifest is older than the local build, report that search verification is pending deployment rather than pretending it passed.
