# Modelling Anti-Patterns

## Repeated concept across many peers

Signals:
- the same business term appears in many models
- the same columns repeat across near-peer entities
- search ambiguity becomes normal

## Helper model acting as analyst surface

Signals:
- analysts land on intermediate or ops tables first
- helper models carry duplicated KPIs or thin semantic wrappers
- canonical dataset is unclear

## Grain confusion

Signals:
- multiple entities appear to answer the same question at incompatible grains
- a rate or KPI can be calculated in multiple places with different aggregation assumptions

## Semantic duplication

Signals:
- identical or nearly identical measures exist across multiple entities
- canonical flags are scattered or contradictory
- synonyms pull search toward many siblings without a clear winner

## Naming drift without contract drift control

Signals:
- same columns with different names across related entities
- one concept uses multiple inconsistent business labels
- downstream dashboard logic compensates for model inconsistency
