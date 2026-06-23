# OpenCode Reference Pack Design

## Purpose

The OpenCode reference pack should give dbt-nova users a deterministic starting
point for agent workflows without making OpenCode mandatory or forking the
shared skill source. This document defines the target layout, role model,
permission profiles, MCP examples, smoke evals, and portability boundaries for
the v0.0.12 OpenCode Reference Agent Pack milestone.

This is a design contract only. The installer, validator, generated pack files,
and certification workflow belong to follow-up issues.

## Current OpenCode Contract

The design is based on the current OpenCode docs for:

- [config locations and `.opencode` directories](https://opencode.ai/docs/config/)
- [agents and subagents](https://opencode.ai/docs/agents/)
- [agent skills](https://opencode.ai/docs/skills/)
- [permissions](https://opencode.ai/docs/permissions/)
- [MCP servers](https://opencode.ai/docs/mcp-servers/)
- [CLI MCP commands and headless server mode](https://opencode.ai/docs/cli/)

Important assumptions:

- Project config lives in `opencode.json` at the repo root.
- Project-local OpenCode files live under plural `.opencode/` directories such
  as `.opencode/agents/` and `.opencode/skills/`.
- Local and remote MCP servers are configured under the top-level `mcp` key.
- MCP tools are registered with the MCP server name as their tool prefix. This
  design uses the server name `nova`, so MCP tools appear as `nova_search`,
  `nova_get_entity`, and so on.
- New configs should use `permission`. The legacy boolean `tools` field still
  exists for compatibility, but it should not be used by the reference pack.
- OpenCode permissions resolve to `allow`, `ask`, or `deny`. Pattern matching
  supports wildcards, and the last matching rule wins.

## Goals And Non-Goals

Goals:

- Define the target checked-out pack layout for `.opencode/agents`,
  `.opencode/skills`, `opencode.json`, and MCP examples.
- Map each reference role to skills, allowed Nova MCP tools, denied tools, and
  approval-required actions.
- Separate read-only discovery, SQL execution, metadata/file edits, eval
  authoring, and review workflows.
- Keep shared Agent Skills as the source of truth.
- Define smoke eval coverage that proves the pack is wired correctly.

Non-goals:

- Do not implement the installer or validator here.
- Do not certify the pack.
- Do not require OpenCode for Nova users.
- Do not fork skill logic away from `.github/skills/*/SKILL.md`.
- Do not rely on OpenCode SDK or OpenCode server internals for the stable path.

## Source Of Truth

The shared skill packages in `.github/skills/` remain canonical:

- `analyst`
- `bi-engineer`
- `engineer`
- `eval-author`
- `governance`
- `kpi-debugger`
- `meta-authoring`
- `model-architect`
- `project-cleanup`

The OpenCode pack should copy those directories into `.opencode/skills/` during
installation. The copied skill folders are generated artifacts from Nova's
shared skill source and should not be edited by hand. If OpenCode ever needs a
small wrapper that cannot live in a portable skill, the wrapper must name the
canonical `.github/skills/<name>/SKILL.md` source and keep all durable workflow
instructions there.

OpenCode recognizes only its supported Agent Skills frontmatter fields. The
portable `allowed-tools` field in Nova's shared skills is advisory for clients
that understand it; the OpenCode reference pack must enforce tool access through
OpenCode `permission` entries on agents and project config.

## Target Layout

The implementation issue should generate or document this layout:

```text
opencode.json
.opencode/
  README.md
  agents/
    nova-analyst.md
    nova-governance.md
    nova-metadata-steward.md
    nova-eval-author.md
    nova-source-scout.md
    nova-sql-reviewer.md
    nova-provenance-reviewer.md
  skills/
    analyst/
      SKILL.md
      references/
      scripts/
    bi-engineer/
    engineer/
    eval-author/
    governance/
    kpi-debugger/
    meta-authoring/
    model-architect/
    project-cleanup/
```

Ownership:

| Path | Source | Owner | Notes |
| --- | --- | --- | --- |
| `opencode.json` | generated from pack template | OpenCode pack | Holds MCP config and conservative global permissions. Users may copy to their project and edit local paths. |
| `.opencode/agents/*.md` | generated from pack template | OpenCode pack | Role prompts, modes, skill permissions, task permissions, and tool permissions. |
| `.opencode/skills/*` | copied from `.github/skills/*` | shared skills | Generated copy. Do not hand-edit. |
| `.opencode/README.md` | generated from pack template | OpenCode pack | Lists generation source, validation commands, and local override guidance. |
| `evals/starter.yml` | repo eval suite | Nova evals | Baseline bridge and agent smoke coverage. |
| `evals/agent-tokenomics-opencode.yml` | repo eval suite | Nova evals | Provider-backed OpenCode agent smoke coverage. |
| `evals/agent-tokenomics-bridge.yml` | repo eval suite | Nova evals | Deterministic bridge coverage for compact tool contracts. |

## MCP Config Examples

Use `nova` as the MCP server key so all role permissions can target
`nova_*`. If a user changes the server key, every prefixed tool permission must
change with it.

Local stdio example:

```json
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "nova": {
      "type": "local",
      "command": ["dbt-nova", "server", "start"],
      "enabled": true,
      "timeout": 60000,
      "environment": {
        "DBT_MANIFEST_PATH": "target/manifest.json",
        "DBT_NOVA_EMBEDDINGS_CACHE_DIR": ".dbt-nova/.fastembed_cache",
        "DBT_NOVA_RESULT_PROFILE": "standard",
        "DBT_NOVA_MCP_RESULT_PROFILE": "compact"
      }
    }
  },
  "permission": {
    "nova_*": "deny",
    "edit": "ask",
    "bash": "ask"
  }
}
```

Remote streamable HTTP example:

```json
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "nova": {
      "type": "remote",
      "url": "https://nova.example.com/mcp",
      "enabled": true,
      "oauth": false,
      "headers": {
        "Authorization": "Bearer {env:DBT_NOVA_MCP_TOKEN}"
      },
      "timeout": 60000
    }
  },
  "permission": {
    "nova_*": "deny",
    "edit": "ask",
    "bash": "ask"
  }
}
```

Remote deployments must follow Nova's hosted-server security posture:
`streamable_http` has no built-in authentication. Non-loopback binds require
`DBT_NOVA_HTTP_EXPECT_AUTH_PROXY=true` and an authenticating reverse proxy in
front of dbt-nova.

## Permission Model

The pack should deny Nova MCP tools globally and reopen only the tools each
agent needs. This prevents a general OpenCode session from discovering and
using the whole Nova catalog accidentally.

Permission classes:

| Class | Default | Rationale |
| --- | --- | --- |
| Read-only discovery | `allow` per role | Search, metadata lookup, lineage, columns, and scoring are safe when scoped to the manifest. |
| SQL execution | `ask` for analyst roles, `deny` for reviewers by default | SQL can touch warehouse data and needs explicit user approval and provider config. |
| Recipe execution | `ask` for analyst roles, `deny` elsewhere | Recipes may execute SQL or produce files depending on recipe design. |
| Metadata and file edits | `ask` for steward/eval author, `deny` for reviewers and scout | Human approval is required before modifying dbt project files or eval suites. |
| Eval authoring and provider runs | `ask` for `nova-eval-author`, `deny` elsewhere | Provider-backed evals can run external CLIs and write artifacts. |
| Storage/admin mutation | `deny` by default, `ask` only for explicit maintenance roles later | Reload, warm, prune, and cleanup operations can mutate local storage or server state. |
| Review | `allow` for read-only trace/provenance tools, `deny` for edits and execution | Reviewers must be able to inspect evidence without changing state. |

The generated global config should start from:

```json
{
  "$schema": "https://opencode.ai/config.json",
  "permission": {
    "nova_*": "deny",
    "edit": "ask",
    "bash": {
      "*": "ask",
      "git status*": "allow",
      "git diff*": "allow",
      "dbt-nova eval validate *": "allow"
    },
    "skill": {
      "*": "deny",
      "analyst": "allow",
      "bi-engineer": "allow",
      "engineer": "allow",
      "eval-author": "allow",
      "governance": "allow",
      "kpi-debugger": "allow",
      "meta-authoring": "allow",
      "model-architect": "allow",
      "project-cleanup": "allow"
    }
  }
}
```

Agent-specific permissions then override these defaults.

## Tool Bundles

The role matrix below uses named bundles to keep the table readable. Bundle
contents are exact OpenCode MCP tool names for an MCP server named `nova`.

| Bundle | Tools |
| --- | --- |
| `discovery` | `nova_health`, `nova_show_metadata`, `nova_search`, `nova_search_indicator`, `nova_indicator_inventory`, `nova_search_columns`, `nova_column_inventory`, `nova_search_recipes`, `nova_get_recipe`, `nova_get_entity`, `nova_batch_get_entities`, `nova_list_entities`, `nova_find_by_path`, `nova_get_columns`, `nova_get_context` |
| `structure` | `nova_get_sql`, `nova_get_lineage`, `nova_get_column_lineage`, `nova_get_impact`, `nova_validate_dag`, `nova_compare_grains`, `nova_find_entity_overlap`, `nova_modelling_consistency_report`, `nova_diff_entities` |
| `metadata-audit` | `nova_validate_nova_meta`, `nova_get_metadata_score`, `nova_get_metadata_audit`, `nova_get_agent_readiness`, `nova_get_test_coverage`, `nova_get_undocumented`, `nova_list_tags`, `nova_list_packages`, `nova_list_databases` |
| `eval-read` | `nova_validate_eval_suite`, `nova_get_eval_gate`, `nova_get_eval_history`, `nova_compare_eval_runs`, `nova_inspect_tool_trace`, `nova_replay_tool_trace` |
| `trace-write` | `nova_summarize_tool_trace`, `nova_redact_tool_trace` |
| `eval-write` | `nova_init_eval_suite`, `nova_run_eval`, `nova_run_agent_eval` |
| `execution` | `nova_execute_sql`, `nova_run_recipe` |
| `admin-mutation` | `nova_reload_manifest`, `nova_warm_manifest`, `nova_inspect_storage`, `nova_prune_storage`, `nova_cleanup_storage`, `nova_show_config`, `nova_validate_config` |
| `local-change` | `edit`, `bash` |

`nova_inspect_storage`, `nova_show_config`, and `nova_validate_config` are read
or validation oriented, but they are grouped with admin tools because they often
expose deployment posture. Roles may allow them explicitly when needed.

## Role Matrix

| Role | Mode | Skills | Allowed tools | Approval required | Denied tools and actions | Smoke eval expectation |
| --- | --- | --- | --- | --- | --- | --- |
| `nova-analyst` | `primary` | `analyst`, `kpi-debugger` | `discovery`, selected `structure` (`nova_get_sql`, `nova_get_lineage`, `nova_get_column_lineage`, `nova_compare_grains`, `nova_diff_entities`) | `execution`; `bash` except allowlisted `git status*`, `git diff*`, and `dbt-nova eval validate *`; task access to `nova-governance` | `eval-write`, `trace-write`, `admin-mutation`, `edit`; all unlisted `nova_*` | `evals/starter.yml` case `analyst_revenue_lookup_flow`; `evals/agent-tokenomics-opencode.yml` cases `metric_contract_no_execution` and `uk_checkout_conversion_answer` when SQL is enabled |
| `nova-governance` | `primary` and `subagent` | `governance`, `project-cleanup` | `discovery`, `metadata-audit`, selected `structure` (`nova_get_lineage`, `nova_get_impact`, `nova_find_entity_overlap`, `nova_modelling_consistency_report`, `nova_compare_grains`) | `bash` for local report commands; task access to `nova-provenance-reviewer` | `execution`, `eval-write`, `trace-write`, `admin-mutation`, `edit`; all unlisted `nova_*` | `evals/starter.yml` cases `sparse_metadata_floor`, `canonical_revenue_discovery`; planned agent-readiness smoke for JOE-24/JOE-27 follow-up |
| `nova-metadata-steward` | `primary` | `meta-authoring`, `model-architect`, `governance` | `discovery`, `metadata-audit`, `structure` | `edit`; `bash` for dbt compile/test and Nova validation; `nova_reload_manifest` after local manifest rebuild | `execution` by default, `eval-write`, `trace-write`, storage pruning/cleanup; all unlisted `nova_*` | `dbt-nova audit nova-meta` against fixture metadata; `evals/starter.yml` metadata score cases; planned `.opencode` pack validator in JOE-28 |
| `nova-eval-author` | `primary` | `eval-author` | `discovery`, `eval-read`, selected `metadata-audit` (`nova_get_metadata_score`, `nova_get_test_coverage`) | `edit`; `bash`; `eval-write`; `trace-write`; `execution` only when an eval case explicitly requires SQL | `admin-mutation` except config validation; all unrelated `nova_*` | `dbt-nova eval validate --suite evals/starter.yml`; `dbt-nova eval validate --suite evals/agent-tokenomics-opencode.yml`; provider smoke for one OpenCode case when credentials are available |
| `nova-source-scout` | `subagent` | `analyst` | read-only subset of `discovery`: `nova_health`, `nova_show_metadata`, `nova_search`, `nova_search_indicator`, `nova_indicator_inventory`, `nova_search_columns`, `nova_column_inventory`, `nova_get_entity`, `nova_list_entities`, `nova_find_by_path`, `nova_get_columns`, `nova_get_context` | none | `structure` that reveals SQL, `execution`, `metadata-audit`, `eval-write`, `trace-write`, `admin-mutation`, `edit`, `bash`; all unlisted `nova_*` | `evals/starter.yml` cases `canonical_revenue_discovery`, `recipe_discovery` without executing recipes |
| `nova-sql-reviewer` | `subagent` | `engineer`, `kpi-debugger` | selected `discovery`; selected `structure` (`nova_get_sql`, `nova_get_lineage`, `nova_get_column_lineage`, `nova_compare_grains`, `nova_get_test_coverage`, `nova_validate_dag`) | `bash` only for read-only local inspection if the primary agent requests it | `execution`, `eval-write`, `trace-write`, `admin-mutation`, `edit`; all unlisted `nova_*` | `evals/starter.yml` case `column_context_and_lineage`; planned adversarial reviewer suite from JOE-15 |
| `nova-provenance-reviewer` | `subagent` | `governance`, planned reviewer skill from JOE-15 | `discovery`, `metadata-audit`, `eval-read`, selected `structure` (`nova_get_lineage`, `nova_get_column_lineage`, `nova_get_impact`) | `trace-write` only when producing redacted artifacts; `bash` only for local artifact inspection | `execution`, `eval-write`, storage pruning/cleanup, `edit`; all unlisted `nova_*` | `evals/agent-tokenomics-bridge.yml` trace/budget cases; planned JOE-15 stale-source and semantic-bypass reviewer suite |

## Agent Template Pattern

Generated agent files should keep prompt text short and push workflow detail to
skills. A primary analyst agent should follow this pattern:

```yaml
---
description: Answer business questions with Nova discovery, bounded SQL approval, and evidence-first reporting.
mode: primary
permission:
  nova_*: deny
  nova_health: allow
  nova_show_metadata: allow
  nova_search: allow
  nova_search_indicator: allow
  nova_indicator_inventory: allow
  nova_get_entity: allow
  nova_get_columns: allow
  nova_get_lineage: allow
  nova_get_context: allow
  nova_get_metadata_score: allow
  nova_execute_sql: ask
  nova_run_recipe: ask
  edit: deny
  bash:
    "*": ask
    "git status*": allow
    "git diff*": allow
    "dbt-nova eval validate *": allow
  skill:
    "*": deny
    analyst: allow
    kpi-debugger: allow
  task:
    "*": deny
    nova-source-scout: allow
    nova-sql-reviewer: allow
    nova-provenance-reviewer: allow
    nova-governance: ask
---

Use the `analyst` skill for the durable workflow. Resolve canonical metrics and
entities before asking for SQL approval. Keep final answers tied to Nova
evidence and name the tools used.
```

Subagent templates should set `mode: subagent`, narrow `task` permissions to
`deny`, and deny `edit`, `bash`, and `execution` unless the role matrix grants
an explicit exception.

## Smoke Eval Expectations

Minimum validation for the design and pack implementation:

```bash
uv run mkdocs build --strict
dbt-nova eval validate --suite evals/starter.yml
dbt-nova eval validate --suite evals/agent-tokenomics-bridge.yml
dbt-nova eval validate --suite evals/agent-tokenomics-opencode.yml
```

Bridge smoke for deterministic tool contracts:

```bash
dbt-nova eval run \
  --suite evals/agent-tokenomics-bridge.yml \
  --manifest-path tests/fixtures/tokenomics_manifest.json \
  --storage-instance-id tokenomics-bridge \
  --cleanup-storage-on-start \
  --fail-under 1.0 \
  --json
```

OpenCode provider smoke when the provider and model are available:

```bash
DBT_NOVA_SQL_PROVIDER=duckdb \
DBT_NOVA_DUCKDB_PATH=tests/fixtures/tokenomics.duckdb \
DBT_NOVA_TOOL_ALLOWLIST=show_metadata,search_indicator,search,get_entity,get_columns,search_columns,execute_sql \
dbt-nova eval agent run \
  --suite evals/agent-tokenomics-opencode.yml \
  --provider opencode \
  --manifest-path tests/fixtures/tokenomics_manifest.json \
  --storage-instance-id tokenomics-opencode \
  --cleanup-storage-on-start \
  --case-id metric_contract_no_execution \
  --fail-under 1.0
```

Manual pack checklist:

- Every generated role has `mode`, skills, allowed tools, approval-required
  tools, denied tools, task permissions, and an eval expectation.
- The MCP server key is `nova`, or all prefixed permissions have been rewritten.
- Local MCP examples use `environment`, not `env`.
- Remote MCP examples require an auth layer and use `/mcp`.
- SQL execution remains approval-gated.
- File edits and eval/provider runs remain approval-gated.
- Generated `.opencode/skills/*` directories match `.github/skills/*`.
- No secrets, manifest URIs with credentials, or local customer paths are
  committed.

## Portability Boundaries

Portable:

- Skill names and durable workflows in `.github/skills/*/SKILL.md`.
- Role concepts such as analyst, governance, metadata steward, eval author, and
  reviewer.
- Nova MCP tool groups and safety posture.
- Eval suites proving discovery, compact response budgets, and provider-backed
  OpenCode behavior.

OpenCode-specific:

- `opencode.json` syntax and config precedence.
- `.opencode/agents/*.md` frontmatter.
- `permission` object syntax, wildcard order, and `task` permission names.
- MCP tool prefixes derived from the OpenCode MCP server key.
- OpenCode provider smoke commands and provider event parsing.

Later Codex and Claude work should reuse the shared skills and role concepts,
but must translate permissions, MCP config, task/subagent mechanics, and install
paths into each client's native model. Do not copy OpenCode agent frontmatter
directly into those clients.

## Risks

- OpenCode config syntax can evolve. Use the official docs during implementation
  and prefer `permission` over legacy `tools`.
- The MCP prefix depends on the configured server key. The reference pack must
  keep `nova` stable or rewrite every prefixed permission.
- Remote MCP auth belongs to the hosting layer. dbt-nova streamable HTTP should
  stay behind an authenticating proxy when exposed beyond loopback.
- Copied skills can drift from `.github/skills`. The implementation issue
  needs a maintenance check that compares generated `.opencode/skills` content
  to the canonical source.
- The OpenCode SDK and headless server are useful for future automation, but the
  stable reference path should remain CLI + MCP + Agent Skills until those
  surfaces are explicitly adopted.

## Follow-Up Implementation Issues

- JOE-24: turn the analyst role and smoke contract into a semantic-first
  workflow.
- JOE-27: add a domain reference skeleton that the OpenCode pack can point to
  without embedding customer-specific context.
- JOE-15: implement the adversarial reviewer skill and stale-source checks used
  by `nova-provenance-reviewer`.
- JOE-28: add generated-skill/reference maintenance checks for `.opencode`
  outputs and dbt model changes.
