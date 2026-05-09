# CLI Commands

Use `dbt-nova` in two modes:

- No subcommand: start MCP server (backward compatible behavior)
- Subcommand: run one-shot CLI commands and exit
- CLI surface: `16` CLI-only leaf commands, plus `tool call` access to all `33` MCP tools

## Command Tree

```text
dbt-nova
├── server start [--transport] [--http-host] [--http-port] [--http-path] [--http-stateful-mode]
├── manifest load [--manifest-path|--manifest-uri] [--storage-instance-id] [--cleanup-storage-on-start] [--read-only] [--json]
├── manifest reload [--manifest-path|--manifest-uri] [--refresh-secs] [--storage-instance-id] [--cleanup-storage-on-start] [--read-only] [--json]
├── manifest warm [--manifest-path|--manifest-uri] [--storage-instance-id] [--vector] [--sparse] [--reranker] [--force] [--json]
├── tool call <tool_name> [--params-json|--params-file|--params-stdin] [--manifest-path|--manifest-uri] [--storage-instance-id] [--cleanup-storage-on-start] [--read-only] [--json]
├── audit metadata-score [--selection-mode] [--changed-files-json|--changed-files-file] [--entity-ids-json|--entity-ids-file] [--resource-types-json] [--personas-json] [--thresholds-json|--thresholds-file] [--manifest-path|--manifest-uri] [--storage-instance-id] [--report-json-path] [--report-md-path] [--fail-on-no-targets] [--json]
├── audit nova-meta [--project-dir] [--path <PATH>...] [--resource-kind] [--resource-name] [--column] [--json]
├── config show [--defaults] [--json]
├── config validate [--json]
├── storage inspect [--storage-instance-id] [--json]
├── storage prune [--max-keep] [--max-bytes] [--storage-instance-id] [--json]
├── storage cleanup [--storage-instance-id] [--json]
├── eval init --out <PATH> [--persona] [--force]
├── eval validate --suite <PATH> [--json]
├── eval run --suite <PATH> [--manifest-path|--manifest-uri] [--storage-instance-id] [--output-dir] [--case-id <ID>...] [--fail-under] [--cleanup-storage-on-start] [--read-only] [--json]
├── eval agent run --suite <PATH> [--provider] [--provider-command] [--provider-args-json] [--manifest-path|--manifest-uri] [--storage-instance-id] [--output-dir] [--case-id <ID>...] [--timeout-secs] [--fail-under] [--cleanup-storage-on-start] [--read-only] [--json]
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

Hosted HTTP form:

```bash
PORT=8080 \
DBT_NOVA_SERVER_TRANSPORT=streamable_http \
DBT_NOVA_HTTP_EXPECT_AUTH_PROXY=true \
dbt-nova server start --http-host 0.0.0.0 --http-path /mcp
```

Hosted probe endpoints:

```bash
curl -fsS http://127.0.0.1:8080/healthz
curl -fsS http://127.0.0.1:8080/readyz
```

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
dbt-nova eval init --persona analyst --out evals/analyst-smoke.yml
dbt-nova eval validate --suite evals/analyst-smoke.yml

dbt-nova eval run \
  --suite evals/analyst-smoke.yml \
  --manifest-path /path/to/target/manifest.json \
  --fail-under 1.0 \
  --json
```

### Run provider-backed agent evals

```bash
dbt-nova eval agent run \
  --suite evals/analyst-agent.yml \
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

`tool call reload_manifest` runs as a one-shot reload and returns a
`SuccessResponse` payload with updated manifest settings and loaded manifest
metadata (`manifest_hash`, `manifest_version`, `entity_count`).

If both `manifest_uri` and `manifest_path` are provided in params, `manifest_path`
takes precedence.

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
    "version": "0.0.4"
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
