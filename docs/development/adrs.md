# Architecture Decision Records

These ADRs record current v0.0.x hardening decisions. They are intentionally
small: they should keep future changes aligned without turning Nova into a heavy
process project.

## ADR-001: Single-Manifest Metadata Bridge

**Status:** Accepted

Nova's core input is one dbt `manifest.json`, optionally enriched with
`catalog.json`, `meta.nova`, and prebuilt Nova artifacts derived from that same
manifest scope.

Nova should make manifest metadata searchable, inspectable, and explainable for
agents. It should not become a multi-project semantic registry, BI catalog
clone, or warehouse-specific source of truth. Cross-project or multi-tenant
coordination belongs in a caller-owned orchestration layer until a concrete
manifest-bound contract proves otherwise.

## ADR-002: Proxy-First Hosted Security

**Status:** Accepted

`streamable_http` has no built-in authentication. Non-loopback binds require an
explicit proxy acknowledgement and must sit behind an authenticating proxy or
platform auth layer.

Nova may provide safer defaults, denylist presets, host checks, body limits,
probes, and operator checklists, but it should not pretend those are identity or
authorization. Native identity work belongs in a separate, explicit milestone.

## ADR-003: Metadata-Bridge Product Boundary

**Status:** Accepted

Nova is model, indicator, lineage, readiness, and metadata evidence for agents.
It helps agents decide what can be queried, what needs context, and what should
be escalated.

dbt Semantic Layer / MetricFlow artifacts in a manifest are optional evidence
rows. Nova may derive indicator metadata from them and tell agents to use the
externally configured MetricFlow/dbt Semantic Layer path, but Nova SQL must not
pretend those rows are relation-backed execution surfaces.

## ADR-004: Runtime Presets Before Overrides

**Status:** Accepted

Runtime presets exist to encode conservative deployment postures:
`local-dev`, `ci-audit`, `hosted-discovery`, and `hosted-sql-trusted`.

Presets apply before environment variables and CLI overrides. This keeps them
useful as safe starting points while preserving operator control. `config
validate` is the source of truth for the effective runtime posture.

## ADR-005: Shared-Port Metrics With Privacy-Safe Labels

**Status:** Accepted

Hosted HTTP exposes `/metrics` on the same listener as `/healthz`, `/readyz`,
and MCP. Operators can disable it with `DBT_NOVA_METRICS_ENABLED=false`.

Metrics must reuse existing recorders where possible. Labels must stay
privacy-safe: tool names and result classes are acceptable; query text, entity
names, paths, user IDs, and credentials are not. Hosted metrics are still an
operator surface and need the same proxy/network ACL as MCP.
