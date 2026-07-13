# Column Lineage (Matching Algorithm)

Column lineage traces the upstream or downstream origin of a column. Nova uses a layered
matching strategy to handle SQL aliases, naming variations, and transformation chains.

## Matching strategies (in order)

1) **Exact match**
- Column name matches directly between nodes.

2) **SQL alias match**
- Column appears in SQL with an alias (e.g., `sum(amount) as revenue`).

3) **SQL proximity match**
- Column appears near a known field in SQL and is treated as a probable match.

4) **Suffix match**
- `customer_id` matches `stg__customer_id` or `dim_customer_id` when suffix rules apply.

5) **Prefix match**
- `id` matches `order_id` or `customer_id` where prefixes convey context.

6) **Normalized match**
- Case/underscore normalization is applied before comparison.

7) **Levenshtein distance**
- Fuzzy matching for small spelling variants; controlled by
  `DBT_NOVA_LEVENSHTEIN_THRESHOLD` and `DBT_NOVA_MIN_LEVENSHTEIN_LENGTH`.

## Confidence levels

The `confidence` parameter controls which matches are accepted:

- **high**: strict (exact/alias only)
- **medium**: includes SQL proximity and normalized matches
- **low**: includes fuzzy/Levenshtein matches

## Safety limits

To avoid runaway traversal:

- `DBT_NOVA_MAX_LINEAGE_RESULTS` caps column lineage results.
- `DBT_NOVA_COLUMN_LINEAGE_MAX_CANDIDATES` caps match candidates.
- `DBT_NOVA_COLUMN_LINEAGE_MAX_DEPTH` caps traversal depth (requested `depth` is clamped).

## Recommended usage

- Use **high** confidence for production audits.
- Use **medium** for analyst exploration.
- Use **low** for discovery and debugging.

Column lineage matching is independent from entity lineage reconstruction. Nova
uses manifest dependency metadata as the DAG source of truth and applies SQL
alias/proximity matching only for column-level mapping.

## Unresolved derived columns

Derived and aggregated columns do not always map cleanly to an upstream column.
When upstream lineage is empty but the model has dependencies, Nova keeps the
existing `lineage_status` values and additively returns definition evidence when
available:

- `definition`: the outer select-list expression for the requested output
  column, from compiled SQL with raw SQL as fallback.
- `definition_source`: `compiled_sql` or `raw_sql`.
- `definition_confidence`: `exact` when parsed from SQL, `best_effort` when
  recovered from a bounded text fallback.
- `referenced_columns`: identifiers found inside the expression, annotated with
  direct upstream entities that expose a same-named column.

This is evidence for agent debugging, not a replacement for a full warehouse
query plan. Wildcard expansion and dialect-specific optimizer behavior remain
outside the column-lineage contract.

## Column reference evidence

Set `include_references: true` to add metadata-plane references for rename and
blast-radius planning. The `references` array can include:

- `nova_grain_dimension` for `meta.nova.grain.dimensions` matches.
- `nova_measure_expression` and `nova_metric_expression` for tokenized Nova
  measure or metric expression matches.
- `test` for dbt manifest tests whose `column_name` or structured kwargs match.
- `recipe_sql` for manifest-backed recipe analysis SQL under `analyses/recipes`
  (or `DBT_NOVA_RECIPES_DIR`).

Metadata and test references are exact or tokenized metadata matches. Recipe SQL
references are word-boundary textual matches and are labeled
`match: "textual"` in the record detail.

## Nested field matches

When a column is stored inside a struct (e.g., `event_properties._revenue`), the response includes
`explanation.details` fields to make the match explicit:

- `struct_column`: the struct column name (e.g., `event_properties`)
- `field`: the nested field name (e.g., `_revenue`)
- `field_path`: a convenience path (e.g., `event_properties._revenue`)

---

## See Also

- [Tools Reference](../api/tools.md) - `get_column_lineage`
- [Configuration Reference](../configuration/reference.md) - Lineage limits
