# Model Architect Tool Recipes

## Broad consistency baseline

**When:** Starting a project-shape review.
**Why:** Surface duplicate indicators, overlap, and grain drift before narrowing.

```json
{"name":"modelling_consistency_report","arguments":{"resource_types":["model"],"limit":50,"offset":0}}
```

## Overlap clustering

**When:** You already have one likely anchor entity or a narrowed scope.
**Why:** Build a cluster around a repeated business concept.

```json
{"name":"find_entity_overlap","arguments":{"id_or_name":"model.package.model_name","resource_types":["model"],"limit":25,"offset":0}}
```

## Grain comparison

**When:** Two near-peer entities both look viable.
**Why:** Confirm whether they actually align at the same grain.

```json
{"name":"compare_grains","arguments":{"entity1":"model.package.model_name","entity2":"model.package.other_model"}}
```

## Semantic inventory

**When:** Canonicality depends on repeated indicators or repeated columns.
**Why:** Compare the semantic surface before choosing the canonical model.

```json
{"name":"indicator_inventory","arguments":{"resource_types":["model"],"indicator_types":["measure","metric"],"canonical_only":false,"limit":100,"offset":0}}
```

```json
{"name":"column_inventory","arguments":{"resource_types":["model"],"annotated_only":true,"limit":200,"offset":0}}
```
