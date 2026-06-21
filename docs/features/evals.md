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

When using an agent to design or debug suites, use the packaged `eval-author`
skill. It guides agents to discover ground truth first, separate bridge evals
from provider-backed agent evals, and choose assertions that produce actionable
failure signals.

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
          - data.entity.name
```

Validate suite shape before running against a manifest or provider:

```bash
dbt-nova eval validate --suite evals/analyst-smoke.yml
```

## Run The Packaged Starter Suite

dbt-nova ships `evals/starter.yml` plus a synthetic manifest fixture at
`tests/fixtures/starter_eval_manifest.json`. The suite is intentionally small
and strict: it checks canonical model search, indicator discovery, context,
lineage, recipe discovery, metadata scoring, and one provider-backed agent
tool-use flow.

Validate the starter suite:

```bash
dbt-nova eval validate --suite evals/starter.yml
```

Run the bridge evals against the packaged synthetic manifest:

```bash
dbt-nova eval run \
  --suite evals/starter.yml \
  --manifest-path tests/fixtures/starter_eval_manifest.json \
  --fail-under 1.0
```

Run the starter agent case with a configured provider:

```bash
dbt-nova eval agent run \
  --suite evals/starter.yml \
  --provider opencode \
  --manifest-path tests/fixtures/starter_eval_manifest.json \
  --case-id analyst_revenue_lookup_flow \
  --fail-under 1.0
```

## Run Bridge Evals

```bash
dbt-nova eval run \
  --suite evals/analyst-smoke.yml \
  --manifest-path /path/to/target/manifest.json \
  --fail-under 1.0 \
  --json
```

Use repeatable `--case-id <ID>` to run only specific bridge cases while
debugging a suite.

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
- `context_field_equals`
- `context_contains`
- `metadata_score_min`
- `metadata_score_max`
- `recipe_rank`
- `recipe_has_queries`
- `lineage_contains`
- `tool_success`
- `tool_response_budget`

`context_has` checks that field paths are present and non-null.
`context_field_equals` compares a field path to an exact JSON value.
`context_contains` checks that expected text appears either anywhere in context
or inside a specific field:

```yaml
assertions:
  - type: context_field_equals
    id_or_name: model.pkg.orders
    field: data.entity.name
    expected: orders
  - type: context_contains
    id_or_name: model.pkg.orders
    field: data.entity.description
    expected: canonical orders
```

`tool_response_budget` calls any Nova tool directly and asserts both serialized
response bytes and lightweight response shape. Field paths support object keys
and numeric array indexes:

```yaml
assertions:
  - type: tool_response_budget
    tool: search_indicator
    params:
      query: checkout conversion rate
      resource_types: [model]
      indicator_types: [metric]
      detail: compact
      group_mode: top
      limit: 3
    max_response_bytes: 12000
    must_contain_paths:
      - data.0.parent_unique_id
      - data.0.expression
    must_not_contain_paths:
      - parent_groups.1
      - data.0.explain
```

## MCP Tool Parity

Eval workflows are available through MCP and `dbt-nova tool call` with the same
report contracts used by the CLI:

| CLI command | MCP tool | MCP safety policy |
| --- | --- | --- |
| `eval validate` | `validate_eval_suite` | Reads suite files under the server working directory. |
| `eval gate` | `get_eval_gate` | Reads eval telemetry and returns gate report JSON. |
| `eval history` | `get_eval_history` | Reads eval telemetry and returns filtered rows. |
| `eval run` | `run_eval` | Disabled unless `DBT_NOVA_MCP_ENABLE_EVAL_RUN=1`. |
| `eval init` | `init_eval_suite` | Disabled unless `DBT_NOVA_MCP_ENABLE_EVAL_WRITES=1`. |
| `eval agent run` | `run_agent_eval` | Disabled unless `DBT_NOVA_MCP_ENABLE_AGENT_EVAL=1`; custom provider commands also require `DBT_NOVA_MCP_ENABLE_CUSTOM_AGENT_PROVIDER=1`. |

MCP eval file paths are resolved under the server working directory and reject
traversal outside that root. `run_eval` executes against the manifest already
loaded by the MCP server; use CLI `eval run` when a one-shot run needs to supply
its own `--manifest-path` or `--manifest-uri`.

Hosted MCP deployments should leave eval write and execution flags disabled
unless the server is isolated for trusted operators. Local MCP deployments can
enable the flags deliberately when agents need to create suites or run evals.

## Run Agent Evals

Agent evals execute a provider CLI and score observed Nova tool calls. The
provider must already be configured to use dbt-nova as an MCP server or local
CLI-backed tool source. dbt-nova first reads sanitized JSONL rows written to
`DBT_NOVA_TRACE_TOOL_CALLS_PATH`; if that trace is empty, it falls back to
provider JSON event streams for MCP tool calls from supported presets such as
Codex, Claude, and OpenCode. Fallback parsing only accepts MCP server aliases
that look like Nova (`nova`, `dbt-nova`, `dbt_nova`, `dbtnova`, or
`dbt-nova-mcp`). If your client uses a different alias, set
`DBT_NOVA_EVAL_MCP_SERVER_ALIASES` to a comma-separated list of accepted aliases.

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
      selected_entity_ranks:
        - unique_id: model.pkg.orders
          tool: search_indicator
          max_rank: 3
      called_with:
        - tool: search_indicator
          contains:
            query: gross merchandise
      max_tool_calls: 4
      max_distinct_tools: 4
      max_total_response_bytes: 65536
      max_response_bytes_by_tool:
        search_indicator: 12000
        get_entity: 12000
      final_answer:
        must_contain:
          - gross merchandise value
```

`selected_entities` checks that an entity appeared anywhere in tool evidence.
`selected_entity_ranks` checks the ordered top entities captured from tool
responses and can be scoped to a specific tool. `called_with.params` matches
sanitized parameter values exactly where possible, while `called_with.contains`
checks case-insensitive substring matches against sanitized parameter summaries.
`called_with.params` supports only scalar values or arrays of scalar values
because trace rows intentionally drop nested objects.
Budget expectations score the sanitized trace:

- `max_tool_calls` caps total observed Nova calls.
- `max_distinct_tools` caps tool-surface breadth.
- `max_total_response_bytes` caps summed serialized response bytes.
- `max_response_bytes_by_tool` caps the largest response for named tools.

Run against the default `opencode` adapter:

```bash
dbt-nova eval agent run \
  --suite evals/analyst-agent.yml \
  --provider opencode \
  --provider-model opencode/deepseek-v4-flash-free \
  --manifest-path /path/to/target/manifest.json \
  --timeout-secs 600 \
  --fail-under 0.9
```

`--provider-model` is supported by the OpenCode preset and inserts
`--model <MODEL>` into the default `opencode run --format json ...`
invocation. Use `--provider-args-json` for custom provider commands or for
presets that need non-standard model flags.

Use repeatable `--case-id <ID>` to run only specific agent cases while
debugging provider behavior.

Supported provider presets are `opencode`, `codex`, `claude`, and `goose`.
Presets use each CLI's normal project/user configuration; dbt-nova injects
`DBT_MANIFEST_PATH`, `DBT_NOVA_STORAGE_INSTANCE_ID`, and
`DBT_NOVA_TRACE_TOOL_CALLS_PATH` into the provider process so local MCP/CLI
tool servers inherit the eval manifest and trace path. Remote hosted MCP
endpoints cannot inherit local trace environment variables, so use a provider
that emits MCP calls in its JSON output or keep the Nova server local when
asserting tool-use traces. Provider stdout fallback parsing is implemented for
Codex, Claude, and OpenCode event streams; Goose is supported as a provider
preset and should be used with local trace inheritance unless its JSON stream is
wrapped into one of the supported event shapes.

`final_answer` assertions are matched against extracted assistant final text
from provider JSON event streams. For custom providers that do not emit JSON
events, dbt-nova falls back to the provider stdout.

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

## Telemetry History

Use `--telemetry` when you want eval results to accumulate across runs instead
of only writing one-off report artifacts. Bridge and agent evals append one
JSON object per assertion to `.nova/eval-runs/telemetry/<suite>-<hash>.jsonl`:

Suites must define a non-empty `name:` to write telemetry. This keeps history
and retention scoped to one suite instead of mixing unrelated unnamed files.

```bash
dbt-nova eval run \
  --suite evals/agent-tokenomics-bridge.yml \
  --manifest-path tests/fixtures/tokenomics_manifest.json \
  --telemetry

dbt-nova eval agent run \
  --suite evals/agent-tokenomics-opencode.yml \
  --provider opencode \
  --manifest-path tests/fixtures/tokenomics_manifest.json \
  --telemetry
```

Rows include a run id, run/suite case counts, run assertion count, suite
name/path/hash, mode, case id, assertion name/type, status, run duration, output
directory, git SHA when available, and manifest hash for bridge runs. Agent rows
also include provider metadata and sanitized trace counters such as tool-call
count, distinct tool count, response bytes, and token counts when a provider
trace exposes them. Telemetry does not store raw SQL parameter maps, provider
stdout/stderr, or credentials.

Use `eval history` for a thin date filter over the JSONL file:

```bash
dbt-nova eval history --suite agent-tokenomics-bridge --since 2026-06-01
```

Use `--telemetry-retention <ROWS>` with `eval run` or `eval agent run` to keep
only the newest rows for that suite after appending the current run.

## Readiness Gates

Suites can declare an advisory readiness threshold:

```yaml
version: 1
name: analyst-smoke
gate:
  threshold: 0.9
```

After running the full suite with `--telemetry`, check the latest run:

```bash
dbt-nova eval gate analyst-smoke --json
```

The gate scans the suite telemetry JSONL, selects the latest run, computes its
pass rate, and reports `allowed`, `blocked`, `gate_configured`, `pass_rate`,
`total_evals`, `failed_evals`, `failed_eval_ids`, `failed_case_ids`, and the
telemetry timestamp. Configured gates require the latest telemetry to match the
current suite file hash, cover the full suite, and include every assertion row
for that run, so stale suite files, filtered `--case-id` runs, and row-trimmed
telemetry are blocked with rerun guidance. If the suite has no
`gate.threshold`, the command returns `allowed=true` with
`gate_configured=false`. If telemetry is missing, the command returns an
actionable message to run the suite with `--telemetry` first. If the latest
telemetry points at a suite file that cannot be read, the gate is blocked
because the threshold cannot be verified.

Gate results are advisory in this release. Use them to warn before
launch-readiness checks or high-stakes analysis; they do not hard-block MCP
tooling or hosted server startup.

## Tool Trace

When `DBT_NOVA_TRACE_TOOL_CALLS_PATH` is set, dbt-nova appends sanitized JSONL
rows for CLI, MCP, and eval tool calls. Rows include:

- `transport`
- `tool`
- `tool_call_index`
- `success`
- `duration_ms`
- safe parameter summaries such as `query`, `persona`, `id_or_name`, and
  `resource_types`
- `response_bytes`
- `response_truncated`
- `result_count`
- `total_available`
- `selected_unique_ids` extracted from the response
- `top_unique_ids` preserving the ordered top response entities where Nova can
  infer them

Trace rows deliberately do not record SQL recipe parameter maps or credential
values. Keep eval artifacts out of public bug reports unless you have reviewed
them for project-specific identifiers.

Use [Trace Inspection And Redaction](traces.md) to inspect trace JSONL, write a
Markdown summary, and create a redacted JSONL artifact before sharing trace
evidence.

## CI Pattern

Use bridge evals as a fast metadata quality gate after producing a manifest:

```bash
dbt-nova eval validate --suite evals/analyst-smoke.yml

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
