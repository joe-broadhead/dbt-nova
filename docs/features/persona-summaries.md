# Persona‑Aware Summary Payloads

Status: **Implemented** (standard + full summaries live).

## Why this exists

Nova search/list results currently return more data than most agents need. This adds noise, slows decisions, and increases context usage. Persona‑aware summaries make discovery **fast, minimal, and decision‑ready**.

## Two‑tier model

We standardize response detail to **two levels only**:

- **Standard**: persona‑optimized summary (default)
- **Full**: complete entity payload (same as `get_entity`)

This removes compact/rich variants and makes tool flows predictable.

## Summary payloads (Standard)

### Analyst (query builders)

**Fields:**
- `unique_id`, `name`, `resource_type`
- `relation_name` (or `database` + `schema` when relation_name is missing)
- `description` (truncated)
- `columns_total`
- `primary_key_columns`
- `nova_domains`, `nova_role`
- `persona_payload` (high-signal analyst contract):
  - `focus: "business_discovery"`
  - `business_definition`
  - `key_dimensions`
  - `time_field` (when available)
  - `candidate_measures`
  - `candidate_metrics`
  - `selection_signals`:
    - `has_metric_definition`
    - `has_measure_definition`
    - `has_grain`
    - `has_time_field`
    - `dimension_overlap`
    - `confidence_band` (`low` | `medium` | `high`)
  - `selection_rationale`

### Engineer (code builders)

**Fields:**
- `unique_id`, `name`, `resource_type`
- `original_file_path`, `package_name`, `layer`
- `alias`, `relation_name` (or `database` + `schema`), `materialization`
- `has_compiled_sql`
- `upstream_count`, `downstream_count`
- `tests_summary`, `doc_coverage`
- `persona_payload` (high-signal engineer contract):
  - `focus: "implementation_impact"`
  - `file_path`
  - `blast_radius_count`
  - `change_risk` (`low` | `medium` | `high`)
  - `readiness_band` (`low` | `medium` | `high`)
  - `impacted_tests`
  - `selection_signals`:
    - `upstream_count`
    - `downstream_count`
    - `has_lineage`
    - `tests_total`
    - `documentation_coverage_pct`
    - `has_owner`
    - `has_primary_key`
    - `missing_required_fields`
  - `selection_rationale`
  - `recommended_tools` (minimal next-tool list for implementation flow)
  - `has_compiled_sql`

### Governance (compliance auditors)

**Fields:**
- `unique_id`, `name`, `resource_type`
- `documentation_status`
- `tests_summary`, `doc_coverage`
- `metadata_score`
- `nova_required_missing`
- `nova_governance`, `nova_domains`, `nova_tier`, `nova_canonical`
- `has_nova_meta`, `package_name`
- `persona_payload` (high-signal governance contract):
  - `focus: "governance_assurance"`
  - `policy_risk` (`low` | `medium` | `high` | `critical`)
  - `gate_status` (`pass` | `fail` | `advisory`)
  - `gate_signals` (boolean presence + pass checks)
  - `gate_policy` (resolved policy/thresholds used for this decision)
  - `lineage_health` (entity-level dependency/ref consistency checks)
  - `manifest_health` (global manifest lineage health summary)
  - `missing_governance_fields` (always present)
  - `blocking_reasons` (always present; deterministic labels)
  - `advisory_reasons` (always present; deterministic labels)
  - `quality_warnings` (non-blocking data quality diagnostics)
  - `metadata_grade`, `metadata_score`
  - `documentation_coverage_pct`
  - `test_coverage`
  - `sensitivity`/`pii`/`compliance` when available

### Default (generic)

**Fields:**
- `unique_id`, `name`, `resource_type`
- `package_name`, `alias`
- `relation_name` (or `database` + `schema`), `original_file_path`
- `description` (truncated)

## Canonical representation (no duplicates, low-noise)

**Summaries never repeat the same concept** (no parallel lists of columns/roles/semantics).
Summaries also strip null/empty values so agent context is compact by default.

Full payloads return the **raw dbt manifest entity**, where `columns` is a map keyed by
column name. We do not add duplicate column lists in summaries.

## Computed fields

- `upstream_count`: `parent_map.get(id).len()`
- `downstream_count`: `child_map.get(id).len()`
- `tests_summary`: `{ model_tests, column_tests }`
- `doc_coverage`: `{ columns_documented, columns_total, coverage_pct }`
- `columns_documented`: precomputed during manifest load
- `has_compiled_sql`: true when compiled SQL is present
- `primary_key_columns`: list of column names marked `meta.primary_key: true`
- `layer`: derived from `DBT_NOVA_LAYER_RULES`
- `nova_required_missing`: derived from `DBT_NOVA_GOV_REQUIRED_FIELDS`
- `metadata_score`: `{ overall_score, grade }` computed with governance weights
- `persona_payload.blast_radius_count`: `upstream_count + downstream_count`
- `persona_payload.change_risk`: heuristic from blast radius, tests, and compiled SQL presence
- `persona_payload.readiness_band`: deterministic readiness classification from lineage/tests/docs/owner/PK/required-field signals
- `persona_payload.selection_signals`: compact machine-checkable implementation signals
- `persona_payload.selection_rationale`: short deterministic rationale from failed/weak signals
- `persona_payload.recommended_tools`: minimal next-step tools chosen from observed gaps
- `persona_payload.policy_risk`: heuristic from governance gaps, metadata score, docs/tests, and PII/compliance signals
- `persona_payload.gate_signals`: deterministic governance checks (`required_fields_pass`, `metadata_grade_pass`, `docs_pass`, `tests_pass`, `owner_pass`, `compliance_pass`)
- `persona_payload.gate_policy`: resolved governance gate thresholds and booleans used at runtime
- `persona_payload.lineage_health`: entity-level check for `ref(...)` calls without manifest dependencies
- `persona_payload.manifest_health`: compact global summary from `health.manifest_health`
- `persona_payload.advisory_reasons`: deterministic reasons when gate is configured advisory-only
- `persona_payload.quality_warnings`: non-blocking warnings (`ref_calls_without_dependencies`, `possible_malformed_ref_syntax`)

## Config hooks

### Layer rules

`DBT_NOVA_LAYER_RULES` (JSON array). First match wins.

```json
[{ "layer": "mart", "path_prefix": "models/marts/", "name_prefix": "fct_" }]
```

### Governance required fields

`DBT_NOVA_GOV_REQUIRED_FIELDS` (JSON map).

```json
{ "model": ["nova.domains", "nova.governance", "nova.tier"] }
```

## Truncation

- Summary `description`: ~120 chars
- Measure/metric `description`: ~80 chars
- Full payloads are unmodified.

## Example usage

```json
{"name":"search","arguments":{"query":"conversion rate","persona":"analyst","detail":"standard","limit":10}}
```

## Example payloads (Standard)

### Analyst

```json
{
  "unique_id": "model.jaffle_shop.fct_orders",
  "name": "fct_orders",
  "resource_type": "model",
  "relation_name": "analytics.jaffle_shop.fct_orders",
  "description": "Order-level fact table with completed orders...",
  "columns_total": 42,
  "primary_key_columns": ["order_id"],
  "nova_domains": ["commerce", "finance"],
  "nova_role": "measure",
  "persona_payload": {
    "focus": "business_discovery",
    "business_definition": "Order-level fact table with completed orders...",
    "key_dimensions": ["customer_id", "order_date", "country"],
    "time_field": "order_date",
    "candidate_measures": ["revenue", "orders"],
    "candidate_metrics": ["net_revenue"],
    "selection_signals": {
      "has_metric_definition": true,
      "has_measure_definition": true,
      "has_grain": true,
      "has_time_field": true,
      "dimension_overlap": 2,
      "confidence_band": "high"
    },
    "selection_rationale": "Selection signals: includes metric definitions, includes measure definitions, declares semantic grain, has an explicit time field, 2 query-aligned dimension(s)."
  }
}
```

### Engineer

```json
{
  "unique_id": "model.jaffle_shop.fct_orders",
  "name": "fct_orders",
  "resource_type": "model",
  "original_file_path": "models/marts/fct_orders.sql",
  "package_name": "jaffle_shop",
  "layer": "mart",
  "alias": "fct_orders",
  "relation_name": "analytics.jaffle_shop.fct_orders",
  "materialization": "table",
  "has_compiled_sql": true,
  "upstream_count": 3,
  "downstream_count": 5,
  "tests_summary": {"model_tests": 4, "column_tests": 8},
  "doc_coverage": {"columns_documented": 31, "columns_total": 42, "coverage_pct": 74},
  "persona_payload": {
    "focus": "implementation_impact",
    "file_path": "models/marts/fct_orders.sql",
    "blast_radius_count": 8,
    "change_risk": "medium",
    "readiness_band": "medium",
    "impacted_tests": {"model_tests": 4, "column_tests": 8},
    "selection_signals": {
      "upstream_count": 3,
      "downstream_count": 5,
      "has_lineage": true,
      "tests_total": 12,
      "documentation_coverage_pct": 74,
      "has_owner": true,
      "has_primary_key": true,
      "missing_required_fields": 0
    },
    "selection_rationale": "Implementation signals: moderate blast radius, documentation coverage below target.",
    "recommended_tools": ["get_lineage", "get_columns", "get_impact", "get_metadata_score"],
    "has_compiled_sql": true
  }
}
```

### Governance

```json
{
  "unique_id": "model.jaffle_shop.fct_orders",
  "name": "fct_orders",
  "resource_type": "model",
  "documentation_status": {"has_description": true, "has_doc_blocks": false},
  "tests_summary": {"model_tests": 4, "column_tests": 8},
  "doc_coverage": {"columns_documented": 31, "columns_total": 42, "coverage_pct": 74},
  "metadata_score": {"overall_score": 74, "grade": "C"},
  "nova_required_missing": ["nova.domains", "nova.governance"],
  "nova_governance": {"sensitivity": "internal", "pii": "false", "compliance": ["sox"]},
  "nova_domains": ["finance"],
  "nova_tier": "gold",
  "nova_canonical": true,
  "has_nova_meta": true,
  "package_name": "jaffle_shop",
  "persona_payload": {
    "focus": "governance_assurance",
    "policy_risk": "high",
    "gate_status": "fail",
    "gate_signals": {
      "required_fields_pass": false,
      "metadata_grade_pass": false,
      "docs_pass": false,
      "tests_pass": true,
      "owner_pass": true,
      "compliance_pass": true
    },
    "missing_governance_fields": ["nova.governance"],
    "blocking_reasons": ["missing_required_nova_fields", "metadata_score_below_a_grade"],
    "metadata_grade": "C",
    "metadata_score": 74,
    "documentation_coverage_pct": 74,
    "test_coverage": {"model_tests": 4, "column_tests": 8},
    "sensitivity": "internal",
    "pii": "false",
    "compliance": ["sox"]
  }
}
```
