# Overlap Triage

## What counts as overlap

Treat these as overlap signals:
- multiple entities answering the same recurring business question
- repeated columns across near-peer entities
- repeated measures or metrics across many parents
- similar grains with only minor naming or join differences
- search ambiguity becoming normal for one business concept

## Triage order

1. Identify the shared business concept.
2. Compare candidate grains.
3. Compare repeated columns and repeated indicators.
4. Choose the likely canonical candidate.
5. Separate specialized variants from accidental duplicates.

## Evidence to collect

- entity list
- grain differences
- repeated column families
- repeated indicator families
- canonical metadata or search hints already present
- downstream impact if the overlap were cleaned up
