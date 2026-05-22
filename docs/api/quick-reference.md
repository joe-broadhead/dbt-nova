# Tools Quick Reference

One-page cheatsheet for all dbt-nova MCP tools.

## Discovery

| Tool | Purpose | Key Parameters |
|------|---------|----------------|
| `search` | Full-text and hybrid search | `query`, `persona`, `resource_types`, `detail`, `include_highlights`, `include_sql` |
| `get_entity` | Fetch single entity by ID or name | `id_or_name`, `resource_type`, `detail` |
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
| `get_undocumented` | Find entities missing descriptions | `resource_type`, `include_columns`, `package`, `path_prefix` |
| `search_recipes` | Find analysis recipe templates + parameter contracts | `topic`, `query`, `include_queries`, `limit`, `offset` |
| `get_recipe` | Load a recipe, SQL, and parameter requirements | `recipe_id`, `include_sql`, `include_queries`, `parameters`, `placeholder_types` |
| `run_recipe` | Execute recipe queries with preflight parameter validation | `recipe_id`, `query_names`, `query_indexes`, `stop_on_failure`, `parameters`, `placeholder_types`, `sql_parameter_types` |

## Validation

| Tool | Purpose | Key Parameters |
|------|---------|----------------|
| `validate_dag` | Check for cycles/orphans | `detail` (full \| summary) |

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
| `execute_sql` | Run SQL against configured provider (`databricks`, `bigquery`, `snowflake`, `duckdb`) | `statement`, `row_limit`, `byte_limit`, `max_poll_seconds`, `parameters` |

## Operations

| Tool | Purpose | Key Parameters |
|------|---------|----------------|
| `health` | Readiness and status check | (none) |
| `reload_manifest` | Reload manifest and rebuild indexes | `manifest_uri`, `manifest_path` (optional) |

---

## Common Patterns

### Fast Discovery
```json
{"name":"search","arguments":{"query":"customer","persona":"analyst","detail":"standard","include_highlights":true,"limit":10}}
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
