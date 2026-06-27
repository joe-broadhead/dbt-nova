# Governance Transport: MCP

Use this reference when the client exposes `mcp__nova__*` tools directly.

## Session contract

- Use `show_metadata` first to capture manifest identity and project scope.
- Use MCP for scope inventory, metadata scoring, test-coverage review, and entity-level blocker extraction.
- Use `health` only when readiness is uncertain or a prior tool suggests startup/cache issues.
- If `health` reports `ready_for_traffic=false` or a tool returns
  `INDEX_BUILDING`, wait for readiness before freezing audit scope or scoring
  evidence.
- Use CLI separately for `audit nova-meta` or other local-only validation.
- Do not reload or mutate shared hosted MCP servers from a governance audit.
- Do not call `warm_manifest` from governance audits unless semantic cache
  readiness is explicitly in scope.

## Practical order

1. `show_metadata` for manifest identity
2. `find_by_path`, `list_tags`, `list_packages`, or bounded `list_entities` to freeze scope
3. `get_metadata_score` for baseline scoring
4. `get_test_coverage`, `get_entity`, and `get_columns` for blocker detail
5. `search_columns` or `column_inventory` for PII/compliance and repeated-field audits
6. `batch_get_entities` for compact review of a small failing set
7. `search` only as a triage helper when the scope definition is still unclear

Scope discipline:
- for exact entity audits, skip inventory and score the entity directly
- for path audits, use `find_by_path` before `list_entities`
- for small path/tag scopes, call `get_metadata_score` on each frozen entity; `scope=project` pages are project baselines, not path-scoped gates
- for project baselines, use bounded pages and report the page/scope limit
- keep the rerun scope identical to the baseline scope

Compliance scan pattern:
- use `search_columns` for likely PII terms such as email, phone, address, customer, member, national identifier
- inspect the parent with `get_metadata_score persona=governance`
- use `get_columns` only when you need to verify whether sensitive-looking fields are present in the selected entity
