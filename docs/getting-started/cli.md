# CLI Commands

Use `dbt-nova` in two modes:

- No subcommand: start MCP server (backward compatible behavior)
- Subcommand: run one-shot CLI commands and exit

## Command Tree

```text
dbt-nova
├── server start [--transport] [--http-host] [--http-port] [--http-path] [--http-stateful-mode]
├── manifest load [--manifest-path|--manifest-uri] [--storage-instance-id] [--cleanup-storage-on-start] [--read-only] [--json]
├── manifest reload [--manifest-path|--manifest-uri] [--refresh-secs] [--storage-instance-id] [--cleanup-storage-on-start] [--read-only] [--json]
├── tool call <tool_name> [--params-json|--params-file|--params-stdin] [--manifest-path|--manifest-uri] [--storage-instance-id] [--cleanup-storage-on-start] [--read-only] [--json]
├── config show [--defaults] [--json]
├── config validate [--json]
├── storage inspect [--storage-instance-id] [--json]
├── storage prune [--max-keep] [--max-bytes] [--storage-instance-id] [--json]
├── storage cleanup [--storage-instance-id] [--json]
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
dbt-nova server start --http-host 0.0.0.0 --http-path /mcp
```

## Common Examples

### Build and inspect manifest indexes

```bash
dbt-nova manifest load \
  --manifest-path /path/to/target/manifest.json
```

### Rebuild manifest indexes with override settings

```bash
dbt-nova manifest reload \
  --manifest-path /path/to/target/manifest.json \
  --refresh-secs 300 \
  --json
```

### One-shot tool execution

```bash
dbt-nova tool call search \
  --params-json '{"query":"orders","limit":5}' \
  --manifest-path /path/to/target/manifest.json
```

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
  "data": { "status": "ready" },
  "meta": {
    "elapsed_ms": 42,
    "timestamp_ms": 1772304167827,
    "version": "0.0.2"
  },
  "error": null
}
```

On errors (`status: "error"`), `error` contains the standard Nova error payload.

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
