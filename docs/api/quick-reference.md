# Tools Quick Reference

One-page cheatsheet for all 43 dbt-nova MCP tools.

## Discovery

| Tool | Purpose | Key Parameters |
|------|---------|----------------|
| `search` | Full-text and hybrid search | `query`, `persona`, `resource_types`, `detail`, `include_highlights`, `include_sql` |
| `search_indicator` | Resolve Nova measures/metrics to execution parents | `query`, `indicator_types`, `resource_types`, `detail`, `group_mode`, `limit` |
| `indicator_inventory` | List Nova measures and metrics deterministically | `indicator_types`, `resource_types`, `canonical_only`, `limit`, `offset` |
| `search_columns` | Search columns by names and semantic hints | `query`, `resource_types`, `roles`, `semantic_types`, `limit`, `offset` |
| `column_inventory` | List columns with semantic context | `resource_types`, `roles`, `semantic_types`, `limit`, `offset` |
| `get_entity` | Fetch single entity by ID or name | `id_or_name` (`unique_id` alias), `resource_type`, `detail` |
| `list_entities` | List entities by type with filters | `resource_type`, `package`, `tags`, `detail` |
| `batch_get_entities` | Retrieve multiple entities at once | `unique_ids`, `detail` |
| `find_by_path` | Find entities by file path glob | `path_pattern`, `resource_types`, `detail` |

## Context

| Tool | Purpose | Key Parameters |
|------|---------|----------------|
| `get_context` | One-shot context bundle (lineage, columns, tests, docs) | `id_or_name`, `include_columns`, `include_upstream`, `include_tests`, `include_sql`, `include_docs`, `lineage_depth`, `context_mode` |

## Lineage

| Tool | Purpose | Key Parameters |
|------|---------|----------------|
| `get_lineage` | Traverse entity lineage | `id_or_name`, `direction`, `depth`, `resource_types`, `detail` |
| `get_impact` | Blast-radius estimate | `id_or_name` |
| `get_column_lineage` | Trace column upstream/downstream | `id_or_name`, `resource_type`, `column_name`, `direction`, `confidence` |

## Code & Schema

| Tool | Purpose | Key Parameters |
|------|---------|----------------|
| `get_columns` | Inspect column names, types, metadata | `id_or_name` |
| `get_sql` | Return raw or compiled SQL | `id_or_name`, `compiled` |
| `diff_entities` | Compare two entities side-by-side | `entity1`, `entity2`, `entity1_resource_type`, `entity2_resource_type`, `compare_fields` |

## Analysis

| Tool | Purpose | Key Parameters |
|------|---------|----------------|
| `get_test_coverage` | Schema/data test coverage | `id_or_name`, `resource_type`, `include_full`, `columns_limit` |
| `get_metadata_score` | Metadata quality score | `id_or_name`, `persona`, `scope` |
| `get_metadata_audit` | Metadata audit report and gate | `selection_mode`, `changed_files_json`, `entity_ids_json`, `thresholds_json` |
| `get_agent_readiness` | Agent-readiness report | `personas_json`, `thresholds_json`, `eval_gate_json` |
| `get_undocumented` | Find entities missing descriptions | `resource_type`, `include_columns`, `package`, `path_prefix` |
| `search_recipes` | Find analysis recipe templates + parameter contracts | `topic`, `query`, `include_queries`, `limit`, `offset` |
| `get_recipe` | Load a recipe, SQL, and parameter requirements | `recipe_id`, `include_sql`, `include_queries`, `parameters`, `placeholder_types` |
| `run_recipe` | Execute recipe queries with preflight parameter validation | `recipe_id`, `query_names`, `query_indexes`, `stop_on_failure`, `parameters`, `placeholder_types`, `sql_parameter_types` |

## Modeling

| Tool | Purpose | Key Parameters |
|------|---------|----------------|
| `compare_grains` | Compare grain metadata between two entities | `entity1`, `entity2`, `entity1_resource_type`, `entity2_resource_type` |
| `find_entity_overlap` | Detect overlapping entities using semantic evidence | `resource_types`, `package`, `min_overlap_score`, `limit`, `offset` |
| `modelling_consistency_report` | Audit duplicate indicators, conflicts, and grain drift | `resource_types`, `package`, `limit`, `offset` |

## Validation

| Tool | Purpose | Key Parameters |
|------|---------|----------------|
| `validate_dag` | Check for cycles/orphans | `detail` (full \| summary) |
| `validate_nova_meta` | Validate project YAML `meta.nova` | `project_dir`, `paths`, `resource_kind`, `resource_name`, `column` |
| `validate_eval_suite` | Validate eval suite YAML/JSON | `suite` |
| `get_eval_gate` | Latest eval gate report | `suite` |
| `get_eval_history` | Filtered eval telemetry rows | `suite`, `since` |
| `run_eval` | Run deterministic bridge evals against loaded manifest | `suite`, `output_dir`, `telemetry`, `case_ids`, `fail_under` |
| `init_eval_suite` | Write a starter eval suite | `persona`, `out`, `force` |
| `run_agent_eval` | Run provider-backed agent evals | `suite`, `provider`, `manifest_path`, `output_dir`, `case_ids`, `timeout_secs`, `fail_under` |

## Metadata Inventory

| Tool | Purpose | Key Parameters |
|------|---------|----------------|
| `show_metadata` | Project overview with entity counts | (none) |
| `list_tags` | All tags with counts | (none) |
| `list_packages` | All packages with counts | (none) |
| `list_databases` | All database.schema combinations | (none) |

## Warehouse

| Tool | Purpose | Key Parameters |
|------|---------|----------------|
| `execute_sql` | Run SQL against configured provider (`databricks`, `bigquery`, `snowflake`, `duckdb`) | `statement` (`sql` alias), `row_limit`, `byte_limit`, `max_poll_seconds`, `parameters` |

## Operations

| Tool | Purpose | Key Parameters |
|------|---------|----------------|
| `health` | Readiness and status check | (none) |
| `reload_manifest` | Reload manifest and rebuild indexes | `manifest_uri`, `manifest_path` (optional) |
| `warm_manifest` | Warm semantic caches for current manifest source | `vector`, `sparse`, `reranker`, `force` |

---

## Common Patterns

### Fast Discovery
```json
{"name":"search_indicator","arguments":{"query":"conversion rate checkout","persona":"analyst","resource_types":["model"],"detail":"compact","group_mode":"top","limit":3}}
```

### Quick Triage
```json
{"name":"get_context","arguments":{"id_or_name":"model.pkg.name","include_columns":true,"include_tests":true,"include_upstream":true,"include_sql":false,"include_docs":false}}
```

### Impact Assessment
```json
{"name":"get_impact","arguments":{"id_or_name":"model.pkg.name"}}
```

### Compliance Scan
```json
{"name":"search","arguments":{"query":"nova_pii:true","persona":"governance","resource_types":["model"]}}
```

---

## See Also

- [Tools Reference](tools.md) - Full documentation for each tool
- [Response Format](response-format.md) - Understanding API responses
- [Personas](../personas/overview.md) - Persona-specific workflows
