# Overlap Triage

## What counts as overlap

Treat these as overlap signals:
- multiple entities answering the same recurring business question
- repeated columns across near-peer entities
- repeated measures or metrics across many parents
- similar grains with only minor naming or join differences
- search ambiguity becoming normal for one business concept
- project-wide consistency reports showing canonical conflicts or duplicate indicators

## Triage order

1. Identify the shared business concept.
2. Compare candidate grains.
3. Compare repeated columns and repeated indicators.
4. Classify the overlap type.
5. Choose the likely canonical candidate when the overlap is semantic.
6. Separate specialized variants, staging partitions, and reporting derivations from accidental duplicates.
7. Quantify downstream impact before setting priority.

## Evidence to collect

- entity list
- grain differences
- repeated column families
- repeated indicator families
- canonical metadata or search hints already present
- downstream impact if the overlap were cleaned up
- explicit reason for cleanup, deferral, or no action
