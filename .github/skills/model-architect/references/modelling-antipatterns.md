# Modelling Anti-Patterns

## Repeated concept across many peers

Signals:
- the same business term appears in many models
- the same columns repeat across near-peer entities
- search ambiguity becomes normal
- canonical flags appear on several competing definitions without a grain/domain explanation

## Helper model acting as analyst surface

Signals:
- analysts land on intermediate or ops tables first
- helper models carry duplicated KPIs or thin semantic wrappers
- canonical dataset is unclear
- search-candidate metadata is missing or contradictory

## Grain confusion

Signals:
- multiple entities appear to answer the same question at incompatible grains
- a rate or KPI can be calculated in multiple places with different aggregation assumptions
- a row-level model and an aggregate mart both claim to be the default without explaining their boundary

## Semantic duplication

Signals:
- identical or nearly identical measures exist across multiple entities
- canonical flags are scattered or contradictory
- synonyms pull search toward many siblings without a clear winner
- project-wide consistency reports show duplicate indicators with inconsistent grains

## Metadata-only cross-grain KPI

Signals:
- a ratio or KPI combines facts at incompatible grains
- the KPI exists only as `meta.nova` text, a derivation note, or a proposed
  composite metadata graph
- no dbt relation, configured Semantic Layer metric, saved query, or recipe owns
  the executable shape
- modelling findings mark the indicator as non-queryable or missing a
  deterministic surface

## Semantic-layer row treated as relation SQL

Signals:
- `search_indicator` returns `execution_surface: "semantic_layer"` but the plan
  tries to query `relation_name` directly despite `direct_sql_queryable: false`
- MetricFlow measure references are not resolved before using the metric
- the refactor plan mixes Semantic Layer ownership with dbt relation ownership
  without an explicit boundary

## Naming drift without contract drift control

Signals:
- same columns with different names across related entities
- one concept uses multiple inconsistent business labels
- downstream dashboard logic compensates for model inconsistency

## Unsafe consolidation

Signals:
- a high-impact canonical model is changed without downstream inventory
- column-level lineage is ignored for renamed or moved fields
- no compatibility view, alias, or deprecation window is planned
- validation only checks the new model and not dependent consumers
