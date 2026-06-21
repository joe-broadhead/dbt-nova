# Trace Inspection And Redaction

Nova can record sanitized tool-call traces when
`DBT_NOVA_TRACE_TOOL_CALLS_PATH` is set. Trace files are JSONL: one JSON object
per observed Nova tool call. Use the `trace` commands to inspect, summarize, and
redact those files before attaching them to PRs, eval artifacts, or support
threads.

Trace commands operate on local files only. They do not replay tool calls, call
providers, execute SQL, upload artifacts, or read arbitrary provider logs.

## Capture A Trace

```bash
DBT_NOVA_TRACE_TOOL_CALLS_PATH=out/tool-calls/analyst-smoke.jsonl \
dbt-nova tool call search_indicator \
  --params-json '{"query":"gross margin","indicator_types":["metric"],"limit":5}' \
  --manifest-path target/manifest.json \
  --json
```

Agent evals set the same trace environment for provider processes when local
trace inheritance is available. See [Nova Evals](evals.md#tool-trace) for eval
trace details.

## Inspect

```bash
dbt-nova trace inspect \
  --path out/tool-calls/analyst-smoke.jsonl \
  --json
```

`trace inspect` reads valid rows, normalizes `tool_call_index` by file order,
and reports malformed JSONL lines as parse warnings instead of panicking.
Missing files fail with a clear error. Empty files return a successful report
with `row_count: 0`.

The JSON report includes:

- raw valid trace rows
- malformed row warnings with line numbers
- tool order and tool counts
- distinct tools
- selected unique IDs and top ranked unique IDs
- total and per-tool response byte budgets
- response truncation counts
- failed tool-call counts and error codes
- semantic-first signal for `search_indicator` before `execute_sql` when the
  trace contains enough evidence

## Summarize

```bash
dbt-nova trace summarize \
  --path out/tool-calls/analyst-smoke.jsonl \
  --report-md-path out/tool-calls/analyst-smoke.trace.md
```

`trace summarize` produces a compact Markdown report for PR comments, release
evidence, and eval artifacts. The report includes overview counters, ordered
tool calls, tool counts, response budgets, semantic-first status, selected
entity IDs, top ranked IDs, and parse warnings.

Use `--json` when automation needs the same summary contract in a CLI envelope.
Omit `--report-md-path` to print the Markdown report to stdout.

## MCP And Tool-Call Parity

The same trace review surface is available to MCP clients and
`dbt-nova tool call`:

- `inspect_tool_trace` maps to `trace inspect`
- `summarize_tool_trace` maps to `trace summarize`
- `redact_tool_trace` maps to `trace redact`

MCP trace paths are scoped under the server working directory. Read-only inspect
and summarize calls can return JSON data directly. Markdown report writes and
redacted JSONL writes require `DBT_NOVA_MCP_ENABLE_TRACE_WRITES=1`.

## Redact

```bash
dbt-nova trace redact \
  --path out/tool-calls/analyst-smoke.jsonl \
  --out out/tool-calls/analyst-smoke.redacted.jsonl \
  --json
```

`trace redact` writes valid JSONL using a conservative allowlist. It preserves:

- `timestamp_ms`, `tool_call_index`, `transport`, and `tool`
- `success`, `duration_ms`, and `error_code`
- safe scalar parameter summaries such as `query`, `persona`, `id_or_name`,
  `resource_type`, `indicator_types`, `limit`, and `offset`
- `response_bytes`, `response_truncated`, `result_count`, and
  `total_available`
- `selected_unique_ids` and `top_unique_ids`

It removes or masks:

- raw nested `params`, SQL parameter maps, and arbitrary nested objects
- credentials, tokens, passwords, private keys, and authorization-like fields
- provider raw output fields if they appear in a trace row
- manifest, artifact, or storage URIs outside the allowlist
- URI query strings or path segments that contain token-like values
- malformed JSONL rows, which are reported in the redaction summary

Redacted output remains compatible with `trace inspect` and `trace summarize`.

## Safe To Share Checklist

Before sharing trace artifacts:

- Share the redacted JSONL file, not the raw trace.
- Share the Markdown summary when reviewers only need behavior evidence.
- Review `selected_unique_ids`, `top_unique_ids`, and safe scalar query text;
  they may still reveal project model names or business terms.
- Keep raw traces, provider stdout/stderr, warehouse logs, and eval working
  directories private unless they have been reviewed separately.
- Do not attach traces that include binary artifacts, third-party provider logs,
  raw SQL parameter maps, credentials, private keys, tokens, or private
  manifest/artifact URIs.
- Treat redaction as a local safety workflow, not a guarantee that arbitrary
  third-party logs are safe.

## Limitations

Trace summaries prove tool-use behavior, not answer correctness. Use bridge or
provider-backed evals when you need assertions about ranked results, selected
entities, or final answers.

The redactor intentionally drops detail when it cannot prove a field is safe.
This can remove useful debugging context, but avoids leaking sensitive values.
