# Agent Readiness Audit

`dbt-nova audit agent-readiness` produces a manifest-level readiness report for
agent workflows. It combines project metadata scores, entity-level metadata
signals, Nova indicator metadata, and optional eval gate evidence into one JSON
and Markdown artifact.

Use it before enabling a manifest for production agent analysis, launch reviews,
or recurring CI evidence.

MCP clients can request the same JSON report with `get_agent_readiness`.
The MCP tool accepts inline `personas_json`, `thresholds_json`, and
`eval_gate_json`, and returns the report without writing files or applying CLI
exit semantics.

## Command

```bash
dbt-nova audit agent-readiness \
  --manifest-path target/manifest.json \
  --report-json-path out/agent-readiness.json \
  --report-md-path out/agent-readiness.md \
  --json
```

The command is metadata-only by default. It loads the manifest with vector,
sparse, and reranker search disabled, even when those search features are
enabled in the surrounding environment.

## Inputs

- `--manifest-path` or `--manifest-uri`: manifest or prebuilt artifact source
- `--storage-instance-id`: stable storage instance for cached manifest indexes
- `--cleanup-storage-on-start`: remove the selected storage instance before
  loading
- `--read-only`: reuse already materialized storage without writes
- `--personas-json`: personas to score, defaulting to
  `["engineer","analyst","governance"]`
- `--thresholds-json` or `--thresholds-file`: advisory or required readiness
  thresholds
- `--eval-gate-json` or `--eval-gate-file`: output from
  `dbt-nova eval gate <SUITE> --json`
- `--report-json-path` and `--report-md-path`: report artifact destinations
- `--fail-on-blockers`: exit non-zero when blocking findings are present
- `--json`: print a CLI envelope containing the report

## Thresholds

Thresholds are optional JSON. Default thresholds are advisory so the command can
produce evidence without failing valid manifests.

```json
{
  "overall": { "min_score": 70, "severity": "advisory" },
  "persona": {
    "engineer": { "min_score": 70, "severity": "advisory" },
    "analyst": { "min_score": 65, "severity": "advisory" },
    "governance": { "min_score": 65, "severity": "advisory" }
  }
}
```

Set `severity` to `required` for a threshold that should create a blocker.
`--fail-on-blockers` turns blockers into a failing CLI exit after reports have
been written.

## Eval Gate Evidence

Agent readiness can include the current eval gate status:

```bash
dbt-nova eval gate analyst-smoke --json > out/eval-gate.json

dbt-nova audit agent-readiness \
  --manifest-path target/manifest.json \
  --eval-gate-file out/eval-gate.json \
  --report-json-path out/agent-readiness.json \
  --report-md-path out/agent-readiness.md \
  --fail-on-blockers \
  --json
```

The readiness command accepts either the raw gate report or the full CLI
envelope emitted by `eval gate --json`. A blocked eval gate is recorded as a
readiness blocker. Missing eval gate evidence is recorded as a next action, not
as a blocker.

## Report Contract

JSON reports use `schema_version: "agent_readiness.v1"` and include:

- sanitized manifest source, hash, version, entity counts, and search readiness
- personas, selected resource types, thresholds, storage mode, and metadata-only
  mode
- overall readiness score, grade, readiness band, and gate status
- per-persona project metadata scores and threshold status
- blocking findings and improvement findings
- lowest-scoring or signal-poor entity findings with top recommendations
- ambiguous Nova indicator definitions that need stronger execution metadata
- eval gate status, if supplied
- ordered next actions

Markdown reports contain the same evidence in a compact review format for PR
comments, release notes, or CI job summaries.

## MCP Tool

```json
{"name":"get_agent_readiness","arguments":{"personas_json":"[\"engineer\",\"analyst\"]"}}
```

To include eval gate evidence, pass either the raw gate report or the full
`eval gate --json` CLI envelope as `eval_gate_json`.
