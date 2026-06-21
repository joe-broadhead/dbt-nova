# MCP/CLI Parity

dbt-nova exposes two callable product surfaces:

- MCP tools, used by MCP clients and hosted deployments.
- CLI commands, used for one-shot local workflows and CI automation.

The MCP catalog currently contains 33 MCP tools. `dbt-nova tool call` is the
CLI bridge to that same canonical tool catalog. Other CLI leaf commands are
tracked below so each capability is either MCP-equivalent, a known parity gap,
or a lifecycle exception.

## Policy

Product capabilities should have MCP and CLI parity. A CLI command may remain
outside MCP parity only when it controls the process lifecycle itself, such as
starting the MCP server.

Operations that read local files, write reports, execute provider commands,
mutate storage, or warm caches need explicit MCP safety semantics before they
are exposed to hosted or remote clients.

## Current Matrix

| CLI command | Current MCP equivalent | Status | Owner | Notes |
| --- | --- | --- | --- | --- |
| `server start` | None | Lifecycle exception | - | Starts the MCP process and cannot be called from inside that process. |
| `manifest load` | `reload_manifest` | Gap | JOE-216 | MCP reloads a running server; CLI load is a one-shot lifecycle command. |
| `manifest reload` | `reload_manifest` | Gap | JOE-216 | Semantics differ between one-shot CLI reload and live MCP reload. |
| `manifest warm` | None | Gap | JOE-216 | Cache warming is CLI-only today. |
| `tool call <tool_name>` | 33 MCP tools | Equivalent | - | CLI tool-call mode supports the canonical MCP tool catalog. |
| `audit agent-readiness` | None | Gap | JOE-213 | Agent-readiness report is CLI-only today. |
| `audit metadata-score` | `get_metadata_score` | Gap | JOE-218 | MCP has the primitive score tool but not the audit/report/gate wrapper. |
| `audit nova-meta` | None | Gap | JOE-214 | Project-file nova-meta validation is CLI-only today. |
| `config show` | None | Gap | JOE-217 | Runtime config inspection is CLI-only today. |
| `config validate` | None | Gap | JOE-217 | Runtime config validation is CLI-only today. |
| `storage inspect` | None | Gap | JOE-217 | Storage admin inspection is CLI-only today. |
| `storage prune` | None | Gap | JOE-217 | Destructive storage admin needs explicit MCP safety gates. |
| `storage cleanup` | None | Gap | JOE-217 | Destructive storage admin needs explicit MCP safety gates. |
| `eval init` | None | Gap | JOE-215 | Eval file creation is CLI-only today. |
| `eval validate` | None | Gap | JOE-215 | Eval suite validation is CLI-only today. |
| `eval run` | None | Gap | JOE-215 | Deterministic eval execution is CLI-only today. |
| `eval agent run` | None | Gap | JOE-215 | Provider-backed execution needs explicit MCP safety controls. |
| `eval gate` | None | Gap | JOE-215 | Eval gate reporting is CLI-only today. |
| `eval history` | None | Gap | JOE-215 | Eval telemetry history reporting is CLI-only today. |
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
