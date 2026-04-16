---
name: cli-governance
description: "Runs deterministic governance audits through the dbt-nova CLI. Use when you need reproducible metadata scoring, documentation/test gap detection, Nova metadata validation, blocker classification, and rerunnable audit evidence from the terminal."
license: MIT
allowed-tools: "Bash Read Write"
metadata:
  owner: "dbt-nova"
  persona: "governance"
  transport: "cli"
  version: "0.0.3"
---

# CLI Governance Skill (dbt-nova)

## Mission

Produce repeatable governance audits with frozen scope and explicit pass/fail evidence.

## Session contract

- Always pass `--manifest-path`.
- Reuse one `--storage-instance-id` for the whole audit session.
- Always use `--json`.
- Freeze scope before scoring and keep it stable for reruns.

## Required workflow

1. Preflight
- Run:
  - `dbt-nova health check --manifest-path /path/to/manifest.json --json`
- If not ready:
  - `dbt-nova manifest reload --manifest-path /path/to/manifest.json --json`
- Capture manifest identity from the health or reload result when reporting audit evidence.

2. Freeze scope
- Define one explicit scope contract before scoring:
  - resource types
  - package / tag / path limits
  - changed-file scope, if applicable
- Reuse the same scope for the final rerun.

3. Run the main audit
- Use `audit metadata-score` as the primary governance gate.
- Add `audit nova-meta` when schema YAML or Nova metadata is in scope.
- Treat sampled or partial outputs as triage only, not final governance decisions.

4. Extract blockers
- Use targeted tool calls only where you need more detail:
  - `list_entities`
  - `get_metadata_score`
  - `get_undocumented`
  - `get_test_coverage`
  - `get_entity`
  - `search` with `persona: "governance"` for triage support

5. Build the remediation queue
- Group blockers by:
  - docs
  - tests
  - ownership / governance fields
  - Nova metadata issues
- Every blocker should have an explicit retest condition.

6. Recheck
- Reload the manifest after fixes.
- Re-run the exact same scope.
- Compare blocker counts and gate outcomes, not just overall scores.

## Command patterns

### Run metadata audit

```bash
dbt-nova audit metadata-score \
  --selection-mode changed \
  --changed-files-json '["models/marts/orders.sql","models/marts/orders.yml"]' \
  --resource-types-json '["model"]' \
  --personas-json '["governance"]' \
  --thresholds-json '{"entity":{"governance":{"min_score":70,"severity":"required"}}}' \
  --manifest-path /path/to/manifest.json \
  --json
```

### Validate Nova metadata project-wide

```bash
dbt-nova audit nova-meta \
  --project-dir /path/to/dbt/project \
  --json
```

### Pull focused blocker detail

```bash
dbt-nova tool call get_undocumented \
  --manifest-path /path/to/manifest.json \
  --storage-instance-id governance-session \
  --params-file undocumented.json \
  --json
```

Example `undocumented.json`:

```json
{"resource_type":"model","package":"analytics_core","include_columns":true,"limit":100}
```

## Guardrails

- Always capture the manifest path and manifest identity used for the audit.
- Keep scope stable between the initial run and the rerun.
- Do not use broad search results as final audit evidence.
- Use `audit nova-meta` for schema/YAML correctness; do not infer Nova compliance from search quality alone.
- Prefer explicit output files for audit evidence when comparing runs over time.

## References

- `docs/getting-started/cli.md`
- `docs/features/metadata-audit.md`
- `docs/features/metadata-scoring.md`
