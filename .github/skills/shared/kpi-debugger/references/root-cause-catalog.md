# Root Cause Catalog

## Definition mismatch

Signals:
- stakeholder term differs from canonical indicator name
- alternate source uses a different numerator / denominator
- metric card or dashboard rederived the KPI differently

## Filter mismatch

Signals:
- geography or channel values were assumed instead of validated
- one source applies default filters that the other does not
- one source uses a broader or narrower segment definition

## Time-window mismatch

Signals:
- sources use different week boundaries
- comparison windows are offset differently
- data freshness timestamps differ

## Grain mismatch

Signals:
- one source is pre-aggregated
- one source rolls up a rate incorrectly
- joins or duplicate keys inflate the denominator or numerator

## Lineage or freshness issue

Signals:
- upstream source changed recently
- one branch of lineage is stale or failed
- a key input column changed meaning or type

## Search / routing issue

Signals:
- the investigation started from a non-canonical entity
- indicator resolution chose the wrong parent
- similarly named KPIs exist across multiple domains
