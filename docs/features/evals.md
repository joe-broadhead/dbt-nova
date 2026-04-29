# Nova Evals

Nova evals let teams test whether their manifest metadata works well for agents.
They are intentionally split into two layers:

- **Bridge evals** call Nova tools directly and verify deterministic search,
  context, lineage, recipe, and metadata-score behavior.
- **Agent evals** run an external agent CLI and score the tools the agent used,
  using a sanitized tool-call trace emitted by dbt-nova.

Use bridge evals to prove the metadata bridge is healthy. Use agent evals to
prove analyst or engineering agents actually discover and apply the right Nova
tools for realistic tasks.

## Create a Suite

Generate a starter suite:

```bash
dbt-nova eval init --persona analyst --out evals/analyst-smoke.yml
```

The suite format is YAML or JSON. A minimal bridge suite looks like this:

```yaml
version: 1
name: analyst-smoke
defaults:
  persona: analyst
  top_k: 5
cases:
  - id: canonical_orders_search
    question: Find the canonical orders model.
    assertions:
      - type: search_rank
        query: orders
        expected_unique_id: model.pkg.orders
        max_rank: 5
      - type: context_has
        id_or_name: model.pkg.orders
        fields:
          - data.unique_id
          - data.name
```

## Run Bridge Evals

```bash
dbt-nova eval run \
  --suite evals/analyst-smoke.yml \
  --manifest-path /path/to/target/manifest.json \
  --fail-under 1.0 \
  --json
```

Outputs are written to `.nova/eval-runs/<timestamp>-<suite>-bridge/` by default:

- `results.json` for machine-readable CI output.
- `results.tsv` for spreadsheet review.
- `report.md` for human review.
- `suite.yml` as the exact suite copy used for the run.

Bridge assertions currently support:

- `search_rank`
- `search_indicator_rank`
- `search_columns_rank`
- `context_has`
- `metadata_score_min`
- `recipe_rank`
- `recipe_has_queries`
- `lineage_contains`
- `tool_success`

## Run Agent Evals

Agent evals execute a provider CLI and score observed Nova tool calls. The
provider must already be configured to use dbt-nova as an MCP server or local
CLI-backed tool source. dbt-nova first reads sanitized JSONL rows written to
`DBT_NOVA_TRACE_TOOL_CALLS_PATH`; if that trace is empty, it falls back to
provider JSON event streams for MCP tool calls from supported presets such as
Codex, Claude, and OpenCode.

```yaml
version: 1
name: analyst-agent-smoke
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
      final_answer:
        must_contain:
          - gross merchandise value
```

Run against the default `opencode` adapter:

```bash
dbt-nova eval agent run \
  --suite evals/analyst-agent.yml \
  --provider opencode \
  --manifest-path /path/to/target/manifest.json \
  --timeout-secs 600 \
  --fail-under 0.9
```

Supported provider presets are `opencode`, `codex`, `claude`, and `goose`.
Presets use each CLI's normal project/user configuration; dbt-nova injects
`DBT_MANIFEST_PATH`, `DBT_NOVA_STORAGE_INSTANCE_ID`, and
`DBT_NOVA_TRACE_TOOL_CALLS_PATH` into the provider process so local MCP/CLI
tool servers inherit the eval manifest and trace path. Remote hosted MCP
endpoints cannot inherit local trace environment variables, so use a provider
that emits MCP calls in its JSON output or keep the Nova server local when
asserting tool-use traces.

For another provider or a custom local wrapper, pass an explicit command:

```bash
dbt-nova eval agent run \
  --suite evals/analyst-agent.yml \
  --provider custom \
  --provider-command ./scripts/run-agent-eval.sh \
  --provider-args-json '["--prompt","{prompt}","--workdir","{workdir}","--trace","{trace_path}"]' \
  --manifest-path /path/to/target/manifest.json
```

Available placeholders in `--provider-args-json`:

- `{prompt}`
- `{workdir}`
- `{trace_path}`
- `{manifest_path}`

## Tool Trace

When `DBT_NOVA_TRACE_TOOL_CALLS_PATH` is set, dbt-nova appends sanitized JSONL
rows for CLI, MCP, and eval tool calls. Rows include:

- `transport`
- `tool`
- `success`
- `duration_ms`
- safe parameter summaries such as `query`, `persona`, `id_or_name`, and
  `resource_types`
- `selected_unique_ids` extracted from the response

Trace rows deliberately do not record SQL recipe parameter maps or credential
values. Keep eval artifacts out of public bug reports unless you have reviewed
them for project-specific identifiers.

## CI Pattern

Use bridge evals as a fast metadata quality gate after producing a manifest:

```bash
dbt-nova eval run \
  --suite evals/analyst-smoke.yml \
  --manifest-path target/manifest.json \
  --output-dir out/nova-evals \
  --fail-under 0.95 \
  --json
```

Run agent evals on a schedule, before major metadata changes, or when updating
agent skills. They are intentionally slower and depend on the configured agent
provider.
