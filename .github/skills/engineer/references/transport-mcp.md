# Engineer Transport: MCP

Use this reference when the client exposes `mcp__nova__*` tools directly.

## Session contract

- Use `show_metadata` first for a fast scope check when you need project identity or manifest scope.
- Use MCP first for discovery, inspection, impact, and quality checks.
- Use `health` only when readiness is uncertain or a prior tool suggests a startup/cache issue.
- If `health` reports `ready_for_traffic=false` or a tool returns
  `INDEX_BUILDING`, wait for readiness before continuing tool-based evidence
  gathering.
- If the change requires local compile/build or `audit nova-meta`, switch to CLI for that part instead of forcing MCP to do it.
- Do not call `warm_manifest` on shared or constrained environments unless the
  task is explicitly about semantic cache lifecycle validation.

## Discovery order

1. `search`, `find_by_path`, `list_entities` for target discovery
2. `search_columns`, `column_inventory`, `indicator_inventory` only when you need repeated field or semantic corroboration
3. `get_entity`, `get_columns`, `get_sql`, `get_context` for contract inspection
4. `get_impact`, `get_lineage`, `get_column_lineage`, `compare_grains`, `diff_entities` for blast-radius analysis
5. `get_test_coverage`, `get_metadata_score`, `validate_dag` for quality gates

Candidate discipline:
- keep the search shortlist to 2-3 candidates
- once one canonical upstream candidate and one downstream candidate are enough to decide placement, stop searching
- do not reopen generic `search` after `get_entity` plus `get_impact` already made the decision clear

## Engineering guidance

- Use compact entity inspection before making assumptions about grain or keys.
- Use context or lineage only when the change risk justifies it.
- Treat warehouse or provider failures as execution blockers, not as proof the design is wrong.
- Do not reload the manifest on shared hosted servers unless the task is explicitly about that manifest lifecycle.
- For quality-hardening questions, lead with `get_test_coverage` and `get_metadata_score`; use deeper lineage only if prioritization remains ambiguous.
- For hosted-manifest analysis, cite Nova entity ids and relation names rather than local file paths unless you have verified the local checkout matches the target dbt project.

## When to switch to CLI

Switch to CLI when you need:
- local compile or build
- `audit nova-meta`
- a local manifest that is newer than the hosted endpoint
- deterministic shell-based validation in the current checkout
