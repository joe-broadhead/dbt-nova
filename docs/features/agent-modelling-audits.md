# Agent Modelling Audits

`modelling_consistency_report` includes deterministic `agent_modelling_findings`
for project-shape risks that make agent answers ambiguous or unsafe. These
findings are produced from the dbt manifest, `meta.nova`, dbt Semantic Layer /
MetricFlow artifacts, optional `catalog.json`, and lineage/index evidence. They
do not use LLM judgement.

Use the audit when you need to answer:

- which indicators are queryable and through which execution surface;
- whether duplicate or canonical indicators conflict;
- whether grain, catalog, governance, or semantic-label evidence is missing;
- whether cross-grain KPIs have a deterministic surface;
- which modelling issues should feed agent-readiness blockers or advisory
  cleanup.

## Execution Surfaces

Indicator rows from `search_indicator` and `indicator_inventory` include
response-only execution metadata:

- `indicator_source`: `nova_meta`, `dbt_metric`, or `dbt_semantic_model`
- `execution_surface`: `relation`, `semantic_layer`, or `metadata_only`
- `queryable`: boolean
- `direct_sql_queryable`: boolean
- `queryable_via`: `relation_name`, `metricflow`, or `none`
- `execution_note`: optional guidance

Treat those fields as the first execution gate:

- Relation-backed indicators can be queried through the returned
  `relation_name` when the grain and fields fit the question.
- Semantic-layer-backed indicators require the configured dbt Semantic Layer /
  MetricFlow execution path. They return `queryable: true` and
  `direct_sql_queryable: false`; do not pretend they are relation-backed just
  because they are discoverable in Nova.
- Metadata-only indicators are context. They are not safe query surfaces for
  SQL execution or agent-inferred joins.

## Finding Shape

The report adds:

```json
{
  "agent_modelling_schema_version": "agent_modelling.v1",
  "agent_modelling_finding_count": 3,
  "agent_modelling_findings": [
    {
      "code": "indicator_parent_not_queryable",
      "severity": "blocker",
      "category": "queryability",
      "message": "Indicator `revenue_per_session` is attached to a metadata-only parent.",
      "entities": [],
      "indicators": [],
      "evidence": {},
      "recommendation": "Move the indicator to a queryable dbt model, expose it through dbt Semantic Layer / MetricFlow, or mark the entity as non-analyst-facing.",
      "drill_down_hints": []
    }
  ]
}
```

`summary.agent_modelling` includes total counts by severity plus top codes and
categories. Findings are sorted by severity, category, code, entity, indicator,
and message so repeated runs are stable.

Fixture-backed contract tests cover clean reports, metadata-only KPI risks,
catalog drift, Semantic Layer-backed MetricFlow surfaces, and relation-backed
direct SQL indicators. They enforce `direct_sql_queryable` as execution-surface
metadata: Nova makes the route explicit for agents, but does not execute
MetricFlow or infer semantic-layer SQL.

## Output Examples

A clean project still includes the agent-modelling section so CI and agent
clients can rely on a stable response shape:

```json
{
  "agent_modelling_schema_version": "agent_modelling.v1",
  "agent_modelling_finding_count": 0,
  "agent_modelling_findings": [],
  "summary": {
    "agent_modelling": {
      "total": 0,
      "blockers": 0,
      "high": 0,
      "medium": 0,
      "low": 0,
      "truncated": false,
      "top_codes": [],
      "top_categories": []
    }
  }
}
```

A problematic metadata-only cross-grain KPI returns deterministic evidence and
a remediation path instead of asking the agent to infer a raw fact-table join:

```json
{
  "agent_modelling_finding_count": 1,
  "agent_modelling_findings": [
    {
      "code": "ratio_like_metric_without_deterministic_surface",
      "severity": "blocker",
      "category": "cross_grain_risk",
      "message": "Ratio-like indicator `revenue_per_session` has no deterministic execution surface.",
      "evidence": {
        "execution_surface": "metadata_only",
        "queryable": false,
        "direct_sql_queryable": false,
        "queryable_via": "none"
      },
      "recommendation": "Move the KPI to a queryable dbt relation, dbt Semantic Layer / MetricFlow metric, saved query, or recipe before agents use it for analysis."
    }
  ],
  "summary": {
    "agent_modelling": {
      "total": 1,
      "blockers": 1,
      "high": 0,
      "medium": 0,
      "low": 0,
      "truncated": false,
      "top_codes": [
        {
          "code": "ratio_like_metric_without_deterministic_surface",
          "count": 1
        }
      ],
      "top_categories": [
        {
          "category": "cross_grain_risk",
          "count": 1
        }
      ]
    }
  }
}
```

## Severity Semantics

- `blocker`: the surface is unsafe or ambiguous for agent execution. Examples
  include metadata-only indicators presented as queryable KPIs or duplicate
  canonical indicators with conflicting grains.
- `high`: the model may be queryable, but agent routing is likely to be wrong
  without cleanup. Examples include cross-grain KPIs without semantic artifacts
  or analyst-facing models depending directly on sources.
- `medium`: meaningful ambiguity or governance/catalog drift that should be
  fixed, but usually does not block all agent use.
- `low`: mild routing noise or helper-model ranking issues.

Do not remediate blocker findings by adding plausible metadata alone. Fix the
execution surface, grain, semantic artifact, catalog mismatch, or governance
contract that produced the finding.

## Readiness Integration

`get_agent_readiness` includes the same modelling counts in
`summary.agent_modelling`. Blocker modelling findings become readiness blockers;
high and medium findings become improvements. Count thresholds are also
available:

```json
{
  "modelling": {
    "max_blockers": { "value": 0, "severity": "required" },
    "max_high": { "value": 10, "severity": "advisory" }
  }
}
```

Use advisory thresholds while establishing a baseline. Tighten only the checks
that have proven stable for the project.

## CLI Surface Decision

Nova intentionally does not add a standalone `dbt-nova audit modelling` command
for v1. The canonical audit surface is:

- MCP: `modelling_consistency_report`
- CLI: `dbt-nova tool call modelling_consistency_report`
- CI/readiness gates: `dbt-nova audit agent-readiness`

This keeps the finding contract in one report while it stabilizes. Add a
dedicated CLI audit wrapper later only if teams need a separate JSON/Markdown
report contract, exit-code semantics, or CI artifact shape that cannot be served
by `tool call modelling_consistency_report` plus agent readiness.

## CI Example

Run an advisory modelling report in CI and publish it as an artifact:

```bash
dbt-nova tool call modelling_consistency_report \
  --manifest-path target/manifest.json \
  --params-json '{"resource_types":["model","metric","semantic_model"],"limit":25}' \
  --json > out/modelling-consistency.json
```

By default, the report keeps overlap rows to candidates with score `>= 50.0` and
uses a 10-row section page when `limit` is omitted. Set `"min_score":0` to
restore exhaustive overlap rows for offline review while keeping the same
duplicate-indicator, grain, and agent-modelling finding sections.

Then run readiness with modelling thresholds:

```bash
dbt-nova audit agent-readiness \
  --manifest-path target/manifest.json \
  --thresholds-json '{"modelling":{"max_blockers":{"value":0,"severity":"required"},"max_high":{"value":10,"severity":"advisory"}}}' \
  --report-json-path out/agent-readiness.json \
  --report-md-path out/agent-readiness.md \
  --json
```

Add `--fail-on-blockers` only after the team is ready for blocker findings to
fail CI.

## Remediation Patterns

- Move a metadata-only KPI to a dbt model, MetricFlow metric, saved query, or
  recipe before treating it as executable.
- Prefer one canonical indicator owner per concept and grain.
- Use `compare_grains` when duplicate indicators disagree.
- Use `get_columns` and optional `catalog.json` evidence to resolve missing or
  drifted output fields.
- Use `search.candidates.analyst: false` for helpers that should remain
  discoverable but not be the analyst landing page.
- Keep cross-grain formulas out of vague `derivation` or `composite_metrics`
  metadata. If Nova cannot point to a deterministic execution surface, the KPI
  is context, not a query.

See also:

- [Tools Reference](../api/tools.md#modelling_consistency_report)
- [Agent Readiness Audit](agent-readiness.md)
- [Nova Meta: Metric Guide](nova-meta-metrics.md#cross-grain-kpis)
- [Agent Modelling Finding Contract](../development/agent-modelling-findings-contract.md)
