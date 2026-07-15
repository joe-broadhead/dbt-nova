# Hosted Identity Contract

Status: proxy-signed identity implemented for JOE-663; JWT remains a planned,
fail-closed contract. Proxy settings are parsed and enforced by streamable HTTP
mode when enabled. JWT settings are parsed for future compatibility but still
fail validation until a verifier implementation lands.

## Product Boundary

Hosted identity is an optional `streamable_http` extension for request
attribution and inbound authentication checks. It does not change Nova's core
role as a one-manifest metadata bridge for agents.

Identity is not authorization. Tool exposure remains controlled by runtime
presets, allowlists, denylists, and explicit safety gates. Entity-level and
warehouse-level authorization stay outside Nova.

## Modes

| Mode | Purpose | Default |
|---|---|---|
| `off` | Current behavior: rely on the authenticating proxy or platform layer | Yes |
| `proxy_signed_headers` | Verify a small signed identity envelope produced by a trusted proxy | No |
| `jwt` | Planned bearer JWT validation at the Nova HTTP boundary | No |

Unknown modes fail validation. Non-off modes fail closed when required
validation material is missing or invalid. `proxy_signed_headers` enforces
request signatures and freshness when configured. `jwt` still fails validation
instead of silently running unauthenticated.

## Planned Config Surface

The names below are the current config contract.

| Setting | Applies to | Contract |
|---|---|---|
| `DBT_NOVA_AUTH_MODE` | all hosted identity | `off`, `proxy_signed_headers`, or `jwt`; default `off` |
| `DBT_NOVA_AUTH_REQUIRED` | non-off modes | Reject unauthenticated requests when true; non-off modes should default to required |
| `DBT_NOVA_IDENTITY_SUBJECT_CLAIM` | proxy/JWT | Claim or field used as the stable subject; required for non-off modes |
| `DBT_NOVA_IDENTITY_EMAIL_CLAIM` | proxy/JWT | Optional email claim; sanitized and never used as an authz decision by default |
| `DBT_NOVA_IDENTITY_NAME_CLAIM` | proxy/JWT | Optional display-name claim; sanitized |
| `DBT_NOVA_IDENTITY_GROUPS_CLAIM` | proxy/JWT | Optional groups claim for future policy hooks; no implicit authorization |
| `DBT_NOVA_PROXY_IDENTITY_HEADER` | proxy mode | Header carrying the base64url JSON identity envelope |
| `DBT_NOVA_PROXY_SIGNATURE_HEADER` | proxy mode | Header carrying `sha256=<base64url-no-pad HMAC-SHA256>` |
| `DBT_NOVA_PROXY_IDENTITY_SECRET_FILE` | proxy mode | Local file containing at least 32 bytes of HMAC verification secret |
| `DBT_NOVA_PROXY_IDENTITY_MAX_AGE_SECS` | proxy mode | Timestamp freshness window |
| `DBT_NOVA_JWT_ISSUER` | JWT mode | Required issuer allowlist entry |
| `DBT_NOVA_JWT_AUDIENCE` | JWT mode | Required audience allowlist entry |
| `DBT_NOVA_JWT_JWKS_URL` | JWT mode | HTTPS JWKS endpoint for signature verification |
| `DBT_NOVA_JWT_ALGORITHMS` | JWT mode | Explicit algorithm allowlist; `none` is never accepted |
| `DBT_NOVA_JWT_CLOCK_SKEW_SECS` | JWT mode | Small leeway for `exp` and `nbf` checks |

Secrets and key material must not be logged or returned by `show_config`.
Config validation should report presence and posture, not raw values. JWT mode
is parsed but not enforced and therefore fails closed until a verifier
implementation lands.

## Proxy Envelope

For `proxy_signed_headers`, the proxy sets:

- `DBT_NOVA_PROXY_IDENTITY_HEADER`: base64url-no-pad encoded JSON object.
- `DBT_NOVA_PROXY_SIGNATURE_HEADER`: `sha256=` followed by the base64url-no-pad
  HMAC-SHA256 signature over the exact identity header value.

The envelope must be a JSON object with:

- `iat`: Unix timestamp in seconds, used with
  `DBT_NOVA_PROXY_IDENTITY_MAX_AGE_SECS`.
- the configured `DBT_NOVA_IDENTITY_SUBJECT_CLAIM` field, default `sub`, as a
  non-empty string.

Optional email, name, and groups fields may be present for future request
context, but Nova does not use them for authorization.

The proxy must strip any inbound client-supplied identity/signature headers
before setting its own trusted values. Stale, malformed, missing, oversized, or
badly signed envelopes return `401 Unauthorized`.

## Runtime Behavior Contract

Mode `off`:

- Preserve current hosted behavior.
- Do not require identity for stdio, CLI, local server development, or loopback
  HTTP.
- Continue to require `DBT_NOVA_HTTP_EXPECT_AUTH_PROXY=true` for non-loopback
  binds.

Mode `proxy_signed_headers`:

- Accept identity only from configured headers.
- Verify signature and timestamp before constructing request identity.
- Reject malformed, unsigned, stale, or oversized envelopes.
- Apply the requirement to all hosted HTTP routes, including probes and metrics.
- Treat identity as request context only.

Mode `jwt`:

- Require `Authorization: Bearer <token>`.
- Validate issuer, audience, expiry, not-before, signature, and accepted
  algorithm.
- Reject unsigned tokens and unsupported algorithms.
- Treat claims as sanitized request context only.

## Observability Contract

Proxy identity logs include bounded `auth_mode` and a SHA-256 subject hash after
verification. Metrics must not add raw subject, email, group, token, query text,
entity name, path, or credential labels by default.

## Tests Before Implementation

The implementation PRs should add failing-closed tests before enabling behavior:

- Unknown mode fails validation.
- Non-off mode without required config fails validation.
- Stdio and CLI behavior are unchanged when mode is `off`.
- Proxy mode rejects missing/invalid/stale signatures.
- JWT mode rejects missing bearer token, bad issuer, bad audience, expired
  token, not-yet-valid token, invalid signature, and unsupported algorithms.
- Sanitized identity never leaks secrets through config, metrics, or error
  payloads.
