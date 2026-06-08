# Agent Tokenomics And Search Quality Plan

## Purpose

Nova should help an agent move from an analytics question to a correct,
auditable answer with the fewest practical tool calls and the smallest practical
tool payloads. The goal is not short responses by themselves. The goal is faster
convergence: high-signal discovery, enough semantic contract to choose one next
step, bounded validation, bounded execution, and measurable regressions.

This plan intentionally improves existing tools instead of adding a planner or
question-answering tool. More tools increase MCP schema context and make weaker
models choose from a larger surface area.

## Product Principles

- Prefer high-signal discovery over broad context loading.
- Keep compact contracts available on existing tools.
- Preserve `standard` and `full` detail for debugging and human workflows.
- Use central MCP budgeting only as a backstop.
- Measure quality and cost together; smaller payloads that cause extra calls are
  not a win.

## Implemented Scope

### Pagination Semantics

`PaginationParams.limit` now distinguishes omitted from explicit values:

- omitted: use the configured default limit for the active transport/profile
- `0`: use the configured default limit for backward compatibility
- positive value: use the caller value capped by the active max page size

This lets config defaults apply to agents that omit `limit`, while preserving
existing explicit limit behavior.

### Compact Detail

`detail: compact` is available on discovery and entity tools that already expose
`detail`:

- `search`
- `search_indicator`
- `get_entity`
- `batch_get_entities`
- `list_entities`
- `find_by_path`
- `get_lineage`

Compact entity summaries include identity, relation, primary key, grain,
domains, canonical status, and capped metric/measure names. They avoid long
descriptions, SQL, docs, lineage, tests, and full column metadata.

### Indicator Parent-Group Controls

`search_indicator` now supports:

```json
{
  "detail": "compact",
  "group_mode": "top",
  "max_parent_groups": 1,
  "include_support_signals": true
}
```

`group_mode` semantics:

- `none`: omit `parent_groups`
- `top`: include the best parent group only
- `all`: include all parent groups, capped by `max_parent_groups` when present

The CLI/default API path remains `all` for compatibility with existing callers.
The default MCP compact profile fills omitted `group_mode` as `top`; explicit
`group_mode: all` remains available for richer diagnostics. Token-sensitive
agent prompts and examples still request `top` explicitly for portability. Keep
`include_support_signals` enabled when the question includes filter values such
as country, channel, market, segment, or device labels; disable it only for pure
definition lookups where the top rows already contain enough evidence.

### Result Profiles

Result profile configuration:

- `DBT_NOVA_RESULT_PROFILE` (default `standard`)
- `DBT_NOVA_MCP_RESULT_PROFILE` (default `compact`)
- `DBT_NOVA_MCP_DEFAULT_LIMIT` (default `10`)
- `DBT_NOVA_MCP_MAX_PAGE_SIZE` (default `100`, `0` disables the MCP-specific cap)

Profiles fill omitted `detail` values only. Explicit `detail: standard` and
`detail: full` continue to work under the compact MCP profile.

### MCP Response Budget

MCP responses pass through a deterministic budget backstop before serialization.
Configuration:

- `DBT_NOVA_MCP_MAX_RESPONSE_BYTES` (default `65536`, `0` disables)
- `DBT_NOVA_MCP_MAX_STRING_CHARS` (default `4096`)
- `DBT_NOVA_MCP_INCLUDE_TRUNCATION_META` (default `true`)

Under-budget responses are returned unchanged. Over-budget responses truncate
long strings and large arrays in deterministic passes. When metadata is enabled,
truncated responses include `_nova_result_meta` with response bytes, budget
bytes, omitted paths, original count when known, and `next_offset` for paginated
MCP responses when another page exists.

### Trace And Eval Budgets

Sanitized trace rows now include:

- `response_bytes`
- `response_truncated`
- `result_count`
- `total_available`
- `tool_call_index`

Bridge evals can use `tool_response_budget`. Agent evals can assert:

- `max_tool_calls`
- `max_distinct_tools`
- `max_total_response_bytes`
- `max_response_bytes_by_tool`

OpenCode agent evals support `--provider-model`, for example:

```bash
dbt-nova eval agent run \
  --suite evals/agent-tokenomics-opencode.yml \
  --provider opencode \
  --provider-model opencode/deepseek-v4-flash-free \
  --manifest-path tests/fixtures/tokenomics_manifest.json
```

## Deterministic Eval Harness

The tokenomics suite uses a synthetic manifest and generated DuckDB fixture:

- manifest: `tests/fixtures/tokenomics_manifest.json`
- generator: `cargo test --locked --test tokenomics_fixture -- --ignored`
- generated DB: `tests/fixtures/tokenomics.duckdb`

The fixture models UK digital sessions with exact expected values:

| Metric | Current 2026-05-24 to 2026-05-30 | YoY 2025-05-25 to 2025-05-31 | Growth |
| --- | ---: | ---: | ---: |
| `conversion_rate` | 12.0% | 10.0% | +2.0 pp / +20.0% |
| `checkout_completion_rate` | 60.0% | 50.0% | +10.0 pp / +20.0% |

Bridge suite:

```bash
dbt-nova eval validate --suite evals/agent-tokenomics-bridge.yml

dbt-nova eval run \
  --suite evals/agent-tokenomics-bridge.yml \
  --manifest-path tests/fixtures/tokenomics_manifest.json \
  --storage-instance-id tokenomics-bridge \
  --cleanup-storage-on-start \
  --fail-under 1.0 \
  --json
```

OpenCode DeepSeek suite:

```bash
DBT_NOVA_SQL_PROVIDER=duckdb \
DBT_NOVA_DUCKDB_PATH=tests/fixtures/tokenomics.duckdb \
DBT_NOVA_TOOL_ALLOWLIST=show_metadata,search_indicator,search,get_entity,get_columns,search_columns,execute_sql \
dbt-nova eval agent run \
  --suite evals/agent-tokenomics-opencode.yml \
  --provider opencode \
  --provider-model opencode/deepseek-v4-flash-free \
  --manifest-path tests/fixtures/tokenomics_manifest.json \
  --storage-instance-id tokenomics-opencode \
  --cleanup-storage-on-start \
  --timeout-secs 600 \
  --fail-under 1.0
```

## Recommended Analyst Agent Path

Use the existing tool catalog with a narrow allowlist for simple KPI work:

```text
show_metadata,search_indicator,search,get_entity,get_columns,search_columns,execute_sql
```

Default workflow:

1. Resolve KPIs with `search_indicator`, `limit: 3`, `detail: compact`,
   `group_mode: top`.
2. Prefer the top shared parent for requested indicators.
3. Inspect `get_entity(detail=compact)` only after choosing a parent.
4. Verify fields with `get_columns` or `search_columns` only when compact
   metadata is not enough.
5. Validate non-trivial filter values with bounded SQL.
6. Execute one aggregate query with row and byte limits.
7. Avoid `get_context`, lineage, SQL source, tests, and `detail: full` unless
   blocked or the user explicitly asks for provenance.

## Rollout

- Bridge evals are deterministic and suitable for normal CI once stable.
- OpenCode provider evals should run locally or in a private scheduled workflow
  because they depend on provider configuration.
- Byte thresholds should be tightened only after stable baseline runs across
  representative manifests.
- Compact defaults should be taught through skills and examples first; do not
  make a breaking global detail default change without a release note.
