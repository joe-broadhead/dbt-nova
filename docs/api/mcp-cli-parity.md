# MCP/CLI Parity

dbt-nova exposes two callable product surfaces:

- MCP tools, used by MCP clients and hosted deployments.
- CLI commands, used for one-shot local workflows and CI automation.

The canonical MCP catalog currently contains 53 MCP tools. Runtime MCP exposure
is profile-filtered by `DBT_NOVA_TOOL_PROFILE` (`agent` by default, `all` for
the complete catalog). `dbt-nova tool call` is the CLI bridge to that same
canonical catalog. Other CLI leaf commands are tracked below so each capability
is either MCP-equivalent, a known parity gap, explicitly safety-gated, or a
lifecycle exception.

## Policy

Product capabilities should have MCP and CLI parity. A CLI command may remain
outside MCP parity only when it controls the process lifecycle itself, such as
starting the MCP server.

Operations that read local files, write reports, execute provider commands,
mutate storage, or warm caches need explicit MCP safety semantics before they
are exposed to hosted or remote clients.

Agent-modelling audits are intentionally handled by
`modelling_consistency_report` through MCP and `dbt-nova tool call`. There is no
separate `dbt-nova audit modelling` leaf in v1; readiness owns the report-file
and blocker exit-code workflow.

`SafetyGated` means the MCP tool exists but rejects by default until an operator
sets the documented opt-in environment variable for local execution or writes.
The canonical per-tool stability tier, profile membership, and safety-gate
matrix lives in [Tools Reference](tools.md#stability-and-profiles) and is
checked against `src/tools/catalog.rs`.

MCP tool bodies and CLI JSON command envelopes both expose the additive
top-level `api` response contract marker documented in
[Response Format](response-format.md#api-contract-marker). CLI JSON keeps its
own command envelope shape. CLI `tool call --json` unwraps the MCP-style tool
body so `.data` is the payload itself and tool bookkeeping lives under
`.meta.tool_response`; MCP budgeting and pagination metadata remain under
`_nova_result_meta` in MCP responses.

## Current Matrix

| CLI command | Current MCP equivalent | Status | Owner | Notes |
| --- | --- | --- | --- | --- |
| `server start` | None | Lifecycle exception | - | Starts the MCP process and cannot be called from inside that process. |
| `manifest load` | `health` | Lifecycle exception | - | MCP server startup performs the initial manifest load; `health` reports the active loaded manifest and `reload_manifest` replaces it. |
| `manifest reload` | `reload_manifest` | SafetyGated | - | MCP current-source reload is allowed; source, refresh, or storage changes require `DBT_NOVA_MCP_ENABLE_MANIFEST_RELOAD=1`. CLI `manifest reload` and CLI `tool call reload_manifest` are one-shot reloads. |
| `manifest warm` | `warm_manifest` | SafetyGated | - | MCP semantic cache warmup requires `DBT_NOVA_MCP_ENABLE_MANIFEST_WARM=1` and uses the current manifest source. |
| `tool call <tool_name>` | 53 MCP tools | Equivalent | - | CLI tool-call mode supports the canonical MCP tool catalog; server exposure may be narrowed by tool profile, allowlist, or denylist. |
| `audit agent-readiness` | `get_agent_readiness` | Equivalent | - | MCP returns the same `agent_readiness.v1` report without CLI file writes. |
| `audit metadata-score` | `get_metadata_audit` | Equivalent | - | MCP returns the same metadata audit report without CLI file writes or exit semantics. |
| `audit nova-meta` | `validate_nova_meta` | Equivalent | - | MCP returns the same nova-meta validation report with scoped local path access. |
| `config show` | `show_config` | Equivalent | - | MCP returns active runtime config or defaults without exposing credential env values. |
| `config validate` | `validate_config` | Equivalent | - | MCP validates the active runtime config and returns the same structured validation payload. |
| `storage inspect` | `inspect_storage` | Equivalent | - | MCP returns the same storage inventory payload without mutating storage. |
| `storage prune` | `prune_storage` | SafetyGated | - | MCP destructive pruning requires `DBT_NOVA_MCP_ENABLE_STORAGE_ADMIN=1`. |
| `storage cleanup` | `cleanup_storage` | SafetyGated | - | MCP destructive cleanup requires `DBT_NOVA_MCP_ENABLE_STORAGE_ADMIN=1`. |
| `eval init` | `init_eval_suite` | SafetyGated | - | MCP file writes require `DBT_NOVA_MCP_ENABLE_EVAL_WRITES=1`. |
| `eval validate` | `validate_eval_suite` | Equivalent | - | MCP returns the same eval suite validation data. |
| `eval run` | `run_eval` | SafetyGated | - | MCP bridge eval execution uses the loaded manifest and requires `DBT_NOVA_MCP_ENABLE_EVAL_RUN=1`. |
| `eval agent run` | `run_agent_eval` | SafetyGated | - | MCP provider execution requires `DBT_NOVA_MCP_ENABLE_AGENT_EVAL=1`; custom commands also require `DBT_NOVA_MCP_ENABLE_CUSTOM_AGENT_PROVIDER=1`. |
| `eval gate` | `get_eval_gate` | Equivalent | - | MCP returns the same eval gate report data. |
| `eval history` | `get_eval_history` | Equivalent | - | MCP returns filtered eval telemetry rows in a standard envelope. |
| `eval compare` | `compare_eval_runs` | Equivalent | - | MCP returns the same local `results.json` comparison and PR-ready Markdown while scoping paths under the server working directory. |
| `trace inspect` | `inspect_tool_trace` | Equivalent | - | MCP returns the same trace rows, parse warnings, and summary while scoping local paths under the server working directory. |
| `trace summarize` | `summarize_tool_trace` | SafetyGated | - | MCP returns the same summary data; Markdown report writes require `DBT_NOVA_MCP_ENABLE_TRACE_WRITES=1`. |
| `trace redact` | `redact_tool_trace` | SafetyGated | - | MCP safe-sharing redaction writes require `DBT_NOVA_MCP_ENABLE_TRACE_WRITES=1`. |
| `trace replay` | `replay_tool_trace` | Equivalent | - | MCP replays supported deterministic trace rows against the currently loaded manifest; CLI replay loads an explicit manifest source. |
| `health check` | `health` | Equivalent | - | Both surfaces report manifest/server readiness. |

## Drift Guards

The code keeps the canonical MCP names in `src/tools/catalog.rs`. Tests verify
that:

- MCP router names match the canonical MCP catalog.
- CLI `tool call` supports the same canonical MCP tool names.
- Every canonical MCP tool has stability metadata, profile membership, and a
  docs row in the Tools Reference.
- MCP and CLI JSON envelope docs cover the shared top-level `api` marker, CLI
  tool-call payload unwrapping, and `_nova_result_meta` as MCP transport
  metadata.
- MCP tool count references in core docs match the catalog.
- CLI leaf commands have an explicit parity row and any gap has an owning issue.

When adding or removing a tool or CLI command, update the catalog, stability
matrix, parity matrix, docs, and tests in the same change.
