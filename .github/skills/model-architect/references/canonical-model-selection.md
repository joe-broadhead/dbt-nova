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
- manageable downstream migration path

## Negative signals

Be cautious when the candidate:
- is helper, staging, or operational by design
- has partial or ambiguous grain
- exists only to support one narrow downstream consumer
- duplicates indicators that already live cleanly elsewhere
- requires hidden filter assumptions to answer common questions
- has a large downstream blast radius without compatibility shims
- is canonical only by name, path, or mart/base prefix

## Decision output

Every canonical selection should state:
- why this entity is canonical
- why nearby candidates are not
- what helper, specialized mart, and reporting models remain legitimate
- what search and metadata consequences follow
- what migration constraints or compatibility layers are needed
