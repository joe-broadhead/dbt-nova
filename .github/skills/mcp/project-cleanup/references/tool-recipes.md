# Project Cleanup Tool Recipes

## Broad cleanup baseline

**When:** Starting a project-wide cleanup pass.
**Why:** Surface the highest-signal overlap and consistency problems first.

```json
{"name":"modelling_consistency_report","arguments":{"resource_types":["model"],"limit":50,"offset":0}}
```

## Overlap cluster

**When:** One model or source already looks suspiciously duplicated.
**Why:** Collect the nearby peer set before deciding on cleanup priority.

```json
{"name":"find_entity_overlap","arguments":{"id_or_name":"model.package.model_name","resource_types":["model"],"limit":25,"offset":0}}
```

## Compare entity pair

**When:** You need to decide whether two entities are accidental duplicates or legitimate variants.
**Why:** Separate grain problems from naming problems.

```json
{"name":"compare_grains","arguments":{"entity1":"model.package.model_name","entity2":"model.package.other_model"}}
```

```json
{"name":"diff_entities","arguments":{"entity1":"model.package.model_name","entity2":"model.package.other_model"}}
```

## Repeated-term inventory

**When:** Cleanup is driven by repeated KPIs, dimensions, or business language.
**Why:** See the duplicated surface directly instead of inferring it from one search result.

```json
{"name":"indicator_inventory","arguments":{"resource_types":["model"],"indicator_types":["measure","metric"],"canonical_only":false,"limit":100,"offset":0}}
```

```json
{"name":"search_columns","arguments":{"query":"market","resource_types":["model"],"limit":50,"offset":0}}
```
