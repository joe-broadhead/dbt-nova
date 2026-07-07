# Agent Modelling Finding Contract

Status: accepted v1 design for the Deterministic Agent Understandability Layer.

Nova's agent modelling findings answer one narrow question:

> Is this dbt project shaped so analyst agents can discover, select, and query
> the right business surfaces without guessing?

This contract extends `modelling_consistency_report`. It is not a new MCP tool,
a standalone `dbt-nova audit modelling` command, a dbt-project-evaluator clone,
a SQL compiler, or a CI gate by default.

## Compatibility

The v1 contract is additive:

- Existing `modelling_consistency_report` top-level fields keep their current
  names and meanings.
- Existing detail arrays remain paged by `limit` and `offset`.
- Existing `summary.section_counts` keys remain present.
- New fields are added under the report object and `summary`; consumers that
  ignore unknown fields remain compatible.

The new section carries its own version so the whole modelling report does not
need a breaking schema version:

```json
{
  "agent_modelling_schema_version": "agent_modelling.v1",
  "agent_modelling_finding_count": 21,
  "agent_modelling_findings": []
}
```

## Finding Shape

Each finding must be deterministic and serializable as:

```json
{
  "code": "indicator_parent_not_queryable",
  "severity": "blocker",
  "category": "queryability",
  "message": "Indicator `revenue_per_session` is attached to a metadata-only parent.",
  "entities": [
    {
      "unique_id": "analysis.pkg.revenue_note",
      "name": "revenue_note",
      "resource_type": "analysis"
    }
  ],
  "indicators": [
    {
      "indicator_name": "revenue_per_session",
      "indicator_type": "metric",
      "parent_unique_id": "analysis.pkg.revenue_note",
      "source": "nova_meta"
    }
  ],
  "evidence": {
    "execution_surface": "metadata_only",
    "queryable": false,
    "direct_sql_queryable": false,
    "queryable_via": "none"
  },
  "recommendation": "Move the indicator to a queryable dbt model, expose it through dbt Semantic Layer / MetricFlow, or mark the entity as non-analyst-facing.",
  "drill_down_hints": [
    {
      "purpose": "inspect_parent_entity",
      "tool": "get_entity",
      "arguments": {
        "id_or_name": "analysis.pkg.revenue_note",
        "detail": "standard"
      }
    }
  ]
}
```

Rust implementation structs should mirror this shape:

```rust
#[derive(Debug, Clone, Serialize)]
struct AgentModellingFinding {
    code: &'static str,
    severity: AgentModellingSeverity,
    category: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    entities: Vec<ModelingEntityRef>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    indicators: Vec<ModelingIndicatorRef>,
    evidence: JsonValue,
    recommendation: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    drill_down_hints: Vec<JsonValue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum AgentModellingSeverity {
    Blocker,
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Serialize)]
struct ModelingEntityRef {
    unique_id: String,
    name: String,
    resource_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    relation_name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct ModelingIndicatorRef {
    indicator_name: String,
    indicator_type: String,
    parent_unique_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<String>,
}
```

## Severity Semantics

Severities are not subjective. A rule assigns one of:

| Severity | Meaning |
| --- | --- |
| `blocker` | An agent is likely to produce unsafe or non-executable analysis. |
| `high` | An agent can proceed only after disambiguation or manual review. |
| `medium` | Modelling is understandable but noisy, brittle, or likely to cause wrong source choice. |
| `low` | Improvement opportunity; not expected to block safe analysis. |

Severity ordering is `blocker`, `high`, `medium`, then `low`.

## Rule Requirements

Every implemented rule must define:

- `code`: stable snake_case identifier.
- `category`: one of the v1 categories below.
- Trigger: deterministic predicate over manifest, Nova metadata, catalog drift,
  lineage, and already-derived search/index metadata.
- Severity: deterministic mapping from trigger evidence to severity.
- Evidence: machine-readable facts used by the trigger, not prose-only claims.
- Recommendation: one specific remediation that does not fabricate business
  truth.
- Drill-down hints: existing tool calls only, usually `get_entity`,
  `get_columns`, `compare_grains`, `find_entity_overlap`, or
  `search_indicator`.

Rules must not require arbitrary SQL-shape parsing, LLM judgement, warehouse
queries, or generated SQL. Default-off SQL-shape checks can be added later only
behind explicit config.

## Bounded Summary

`modelling_consistency_report.summary` adds:

```json
{
  "section_counts": {
    "overlap_candidates": 18,
    "duplicate_indicators": 6,
    "canonical_indicator_conflicts": 2,
    "entities_with_multiple_grain_variants": 4,
    "agent_modelling_findings": 21
  },
  "agent_modelling": {
    "total": 21,
    "blockers": 2,
    "high": 7,
    "medium": 10,
    "low": 2,
    "truncated": false,
    "top_codes": [
      {"code": "duplicate_canonical_indicator", "count": 2},
      {"code": "indicator_parent_not_queryable", "count": 2}
    ],
    "top_categories": [
      {"category": "grain_safety", "count": 8},
      {"category": "queryability", "count": 4}
    ]
  }
}
```

Bounds:

- `agent_modelling_findings` defaults to at most 100 findings.
- `top_codes` and `top_categories` contain at most 5 rows each.
- `entities` and `indicators` contain at most 8 refs per finding.
- Evidence arrays contain at most 10 examples unless a rule documents a smaller
  cap.
- A finding emits at most 3 drill-down hints.

If findings are truncated, keep the most severe findings first and set
`summary.agent_modelling.truncated` to `true`.

Sort findings by severity rank, `category`, `code`, first entity `unique_id`,
first indicator name, and `message`. Sort summary code/category buckets by
count descending, then name ascending.

## V1 Categories And Rules

### Indicator Resolution

| Code | Severity | Trigger |
| --- | --- | --- |
| `duplicate_canonical_indicator` | `blocker` when canonical parents have inconsistent grains, otherwise `high` | Same indicator has more than one canonical parent. |
| `duplicate_indicator_without_canonical_parent` | `medium` | Same indicator has multiple parents and no canonical parent. |
| `semantic_label_collision` | `high` when multiple canonical refs share a label, otherwise `medium` | Normalized indicator name or synonym maps to multiple indicator refs. |

### Queryability

| Code | Severity | Trigger |
| --- | --- | --- |
| `indicator_parent_not_queryable` | `blocker` | A Nova metric or measure is attached to a parent whose execution surface is `metadata_only`. |
| `metric_output_column_missing` | `medium` | A relation-backed non-template metric has no output column matching the metric name. |
| `metric_grain_field_not_in_output` | `high` | A relation-backed metric declares grain fields that are absent from output columns. |

### Grain Safety

| Code | Severity | Trigger |
| --- | --- | --- |
| `metric_missing_time_field` | `high` | A metric lacks an effective `grain.time_field`. |
| `canonical_entity_missing_primary_key` | `high` | A canonical model has no `meta.nova.grain.primary_key` and no primary-key column. |
| `entity_multiple_grain_variants` | `high` for analyst-facing entities, otherwise `medium` | Existing modelling logic finds more than one grain variant for an entity. |

### Semantic Artifact Integrity

| Code | Severity | Trigger |
| --- | --- | --- |
| `semantic_metric_unresolved_measure_ref` | `high` | A dbt metric references a measure that is absent from all semantic models in the manifest. |
| `semantic_model_missing_primary_entity` | `medium` | A semantic model with measures derives no primary key from semantic entities. |
| `semantic_model_missing_time_dimension` | `high` | A semantic model with measures derives no time field. |

### Catalog Reality

| Code | Severity | Trigger |
| --- | --- | --- |
| `catalog_type_drift_on_indicator_field` | `medium` | An indicator field has catalog type drift. |
| `catalog_missing_indicator_field` | `high` | An indicator field exists in the manifest but is missing from catalog reality. |
| `catalog_only_candidate_measure_column` | `low` | A catalog-only numeric column looks measure-like on an analyst-facing entity. |

### Cross-Grain And Multi-Fact Risk

| Code | Severity | Trigger |
| --- | --- | --- |
| `multi_fact_metric_model` | `high` when fact-like parents have different grain signatures, otherwise `medium` | An entity exposing indicators has two or more direct fact-like parents. |
| `ratio_like_metric_without_deterministic_surface` | `blocker` | A ratio-like metric lives on a metadata-only parent. |
| `cross_grain_kpi_without_semantic_artifact` | `high` | A ratio-like metric combines multiple fact inputs and no semantic metric with the same label exists. |

### Layering

| Code | Severity | Trigger |
| --- | --- | --- |
| `analyst_facing_model_depends_on_source` | `high` | An analyst-facing entity has a direct source parent. |
| `non_mart_model_exposes_canonical_indicator` | `medium` | A staging or intermediate model exposes a canonical indicator. |
| `helper_ranked_as_analyst_candidate` | `low` | A staging or intermediate model has Nova metadata and is not explicitly de-ranked for analysts. |
| `agent_surface_too_many_parents` | `medium` | An analyst-facing entity has at least `too_many_parents_threshold` direct parents. |

### Column Semantics

| Code | Severity | Trigger |
| --- | --- | --- |
| `column_semantic_role_conflict` | `medium` | The same semantic type appears with multiple roles. |
| `column_name_semantic_drift` | `medium` | The same column name has multiple semantic types on analyst-facing entities. |

### Governance

| Code | Severity | Trigger |
| --- | --- | --- |
| `analyst_surface_missing_governance` | `medium` | An analyst-facing entity lacks Nova governance metadata. |
| `pii_like_column_without_governance` | `medium` | A PII-like column appears on an analyst-facing entity without column or entity governance. |

## Advisory Or Deferred Rules

The following are not required for v1 default-on behavior:

- Arbitrary SQL-shape parsing. Keep it default-off until validated against
  realistic SQL and adapter differences.
- `source_fanout_without_canonical_staging`. This is promising but needs
  source-fanout thresholds tuned against fixtures before it should emit default
  findings.
- Warehouse-query-backed checks. V1 uses manifest plus optional `catalog.json`
  only.
- Standalone modelling-audit CLI wrapper. `modelling_consistency_report`
  reports findings through MCP and `dbt-nova tool call`; agent readiness owns
  report-file and blocker exit-code semantics for v1.

## Readiness Mapping

When `get_agent_readiness` consumes these findings:

- `blocker` becomes a readiness blocking finding.
- `high` and `medium` become readiness improvement findings.
- `low` stays in modelling report summary and does not affect readiness by
  default.

Readiness should carry bounded modelling summary counts and at most the top
severe findings. It must not generate suggested metadata patches that invent
unknown business truth.
