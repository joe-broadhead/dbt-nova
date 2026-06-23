# Evidence Sources

Prefer the review packet supplied by the primary agent. Use read-only Nova
context only when the packet is incomplete and the client can safely gather more
evidence.

## MCP Evidence

Useful read-only MCP tools:

- `search_indicator` for governed metrics and measures
- `search` only after semantic evidence is absent or explicitly insufficient
- `get_entity` or `get_context` for the selected entity contract
- `get_lineage` or `get_column_lineage` for upstream provenance when relevant
- `get_metadata_score` and `get_test_coverage` for readiness caveats
- `search_recipes` and `get_recipe` for recurring deliverables

Do not call `execute_sql`, `run_recipe`, manifest lifecycle tools, storage admin
tools, or eval write/run tools from a reviewer workflow.

## CLI Evidence

When MCP tools are unavailable and CLI access is explicitly allowed, use
read-only `dbt-nova tool call` commands only. Examples:

```bash
dbt-nova tool call search_indicator \
  --params-json '{"query":"gross revenue","detail":"compact","group_mode":"top","limit":3}' \
  --json
```

```bash
dbt-nova tool call get_context \
  --params-json '{"id_or_name":"model.pkg.orders","detail":"standard"}' \
  --json
```

For review, CLI output is evidence only if it includes the relevant identity,
semantic contract, provenance tier, and freshness status. If a CLI call cannot
return those fields, ask for the missing evidence instead of guessing.
