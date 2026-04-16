---
name: mcp-governance
description: "Runs deterministic governance audits through Nova MCP tools. Use when enforcing metadata standards, scoring entity quality, extracting documentation/test gaps, and producing rerunnable pass/fail audit evidence."
license: MIT
allowed-tools: "mcp__nova__health mcp__nova__reload_manifest mcp__nova__list_entities mcp__nova__batch_get_entities mcp__nova__get_metadata_score mcp__nova__get_undocumented mcp__nova__get_test_coverage mcp__nova__get_entity mcp__nova__search Read"
metadata:
  owner: "dbt-nova"
  persona: "governance"
  version: "0.0.3"
---

# MCP Governance Skill (dbt-nova)

## Transport contract

- Run `health` before substantive work.
- If status is not `ready`, run `reload_manifest` and wait for readiness.
- MCP does not expose the CLI-only `audit nova-meta` validator.
- Use MCP governance tools for metadata, documentation, and test audits; use the CLI separately for Nova schema/YAML validation.

## MCP surface

- `list_entities`: frozen scope inventory
- `get_metadata_score`: project baselines and entity-level gates
- `get_undocumented` / `get_test_coverage` / `get_entity`: blocker detail
- `search` with `persona: "governance"`: triage support only
- `reload_manifest` / `health`: rerun readiness

## References

- `../../shared/governance/references/workflow.md`
- `../../shared/governance/references/metadata-rubric.md`
- `../../shared/common/references/manifest-refresh.md`
- `../../shared/governance/assets/governance-audit-template.md`
- `../../shared/governance/assets/remediation-queue-template.md`
- `references/tool-recipes.md`
