# MCP/CLI Parity

dbt-nova exposes two callable product surfaces:

- MCP tools, used by MCP clients and hosted deployments.
- CLI commands, used for one-shot local workflows and CI automation.

The MCP catalog currently contains 43 MCP tools. `dbt-nova tool call` is the
CLI bridge to that same canonical tool catalog. Other CLI leaf commands are
tracked below so each capability is either MCP-equivalent, a known parity gap,
explicitly safety-gated, or a lifecycle exception.

## Policy

Product capabilities should have MCP and CLI parity. A CLI command may remain
outside MCP parity only when it controls the process lifecycle itself, such as
starting the MCP server.

Operations that read local files, write reports, execute provider commands,
mutate storage, or warm caches need explicit MCP safety semantics before they
are exposed to hosted or remote clients.

`SafetyGated` means the MCP tool exists but rejects by default until an operator
sets the documented opt-in environment variable for local execution or writes.

## Current Matrix

| CLI command | Current MCP equivalent | Status | Owner | Notes |
| --- | --- | --- | --- | --- |
| `server start` | None | Lifecycle exception | - | Starts the MCP process and cannot be called from inside that process. |
| `manifest load` | `health` | Lifecycle exception | - | MCP server startup performs the initial manifest load; `health` reports the active loaded manifest and `reload_manifest` replaces it. |
| `manifest reload` | `reload_manifest` | Equivalent | - | MCP starts a background reload for the live server; CLI `manifest reload` and CLI `tool call reload_manifest` are one-shot reloads. |
| `manifest warm` | `warm_manifest` | SafetyGated | - | MCP semantic cache warmup requires `DBT_NOVA_MCP_ENABLE_MANIFEST_WARM=1` and uses the current manifest source. |
| `tool call <tool_name>` | 43 MCP tools | Equivalent | - | CLI tool-call mode supports the canonical MCP tool catalog. |
| `audit agent-readiness` | `get_agent_readiness` | Equivalent | - | MCP returns the same `agent_readiness.v1` report without CLI file writes. |
| `audit metadata-score` | `get_metadata_audit` | Equivalent | - | MCP returns the same metadata audit report without CLI file writes or exit semantics. |
| `audit nova-meta` | `validate_nova_meta` | Equivalent | - | MCP returns the same nova-meta validation report with scoped local path access. |
| `config show` | None | Gap | JOE-217 | Runtime config inspection is CLI-only today. |
| `config validate` | None | Gap | JOE-217 | Runtime config validation is CLI-only today. |
| `storage inspect` | None | Gap | JOE-217 | Storage admin inspection is CLI-only today. |
| `storage prune` | None | Gap | JOE-217 | Destructive storage admin needs explicit MCP safety gates. |
| `storage cleanup` | None | Gap | JOE-217 | Destructive storage admin needs explicit MCP safety gates. |
| `eval init` | `init_eval_suite` | SafetyGated | - | MCP file writes require `DBT_NOVA_MCP_ENABLE_EVAL_WRITES=1`. |
| `eval validate` | `validate_eval_suite` | Equivalent | - | MCP returns the same eval suite validation data. |
| `eval run` | `run_eval` | SafetyGated | - | MCP bridge eval execution uses the loaded manifest and requires `DBT_NOVA_MCP_ENABLE_EVAL_RUN=1`. |
| `eval agent run` | `run_agent_eval` | SafetyGated | - | MCP provider execution requires `DBT_NOVA_MCP_ENABLE_AGENT_EVAL=1`; custom commands also require `DBT_NOVA_MCP_ENABLE_CUSTOM_AGENT_PROVIDER=1`. |
| `eval gate` | `get_eval_gate` | Equivalent | - | MCP returns the same eval gate report data. |
| `eval history` | `get_eval_history` | Equivalent | - | MCP returns filtered eval telemetry rows in a standard envelope. |
| `health check` | `health` | Equivalent | - | Both surfaces report manifest/server readiness. |

## Drift Guards

The code keeps the canonical MCP names in `src/tools/catalog.rs`. Tests verify
that:

- MCP router names match the canonical MCP catalog.
- CLI `tool call` supports the same canonical MCP tool names.
- MCP tool count references in core docs match the catalog.
- CLI leaf commands have an explicit parity row and any gap has an owning issue.

When adding or removing a tool or CLI command, update the catalog, parity matrix,
docs, and tests in the same change.
