# Canonical Model Selection

## Goal

Choose the one entity that should act as the preferred analyst-facing execution model for a repeated business concept.

## Selection rubric

Prefer the entity that best satisfies:
- the correct business grain
- stable primary keys
- explicit time field
- required breakdown dimensions
- reusable canonical measures or metrics
- acceptable tests and documentation
- minimal reliance on downstream workaround logic

## Negative signals

Be cautious when the candidate:
- is helper, staging, or operational by design
- has partial or ambiguous grain
- exists only to support one narrow downstream consumer
- duplicates indicators that already live cleanly elsewhere
- requires hidden filter assumptions to answer common questions

## Decision output

Every canonical selection should state:
- why this entity is canonical
- why nearby candidates are not
- what helper models remain legitimate
- what search and metadata consequences follow
