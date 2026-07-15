# Hosted Identity Contract

Status: proxy-signed identity implemented for JOE-663 and JWT validation
implemented for JOE-664. Both modes are default-off, fail closed when enabled,
and apply only to hosted `streamable_http` requests.

## Product Boundary

Hosted identity is an optional `streamable_http` extension for request
attribution and inbound authentication checks. It does not change Nova's core
role as a one-manifest metadata bridge for agents.

The deployment posture stays proxy-first: Nova should sit behind an
authenticating proxy or platform boundary even when a default-off hosted
identity mode is enabled.

Identity is not authorization. Tool exposure remains controlled by runtime
presets, allowlists, denylists, and explicit safety gates. Entity-level and
warehouse-level authorization stay outside Nova.

## Modes

| Mode | Purpose | Default |
|---|---|---|
| `off` | Current behavior: rely on the authenticating proxy or platform layer | Yes |
| `proxy_signed_headers` | Verify a small signed identity envelope produced by a trusted proxy | No |
| `jwt` | Bearer JWT validation at the Nova HTTP boundary | No |

Unknown modes fail validation. Non-off modes fail closed when required
validation material is missing or invalid. `proxy_signed_headers` enforces
request signatures and freshness when configured. `jwt` enforces bearer token
issuer, audience, time, signature, key ID, and algorithm checks when configured.

## Config Surface

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
| `DBT_NOVA_JWT_ALGORITHMS` | JWT mode | Explicit asymmetric algorithm allowlist; `none` and HS* algorithms are never accepted |
| `DBT_NOVA_JWT_CLOCK_SKEW_SECS` | JWT mode | Small leeway for `exp` and `nbf` checks |

Secrets and key material must not be logged or returned by `show_config`.
Config validation should report presence and posture, not raw values.

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

## JWT Mode

For `jwt`, callers send:

- `Authorization: Bearer <token>`.

Nova fetches the configured HTTPS JWKS at startup and caches it in process. On an
unknown `kid` or signature failure, Nova refreshes JWKS once and retries so
normal key rotation can succeed. If JWKS cannot be fetched at startup or during
refresh, JWT mode fails closed.

JWT requirements:

- token header `kid` must be present and match a JWKS key;
- `alg` must be in `DBT_NOVA_JWT_ALGORITHMS`;
- only asymmetric/EdDSA algorithms are accepted (`RS*`, `PS*`, `ES*`, `EdDSA`);
- `iss`, `aud`, `exp`, and `nbf` are required and validated;
- the configured `DBT_NOVA_IDENTITY_SUBJECT_CLAIM`, default `sub`, must be a
  non-empty string.

Nova ignores token-provided `jku`, embedded `jwk`, and x509 URL/header hints. It
uses only the operator-configured JWKS URL.

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
- Reject unsigned tokens, HS* algorithms, missing/unknown `kid`, and unsupported
  algorithms.
- Apply the requirement to all hosted HTTP routes, including probes and metrics.
- Treat claims as sanitized request context only.

## Observability Contract

Hosted identity logs include bounded `auth_mode` and a SHA-256 subject hash after
verification. Metrics must not add raw subject, email, group, token, query text,
entity name, path, or credential labels by default.

## Verification Coverage

Implementation PRs should keep failing-closed tests around:

- Unknown mode fails validation.
- Non-off mode without required config fails validation.
- Stdio and CLI behavior are unchanged when mode is `off`.
- Proxy mode rejects missing/invalid/stale signatures.
- JWT mode rejects missing bearer token, bad issuer, bad audience, expired
  token, not-yet-valid token, invalid signature, unknown `kid`, JWKS outage,
  and unsupported algorithms.
- Sanitized identity never leaks secrets through config, metrics, or error
  payloads.
