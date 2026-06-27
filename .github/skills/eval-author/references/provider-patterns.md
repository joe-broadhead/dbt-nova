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

## Semantic-First KPI Pattern

For metric/KPI tasks with known semantic coverage, assert that semantic
discovery happens before context or execution. Also forbid broad `search` when
the fixture has a governed indicator that should answer the question.

Definition/context task:

```yaml
agent_cases:
  - id: semantic_metric_context_flow
    task: Which governed model and metric should be used for gross merchandise value? Do not execute SQL.
    expected:
      must_call:
        - search_indicator
        - get_context
      must_not_call:
        - search
        - execute_sql
      ordered:
        - before: get_context
          must_have_called:
            - search_indicator
      selected_entity_ranks:
        - unique_id: model.pkg.orders
          tool: search_indicator
          max_rank: 3
```

Execution task:

```yaml
agent_cases:
  - id: semantic_metric_execution_flow
    task: Compute gross merchandise value for March 2026 from the governed metric.
    expected:
      must_call:
        - search_indicator
        - execute_sql
      must_not_call:
        - search
        - get_sql
      ordered:
        - before: execute_sql
          must_have_called:
            - search_indicator
      called_with:
        - tool: search_indicator
          contains:
            query: gross merchandise
```

Fallback cases should be separate. They should still require a
`search_indicator` attempt, then allow broad `search` only when the prompt and
expected final answer capture why no governed indicator or semantic parent was
usable.

## Reviewer Agent Eval Pattern

Use reviewer agent evals when the behavior under test is adversarial review of
a draft answer, not fresh analysis. Set `defaults.persona: reviewer` so the
provider prompt uses the reviewer contract. Give the task a self-contained
review packet with the user question, draft answer, selected entity/source,
semantic discovery evidence, provenance/freshness blocks, and SQL or recipe
summary when available.

Keep reviewer eval assertions durable. Prefer final-answer verdict terms over
tool-trace expectations unless the provider reliably emits trace rows for
no-tool reviews.

Semantic-layer bypass case:

```yaml
version: 1
name: reviewer-smoke
defaults:
  persona: reviewer
agent_cases:
  - id: flags_semantic_layer_bypass
    task: |
      Review packet:
      - user question: What was gross revenue last week?
      - governed semantic evidence: search_indicator returned measure
        gross_revenue on model.pkg.orders with provenance.tier semantic_layer.
      - draft route: draft answer used source.pkg.raw_orders as primary
        evidence and gave no fallback reason.
      - draft answer: Gross revenue was 42,000 from raw_orders.
      Return the reviewer output contract.
    expected:
      final_answer:
        must_contain:
          - fix_required
          - semantic-layer bypass
          - gross_revenue
          - model.pkg.orders
```

Stale or unknown freshness case:

```yaml
agent_cases:
  - id: flags_unknown_freshness_without_caveat
    task: |
      Review packet:
      - user question: What was gross revenue last week?
      - selected entity: model.pkg.orders
      - provenance.freshness.status: unknown
      - provenance.freshness.reason: no_freshness_timestamp
      - draft answer: Gross revenue was 42,000.
      Return the reviewer output contract.
    expected:
      final_answer:
        must_contain:
          - fix_required
          - freshness
          - unknown
          - caveat
```

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
        - search
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

Default provider presets:

| Provider | Default command shape |
| --- | --- |
| `opencode` | `opencode run --format json <prompt>` |
| `codex` | `codex exec --json --cd <workdir> <prompt>` |
| `claude` | `claude -p --verbose --output-format stream-json <prompt>` |
| `goose` | `goose run --text <prompt> --output-format stream-json --no-session` |

Default provider run:

```bash
dbt-nova eval agent run \
  --suite evals/analyst-agent.yml \
  --provider opencode \
  --manifest-path target/manifest.json \
  --case-id metric_lookup_flow \
  --fail-under 1.0
```

For a trusted local hardening run where the agent CLIs are already configured
and noninteractive execution is acceptable, use explicit custom provider args:

```bash
dbt-nova eval agent run \
  --suite evals/analyst-agent.yml \
  --provider custom \
  --provider-command opencode \
  --provider-args-json '["run","--format","json","--dangerously-skip-permissions","{prompt}"]' \
  --manifest-path target/manifest.json \
  --telemetry \
  --fail-under 1.0

dbt-nova eval agent run \
  --suite evals/analyst-agent.yml \
  --provider custom \
  --provider-command claude \
  --provider-args-json '["-p","--dangerously-skip-permissions","--verbose","--output-format","stream-json","{prompt}"]' \
  --manifest-path target/manifest.json \
  --telemetry \
  --fail-under 1.0

dbt-nova eval agent run \
  --suite evals/analyst-agent.yml \
  --provider custom \
  --provider-command codex \
  --provider-args-json '["exec","--json","--dangerously-bypass-approvals-and-sandbox","--skip-git-repo-check","--cd","{workdir}","{prompt}"]' \
  --manifest-path target/manifest.json \
  --telemetry \
  --fail-under 1.0
```

Use bypass flags only on trusted local machines or reviewed private runners.
Keep them out of public default CI.

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
- If a provider chooses a per-unit metric when the task asks for a total
  business amount, tighten the task wording and assert the intended indicator in
  `selected_entity_ranks` or final-answer durable terms.
- If a gate reports missing telemetry, rerun the full suite with `--telemetry`;
  filtered `--case-id` runs do not satisfy launch-readiness gates.
