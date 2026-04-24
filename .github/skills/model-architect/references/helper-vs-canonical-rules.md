# Helper vs Canonical Rules

## Canonical model

The canonical model should:
- represent the preferred analyst-facing execution entity
- have explicit grain and time field
- carry the reusable semantic contract for repeated business questions
- support common breakdowns without hidden assumptions
- be stable enough for downstream users to depend on

## Helper model

A helper model may:
- simplify joins
- stage inputs
- support engineering workflows
- provide specialized or intermediate transformations
- preserve a performant pre-aggregation when its grain is explicit

A helper model should not dominate analyst discovery unless it truly is the best execution model.

## Specialized model

A specialized mart or reporting model may remain separate when it:
- has a distinct business scope
- has a distinct aggregation grain
- protects performance for repeated reporting
- adds domain-specific transformations that do not belong in the canonical fact

## Demotion rule

If a helper model is discoverable but should not rank first for analysts:
- keep it searchable for engineers
- avoid marking it canonical
- encode audience exceptions through metadata where appropriate

## Promotion rule

Promote a helper model to canonical only if:
- its grain is truly the correct analyst-facing grain
- it owns the reusable semantics cleanly
- nearby candidates are objectively worse execution models
- downstream migration can be made safely
