# CLI Commands

Use `dbt-nova` in two modes:

- No subcommand: start MCP server (backward compatible behavior)
- Subcommand: run one-shot CLI commands and exit
- CLI surface: 25 CLI leaf commands, including `tool call` access to the canonical 53-tool MCP catalog

For the command-by-command MCP equivalent map, see
[MCP/CLI Parity](../api/mcp-cli-parity.md).

## Command Tree

```text
dbt-nova
├── server start [--transport] [--http-host] [--http-port] [--http-path] [--http-stateful-mode]
├── manifest load [--manifest-path|--manifest-uri] [--storage-instance-id] [--cleanup-storage-on-start] [--read-only] [--json]
├── manifest reload [--manifest-path|--manifest-uri] [--refresh-secs] [--storage-instance-id] [--cleanup-storage-on-start] [--read-only] [--json]
├── manifest warm [--manifest-path|--manifest-uri] [--storage-instance-id] [--vector] [--sparse] [--reranker] [--force] [--json]
├── tool call <tool_name> [--params-json|--params-file|--params-stdin] [--manifest-path|--manifest-uri] [--storage-instance-id] [--cleanup-storage-on-start] [--read-only] [--json]
├── audit agent-readiness [--manifest-path|--manifest-uri] [--storage-instance-id] [--cleanup-storage-on-start] [--read-only] [--personas-json] [--thresholds-json|--thresholds-file] [--eval-gate-json|--eval-gate-file] [--report-json-path] [--report-md-path] [--fail-on-blockers] [--json]
├── audit metadata-score [--selection-mode] [--changed-files-json|--changed-files-file] [--entity-ids-json|--entity-ids-file] [--resource-types-json] [--personas-json] [--thresholds-json|--thresholds-file] [--include-breakdown] [--include-recommendations] [--manifest-path|--manifest-uri] [--storage-instance-id] [--report-json-path] [--report-md-path] [--fail-on-no-targets] [--json]
├── audit nova-meta [--project-dir] [--path <PATH>...] [--resource-kind] [--resource-name] [--column] [--json]
├── config show [--defaults] [--json]
├── config validate [--json]
├── storage inspect [--storage-instance-id] [--json]
├── storage prune [--max-keep] [--max-bytes] [--storage-instance-id] [--json]
├── storage cleanup [--storage-instance-id] [--json]
├── trace inspect --path <PATH> [--json]
├── trace summarize --path <PATH> [--report-md-path <PATH>] [--json]
├── trace redact --path <PATH> --out <PATH> [--json]
├── trace replay --path <PATH> [--manifest-path|--manifest-uri] [--storage-instance-id] [--cleanup-storage-on-start] [--read-only] [--json]
├── eval init --out <PATH> [--persona] [--force]
├── eval validate --suite <PATH> [--json]
├── eval run --suite <PATH> [--manifest-path|--manifest-uri] [--storage-instance-id] [--output-dir] [--telemetry] [--telemetry-retention] [--case-id <ID>...] [--fail-under] [--cleanup-storage-on-start] [--read-only] [--json]
├── eval agent run --suite <PATH> [--provider] [--provider-model] [--provider-command] [--provider-args-json] [--manifest-path|--manifest-uri] [--storage-instance-id] [--output-dir] [--telemetry] [--telemetry-retention] [--case-id <ID>...] [--timeout-secs] [--fail-under] [--cleanup-storage-on-start] [--read-only] [--json]
├── eval compare --before <PATH> --after <PATH> [--json]
├── eval gate <NAME> [--json]
├── eval history --suite <NAME> --since <YYYY-MM-DD>
└── health check [--manifest-path|--manifest-uri] [--json]
```

## No-Arg Compatibility

`dbt-nova` with no subcommand still starts the MCP server:

```bash
dbt-nova
```

Equivalent explicit form:

```bash
dbt-nova server start
```

Local HTTP form:

```bash
DBT_NOVA_SERVER_TRANSPORT=streamable_http \
dbt-nova server start --http-host 127.0.0.1 --http-port 8080 --http-path /mcp
```

Hosted probe endpoints:

```bash
curl -fsS http://127.0.0.1:8080/healthz
curl -fsS http://127.0.0.1:8080/readyz
```

Validate effective runtime posture before exposing a hosted process:

```bash
DBT_NOVA_PRESET=hosted-discovery \
dbt-nova config validate --json
```

The JSON response includes the active preset, effective tool filters, exposed
SQL/recipe/admin/write surfaces, hosted HTTP proxy acknowledgement, health
metrics exposure, storage/artifact writability posture, and warnings.

## Common Examples

### Build and inspect manifest indexes

```bash
dbt-nova manifest load \
  --manifest-path /path/to/target/manifest.json
```

### Build indexes with manifest pruning

```bash
DBT_NOVA_PRUNE_ALLOW_IDS='["model.my_proj.fct_orders","model.my_proj.dim_*"]' \
DBT_NOVA_PRUNE_DENY_IDS='["model.my_proj.dim_legacy_*"]' \
dbt-nova manifest load \
  --manifest-path /path/to/target/manifest.json \
  --json
```

Generate `DBT_NOVA_PRUNE_ALLOW_IDS` from a dbt selector:

```bash
DBT_NOVA_PRUNE_ALLOW_IDS="$(
  dbt ls -s <lineage selection expression> --output json --quiet \
  | jq -cs '[.[] | .unique_id]'
)" \
dbt-nova manifest load \
  --manifest-path /path/to/target/manifest.json \
  --json
```

Prune variables must be valid JSON arrays of strings. Invalid JSON fails config
validation and startup instead of falling back to an unpruned manifest.

### Rebuild manifest indexes with override settings

```bash
dbt-nova manifest reload \
  --manifest-path /path/to/target/manifest.json \
  --refresh-secs 300 \
  --json
```

### Warm manifest-scoped semantic caches and reranker files

```bash
dbt-nova manifest warm \
  --manifest-path /path/to/target/manifest.json \
  --vector \
  --sparse \
  --reranker \
  --json
```

If no component flag is supplied, `manifest warm` requests vector and sparse
warmup. On constrained machines or very large manifests, skip this command and
leave `DBT_NOVA_SEARCH_ENABLE_VECTOR=false`,
`DBT_NOVA_SEARCH_ENABLE_SPARSE=false`, and
`DBT_NOVA_SEARCH_ENABLE_RERANKER=false` for smoke tests.

### One-shot tool execution

```bash
dbt-nova tool call search \
  --params-json '{"query":"orders","limit":5}' \
  --manifest-path /path/to/target/manifest.json
```

### Resolve canonical measures and metrics directly

```bash
dbt-nova tool call search_indicator \
  --params-json '{"query":"average order value","indicator_types":["metric"],"persona":"analyst","limit":5}' \
  --manifest-path /path/to/target/manifest.json \
  --json
```

### Run agent-modelling audits

Agent-modelling findings are intentionally exposed through the existing
modelling report tool instead of a standalone `dbt-nova audit modelling`
wrapper:

```bash
dbt-nova tool call modelling_consistency_report \
  --manifest-path /path/to/target/manifest.json \
  --params-json '{"resource_types":["model","metric","semantic_model"],"limit":25}' \
  --json
```

Use `audit agent-readiness` when you need readiness blockers, Markdown/JSON
report files, or `--fail-on-blockers` CI behavior from the same modelling
findings.

### Inspect, summarize, and redact tool traces

```bash
dbt-nova trace inspect \
  --path out/nova-evals/tool-calls/custom-analyst-discovery.jsonl \
  --json

dbt-nova trace summarize \
  --path out/nova-evals/tool-calls/custom-analyst-discovery.jsonl \
  --report-md-path out/nova-evals/tool-calls/custom-analyst-discovery.trace.md

dbt-nova trace redact \
  --path out/nova-evals/tool-calls/custom-analyst-discovery.jsonl \
  --out out/nova-evals/tool-calls/custom-analyst-discovery.redacted.jsonl \
  --json

dbt-nova trace replay \
  --path out/nova-evals/tool-calls/custom-analyst-discovery.redacted.jsonl \
  --manifest-path /path/to/target/manifest.json \
  --json
```

See [Trace Inspection And Redaction](../features/traces.md) for safe-sharing
guidance, Markdown report fields, and deterministic replay limits.

### Agent-readiness report for CI

```bash
dbt-nova audit agent-readiness \
  --manifest-path /path/to/target/manifest.json \
  --thresholds-json '{"overall":{"min_score":70,"severity":"advisory"},"persona":{"engineer":{"min_score":70,"severity":"advisory"},"analyst":{"min_score":65,"severity":"advisory"},"governance":{"min_score":65,"severity":"advisory"}}}' \
  --report-json-path out/agent-readiness.json \
  --report-md-path out/agent-readiness.md \
  --json
```

Add `--fail-on-blockers` only after you have moved the relevant thresholds from
advisory to required. See [Agent Readiness Audit](../features/agent-readiness.md)
for the GitHub Actions pattern and report artifact guidance.

### Metadata audit for changed models

```bash
dbt-nova audit metadata-score \
  --selection-mode changed \
  --changed-files-json '["models/marts/orders.sql","models/marts/orders.yml"]' \
  --resource-types-json '["model"]' \
  --personas-json '["engineer","analyst","governance"]' \
  --thresholds-json '{"entity":{"engineer":{"min_score":70,"severity":"required"},"analyst":{"min_score":65,"severity":"advisory"},"governance":{"min_score":65,"severity":"advisory"}}}' \
  --manifest-path /path/to/target/manifest.json \
  --report-json-path out/metadata-audit.json \
  --report-md-path out/metadata-audit.md \
  --json
```

### Validate `meta.nova` across a dbt project

```bash
dbt-nova audit nova-meta \
  --project-dir /path/to/dbt/project \
  --json
```

### Validate a single YAML file while authoring Nova meta

```bash
dbt-nova audit nova-meta \
  --project-dir /path/to/dbt/project \
  --path models/marts/orders.yml
```

### Validate one model or one column only

```bash
dbt-nova audit nova-meta \
  --project-dir /path/to/dbt/project \
  --resource-kind model \
  --resource-name fct_orders
```

```bash
dbt-nova audit nova-meta \
  --project-dir /path/to/dbt/project \
  --resource-kind model \
  --resource-name fct_orders \
  --column order_date
```

### Run Nova bridge evals

```bash
dbt-nova eval init --persona analyst --out evals/custom-analyst-discovery.yml
dbt-nova eval validate --suite evals/custom-analyst-discovery.yml

dbt-nova eval run \
  --suite evals/custom-analyst-discovery.yml \
  --manifest-path /path/to/target/manifest.json \
  --fail-under 1.0 \
  --json
```

### Run provider-backed agent evals

```bash
dbt-nova eval agent run \
  --suite evals/custom-agent-smoke.yml \
  --provider opencode \
  --manifest-path /path/to/target/manifest.json \
  --timeout-secs 600
```

Agent evals score sanitized Nova tool-call traces, falling back to supported
provider JSON event streams when a remote MCP endpoint cannot inherit local
trace environment variables. The selected provider must already be configured to
use dbt-nova tools.

### Health diagnostics

```bash
dbt-nova health check \
  --manifest-path /path/to/target/manifest.json \
  --json
```

## `tool call` Parameter Input Modes

Exactly one of the following may be used at a time:

- `--params-json '{"query":"orders"}'`
- `--params-file /path/to/params.json`
- `--params-stdin` (reads full JSON payload from `stdin`)

Examples:

```bash
dbt-nova tool call get_entity \
  --params-file ./params/get_entity.json \
  --manifest-path /path/to/target/manifest.json
```

```bash
echo '{"query":"customers","limit":10}' | \
  dbt-nova tool call search --params-stdin --manifest-path /path/to/target/manifest.json
```

## `reload_manifest` via `tool call`

`reload_manifest` is available in CLI mode through `tool call`:

```bash
dbt-nova tool call reload_manifest \
  --manifest-path /path/to/target/manifest.json \
  --params-json '{"refresh_secs":300}' \
  --json
```

`tool call reload_manifest` runs as a one-shot reload and returns updated
manifest settings and loaded manifest metadata (`manifest_hash`,
`manifest_version`, `entity_count`) under the CLI envelope's `.data` field.
Tool-response bookkeeping such as `count` is available under
`.meta.tool_response`.

If both `manifest_uri` and `manifest_path` are provided in params, `manifest_path`
takes precedence.

MCP `reload_manifest` differs because it mutates a running server: it accepts
the request, starts a background rebuild, and keeps serving the previous
manifest until the new one is ready. No-argument MCP reloads refresh the current
source; changing `manifest_uri`, `manifest_path`, `refresh_secs`, or
`storage_instance_id` requires `DBT_NOVA_MCP_ENABLE_MANIFEST_RELOAD=1`. CLI
`manifest reload` and CLI `tool call reload_manifest` load once and return after
the target manifest is available.

## `warm_manifest` via `tool call`

`warm_manifest` is available through `tool call` for parity with
`manifest warm`, but it is disabled by default because it writes semantic cache
artifacts:

```bash
DBT_NOVA_MCP_ENABLE_MANIFEST_WARM=1 \
dbt-nova tool call warm_manifest \
  --manifest-path /path/to/target/manifest.json \
  --params-json '{"vector":true,"sparse":true}' \
  --json
```

The tool-call form uses the manifest source loaded for the tool call. The MCP
server form uses the currently configured live-server manifest source.
When no component flag is supplied, the tool requests vector and sparse warmup,
matching `manifest warm`; keep the safety gate unset for no-warm release or
large-manifest smoke tests.

## Operator admin tools via `tool call`

The config and storage admin MCP tools are also available through `tool call`:

```bash
dbt-nova tool call show_config \
  --manifest-path /path/to/target/manifest.json \
  --params-json '{"defaults":true}' \
  --json

dbt-nova tool call inspect_storage \
  --manifest-path /path/to/target/manifest.json \
  --params-json '{}' \
  --json
```

`prune_storage` and `cleanup_storage` are disabled by default because they
delete storage directories. Set `DBT_NOVA_MCP_ENABLE_STORAGE_ADMIN=1` only for
trusted local or isolated operator sessions:

```bash
DBT_NOVA_MCP_ENABLE_STORAGE_ADMIN=1 \
dbt-nova tool call prune_storage \
  --manifest-path /path/to/target/manifest.json \
  --params-json '{"max_keep":1}' \
  --json
```

## JSON Envelope and Exit Codes

When `--json` is passed, CLI commands return a standard envelope:

```json
{
  "command": "health check",
  "status": "success",
  "data": { "status": "ready", "ready_for_traffic": true },
  "meta": {
    "elapsed_ms": 42,
    "timestamp_ms": 1772304167827,
    "version": "0.0.6"
  },
  "error": null
}
```

Top-level health `status` can be `degraded` when the manifest is loaded but one or more enabled
semantic components are not yet query-ready. Use `ready_for_traffic` for automation gates.

On errors (`status: "error"`), `error` contains the standard Nova error payload.

`audit nova-meta` validates discovered `meta.nova` blocks against
`schemas/nova/v0.json` and applies additional local semantic checks such as
field references, duplicate semantic names, and invalid filter/value
combinations.

When scanning a project root, it skips common generated and vendor directories
by default, including `.git`, `.venv`, `venv`, `target`, `dbt_packages`, and
`node_modules`. Use explicit `--path` values when you intentionally want to
validate inside one of those trees.

Exit codes:

| Code | Category |
|---|---|
| `0` | Success |
| `1` | Invalid params / request shape |
| `2` | Manifest/index lifecycle errors |
| `3` | Runtime/server/provider errors |

See also:

- [Response Format](../api/response-format.md)
- [Error Codes](../api/error-codes.md)
