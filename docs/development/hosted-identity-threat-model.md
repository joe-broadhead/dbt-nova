# Hosted Identity Threat Model

Status: threat model for shipped proxy-signed identity and JWT hosted identity
modes. Future identity work must keep these boundaries intact.

## Current State

`streamable_http` has no built-in authentication by default. Non-loopback hosted
deployments must sit behind an authenticating reverse proxy or platform auth
layer and set `DBT_NOVA_HTTP_EXPECT_AUTH_PROXY=true` only when that layer is
actually enforcing access. Operators may additionally enable default-off
`proxy_signed_headers` or `jwt` verification for hosted HTTP requests.

The identity extension track is default-off and hosted-only. It is scoped to
request attribution and optional inbound authentication checks for hosted HTTP.
It must not change stdio, CLI, loopback development, or the one-manifest
metadata-bridge model.

## Assets

Protect:

- Manifest-derived metadata, lineage, SQL text, recipes, and readiness evidence.
- Warehouse-backed execution paths such as `execute_sql` and `run_recipe`.
- Operator surfaces such as config inspection, storage admin, eval execution,
  trace writes, and manifest lifecycle controls.
- Logs, traces, metrics, and future request identity fields.
- Local storage and prebuilt artifacts materialized by the Nova process.

## Trust Boundaries

- Client to authenticating proxy or platform auth layer.
- Proxy to Nova `streamable_http` listener.
- Nova process to local storage, prebuilt artifact stores, and configured
  warehouse providers.
- Nova process to logs, metrics, and trace artifacts.

Nova may trust identity only after validating the selected auth mode. A request
identity is not a tenant boundary and is not evidence that a caller may access a
specific entity, column, model, metric, recipe, or warehouse credential.

## Threats

| Threat | Required control |
|---|---|
| Direct internet access to Nova without proxy auth | Non-loopback bind validation plus proxy deployment requirement |
| Spoofed proxy identity headers | Signed proxy envelope, trusted header names, timestamp/nonce bounds, and fail-closed parsing |
| Bearer token replay or substitution | JWT issuer, audience, expiry, not-before, signature, and algorithm allowlist validation |
| Algorithm confusion or unsigned token acceptance | Explicit accepted algorithms, reject `none`, reject unsupported key types |
| Claim injection into logs or metrics | Sanitize and bound request identity fields; never add user IDs to Prometheus labels by default |
| Confusing identity with authorization | Document identity as request attribution only; keep tool exposure controlled by presets/allowlists/denylists |
| Tenant or manifest isolation by claim | Explicitly reject tenant routing and multi-manifest selection in this track |
| Warehouse credential brokering by claim | Keep provider credentials configured out of band; no per-request credential exchange |
| Semantic-layer authorization drift | Do not use identity claims to authorize MetricFlow/dbt Semantic Layer execution inside Nova |

## Non-Goals

Identity work must not add:

- Tenant routing.
- Per-entity, per-column, or per-indicator ABAC.
- warehouse credential brokering.
- Semantic-layer authorization.
- A SaaS control plane.
- Multi-manifest isolation.
- Required identity for stdio, CLI, or loopback local development.

## Request Identity

Request identity is limited to sanitized attribution context:

- Stable subject.
- Optional display name.
- Optional email when explicitly configured.
- Optional groups or roles only for future policy hooks, not implicit
  authorization.
- Issuer or proxy source.
- Authentication mode.

Request identity may appear in structured logs or audit events only after
redaction, length limits, and cardinality review. It must not become a
Prometheus label by default.

## Fail-Closed Rules

Nova must reject startup or requests when:

- An unknown auth mode is configured.
- A non-off auth mode lacks required validation material.
- A proxy-signed header envelope has an invalid signature, stale timestamp, or
  malformed identity body.
- JWT mode receives a missing bearer token, invalid issuer, invalid audience,
  expired token, not-yet-valid token, unsupported algorithm, invalid signature,
  or missing required subject claim.

Mode `off` preserves current behavior: Nova relies on the external proxy or
platform layer and does not construct request identity.

## Shipped Sequence

1. JOE-665: accept this threat model and default-off hosted identity design.
2. JOE-662: add fail-closed config parsing without changing effective runtime
   behavior for mode `off`.
3. JOE-663: implement proxy-signed identity headers.
4. JOE-664: implement JWT/JWKS validation.
