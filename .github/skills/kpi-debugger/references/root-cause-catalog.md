# Root Cause Catalog

## Definition mismatch

Signals:
- stakeholder term differs from canonical indicator name
- alternate source uses a different numerator / denominator
- metric card or dashboard rederived the KPI differently
- local-currency, euro, and constant-rate versions are mixed

## Filter mismatch

Signals:
- geography or channel values were assumed instead of validated
- one source applies default filters that the other does not
- one source uses a broader or narrower segment definition
- filter labels map to different physical columns across entities

## Time-window mismatch

Signals:
- sources use different week boundaries
- comparison windows are offset differently
- data freshness timestamps differ
- weekly comparisons use date-aligned rather than weekday-aligned windows
- month/year comparisons use rolling windows rather than calendar windows

## Grain mismatch

Signals:
- one source is pre-aggregated
- one source rolls up a rate incorrectly
- joins or duplicate keys inflate the denominator or numerator
- an item-level entity is compared with an order-level or daily aggregate entity

## Lineage or freshness issue

Signals:
- upstream source changed recently
- one branch of lineage is stale or failed
- a key input column changed meaning or type
- only the current period is partial while the comparison period is complete
- relation preflight, permissions, or warehouse state prevents reproduction

## Search / routing issue

Signals:
- the investigation started from a non-canonical entity
- indicator resolution chose the wrong parent
- similarly named KPIs exist across multiple domains
- alternate entities have overlapping labels but different grain or business purpose
