# Cleanup Prioritization

## Priority order

### P0

Structural problems that actively break trust or discovery:
- wrong canonical target for a major business concept
- incompatible grains treated as interchangeable
- severe duplication causing the wrong KPI source to be used repeatedly
- duplicate canonical indicators with no clear grain or domain boundary

### P1

Problems that materially increase downstream maintenance:
- repeated near-peer entities for one concept
- repeated filter / dimension columns with inconsistent naming
- duplicated measures across many siblings
- high-impact helper or detail models carrying duplicate semantics

### P2

Problems that create moderate ambiguity:
- helper models that outrank canonical models
- missing semantic hints on otherwise correct canonical entities
- partially duplicated specialized variants
- repeated staging/source-family tables with clear partitioning but weak ownership or metadata

### P3

Low-signal cleanup:
- cosmetic naming drift that does not affect discovery or downstream usage
- overlap that is real but intentionally specialized and well-bounded
- duplicate country/source staging tables with no downstream consumers and no current consolidation path

## Prioritization rule

Prioritize the cleanup item that reduces the most downstream ambiguity with the least semantic risk.

Demote cleanup when overlap is intentional, downstream impact is near zero, or consolidation would require a new architecture decision that is outside the frozen scope.
