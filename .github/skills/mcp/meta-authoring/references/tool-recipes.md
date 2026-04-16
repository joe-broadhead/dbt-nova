# Metadata Authoring Tool Recipes

## Repeated indicator inventory

**When:** Authoring a measure or metric for a business term that already appears elsewhere.
**Why:** Avoid adding a new canonical definition blindly.

```json
{"name":"indicator_inventory","arguments":{"resource_types":["model"],"indicator_types":["measure","metric"],"canonical_only":false,"limit":100,"offset":0}}
```

## Repeated column semantics

**When:** Authoring or refining column-level metadata.
**Why:** Check whether the project already encodes the same column concept elsewhere.

```json
{"name":"search_columns","arguments":{"query":"market","resource_types":["model"],"limit":20,"offset":0}}
```

```json
{"name":"column_inventory","arguments":{"resource_types":["model"],"annotated_only":true,"limit":100,"offset":0}}
```

## Canonical placement check

**When:** The concept spans multiple entities and the canonical parent is unclear.
**Why:** Make the placement decision explicit before you mark anything canonical.

```json
{"name":"find_entity_overlap","arguments":{"id_or_name":"model.package.model_name","resource_types":["model"],"limit":25,"offset":0}}
```

```json
{"name":"compare_grains","arguments":{"entity1":"model.package.model_name","entity2":"model.package.other_model"}}
```

## Post-authoring verification

**When:** Metadata edits are loaded and you need to confirm routing behavior.
**Why:** Validate the surfaced definition, not just the raw YAML.

```json
{"name":"search_indicator","arguments":{"query":"gross merchandise value","resource_types":["model"],"limit":10}}
```

```json
{"name":"get_entity","arguments":{"id_or_name":"model.package.model_name","detail":"standard"}}
```
