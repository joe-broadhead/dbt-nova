# Provider-Backed Agent Eval Patterns

## What Agent Evals Should Prove

Agent evals should test behavior, not every output detail:
- discovery tools are called before context or execution
- the correct entity appears in tool evidence
- forbidden tools are avoided for non-execution tasks
- safe parameter summaries contain the intended business term
- the final answer contains short durable terms

Agent evals score sanitized tool traces. `called_with.params` can check exact
safe scalar or scalar-array values such as `query`, `id_or_name`, `persona`,
`resource_types`, `recipe_id`, `direction`, `limit`, and `offset`.
`called_with.contains` checks text inside those sanitized fields. For
`execute_sql`, nested `parameters` and raw SQL are intentionally not exposed;
check presence through the `keys` summary instead of asserting query text.

## Useful Expectations

```yaml
agent_cases:
  - id: metric_lookup_flow
    task: Which canonical model and indicator should be used to analyze gross merchandise value?
    expected:
      must_call:
        - search_indicator
        - get_context
      must_not_call:
        - execute_sql
      ordered:
        - before: get_context
          must_have_called:
            - search_indicator
      selected_entities:
        - model.pkg.orders
      selected_entity_ranks:
        - unique_id: model.pkg.orders
          tool: search_indicator
          max_rank: 3
      called_with:
        - tool: search_indicator
          contains:
            query: gross merchandise
      final_answer:
        must_contain:
          - gross merchandise value
```

For execution workflows, assert sanitized SQL parameter presence without
capturing raw SQL:

```yaml
called_with:
  - tool: execute_sql
    contains:
      keys: statement
  - tool: execute_sql
    contains:
      keys: parameters
  - tool: execute_sql
    contains:
      keys: row_limit
```

## Provider Runs

Default provider:

```bash
dbt-nova eval agent run \
  --suite evals/analyst-agent.yml \
  --provider opencode \
  --manifest-path target/manifest.json \
  --case-id metric_lookup_flow \
  --fail-under 1.0
```

Custom provider:

```bash
dbt-nova eval agent run \
  --suite evals/analyst-agent.yml \
  --provider custom \
  --provider-command ./scripts/run-agent-eval.sh \
  --provider-args-json '["--prompt","{prompt}","--trace","{trace_path}","--manifest","{manifest_path}"]' \
  --manifest-path target/manifest.json
```

## Debugging Agent Failures

- If `tool_trace_missing` appears, confirm the provider launches a local dbt-nova process or emits supported JSON MCP events.
- If the wrong MCP server alias is used, set `DBT_NOVA_EVAL_MCP_SERVER_ALIASES`.
- If `selected_entities` fails but `must_call` passes, inspect the tool result shape and prefer `selected_entity_ranks` scoped to the discovery tool.
- If final-answer checks are brittle, remove broad prose expectations and keep only durable business terms.
