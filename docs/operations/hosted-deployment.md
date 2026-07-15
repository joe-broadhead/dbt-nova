# Hosted Deployment

Use streamable HTTP mode when you want to run `dbt-nova` as a containerized MCP
service on platforms like Cloud Run.

## Authentication Requirement

Hosted `streamable_http` mode has no built-in authentication or authorization by
default. If you expose it beyond loopback, you must place it behind an
authenticating reverse proxy or platform auth layer first.

Default-off hosted identity config also supports
`DBT_NOVA_AUTH_MODE=proxy_signed_headers`, where Nova verifies a small HMAC
identity envelope produced by the trusted proxy, and `DBT_NOVA_AUTH_MODE=jwt`,
where Nova verifies inbound bearer JWTs against an operator-configured HTTPS
JWKS. Hosted identity is request attribution and inbound access checking, not
authorization; the proxy or platform layer remains responsible for coarse access
control and network exposure.
See
[Hosted Identity Threat Model](../development/hosted-identity-threat-model.md)
and [Hosted Identity Contract](../development/hosted-identity-contract.md).

## Admin Tool Exposure

MCP/CLI parity includes operator/admin tools such as `show_config`,
`validate_config`, `reload_manifest`, `inspect_storage`, `prune_storage`,
`cleanup_storage`, and `warm_manifest`. Keep admin tools away from normal hosted
agent clients unless the service is isolated and access controlled.

Recommended hosted posture:

- Use `DBT_NOVA_PRESET=hosted-discovery` for general-purpose hosted agent
  endpoints.
- Keep `execute_sql` and `run_recipe` denied for discovery-only endpoints.
  Published container images set `DBT_NOVA_PRESET=hosted-discovery` by default.
- Leave `DBT_NOVA_MCP_ENABLE_STORAGE_ADMIN` unset unless a trusted operator is
  intentionally pruning or cleaning storage.
- Leave `DBT_NOVA_MCP_ENABLE_MANIFEST_WARM` unset unless a trusted operator is
  intentionally warming semantic caches.
- Set `DBT_NOVA_HTTP_EXPECT_AUTH_PROXY=true` only when the reverse proxy or
  platform auth layer is actually enforcing authentication.
- Leave `DBT_NOVA_AUTH_MODE=off` unless the proxy-signed header or JWT contract
  is configured end to end.

## Local HTTP Profile

For local MCP-over-HTTP testing, keep Nova bound to loopback:

```bash
DBT_NOVA_SERVER_TRANSPORT=streamable_http \
dbt-nova server start --http-host 127.0.0.1 --http-port 8080 --http-path /mcp
```

The hosted acknowledgement is not needed for loopback binds.

## Hosted Discovery-Only Profile

Recommended runtime env:

```bash
export DBT_NOVA_PRESET=hosted-discovery
export PORT=8080
export DBT_NOVA_HTTP_PATH=/mcp
export DBT_NOVA_HTTP_EXPECT_AUTH_PROXY=true
export DBT_NOVA_HTTP_ALLOWED_HOSTS=nova.example.com
export DBT_NOVA_HTTP_MAX_BODY_BYTES=16777216
export DBT_NOVA_LOG=info
export DBT_NOVA_LOG_FORMAT=json
# Optional if the proxy/network cannot restrict scrapes:
# export DBT_NOVA_METRICS_ENABLED=false
export DBT_NOVA_STORAGE_DIR=/tmp/dbt-nova
export DBT_NOVA_EMBEDDINGS_CACHE_DIR=/tmp/dbt-nova/models
export DBT_NOVA_BOOTSTRAP_URI='https://example.invalid/bootstrap.json'
export DBT_NOVA_ARTIFACT_FETCH_POLICY=if_missing
unset DBT_NOVA_STORAGE_READ_ONLY

# Optional: enable semantic search components when your runtime has warmed or hydrated models
# export DBT_NOVA_SEARCH_ENABLE_VECTOR=true
# export DBT_NOVA_SEARCH_ENABLE_SPARSE=true
# export DBT_NOVA_SEARCH_ENABLE_RERANKER=true
```

Why these defaults matter:

- `DBT_NOVA_PRESET=hosted-discovery` selects streamable HTTP plus the lean
  `agent` tool profile and hides SQL, recipe execution, eval execution/writes,
  trace replay/write, manifest lifecycle, config inspection, and storage-admin
  tools by default.
- `PORT` lets Nova bind correctly on Cloud Run-style platforms.
- `DBT_NOVA_HTTP_EXPECT_AUTH_PROXY=true` is required for non-loopback hosted binds and documents that an authenticating reverse proxy is in front of Nova.
- `DBT_NOVA_HTTP_ALLOWED_HOSTS` allows the public/proxy `Host` header while the transport still rejects unexpected hosts.
- `DBT_NOVA_HTTP_MAX_BODY_BYTES` caps request bodies before the mounted MCP transport buffers them; keep it bounded unless a stricter proxy limit is enforced.
- `DBT_NOVA_LOG=info` plus `DBT_NOVA_LOG_FORMAT=json` emits newline-delimited
  JSON logs that are suitable for hosted collectors. Leave
  `DBT_NOVA_LOG_FORMAT` unset for the default human-readable stderr format.
- `GET /metrics` is enabled by default for hosted HTTP. It does not include
  query text, entity names, paths, user IDs, or credentials, but it does expose
  readiness, tool names, call counts, error counts, and latency histograms.
  Protect it with the same proxy/network ACL as MCP, or set
  `DBT_NOVA_METRICS_ENABLED=false`.
- Env vars override preset values, so `DBT_NOVA_TOOL_DENYLIST=` intentionally
  clears the hosted denylist. Run `dbt-nova config validate --json` in deploy
  smoke tests to inspect the effective checklist.
- `DBT_NOVA_STORAGE_DIR=/tmp/dbt-nova` gives artifact hydration a writable local filesystem.
- `DBT_NOVA_EMBEDDINGS_CACHE_DIR=/tmp/dbt-nova/models` keeps model cache resolution deterministic.
- `DBT_NOVA_BOOTSTRAP_URI` should point at the stable bootstrap alias published by the reusable asset workflow.
- First start should **not** use strict read-only mode. Nova may need to materialize prebuilt assets locally before it can serve traffic.

## Proxy-Signed Identity Mode

Use this mode when a trusted reverse proxy has already authenticated the caller
and you want Nova to fail closed unless that proxy signs a bounded request
identity envelope.

Nova config:

```bash
export DBT_NOVA_AUTH_MODE=proxy_signed_headers
export DBT_NOVA_AUTH_REQUIRED=true
export DBT_NOVA_PROXY_IDENTITY_HEADER=X-Nova-Identity
export DBT_NOVA_PROXY_SIGNATURE_HEADER=X-Nova-Signature
export DBT_NOVA_PROXY_IDENTITY_SECRET_FILE=/run/secrets/nova_proxy_identity_hmac
export DBT_NOVA_PROXY_IDENTITY_MAX_AGE_SECS=300
```

The secret file must contain at least 32 bytes of HMAC secret material. Nova
reads it locally at startup, trims surrounding ASCII whitespace, and never logs
or returns the secret through config inspection.

The proxy must:

- authenticate the caller before forwarding to Nova;
- remove any inbound client-supplied `X-Nova-Identity` and `X-Nova-Signature`
  headers;
- set the identity header to base64url-no-pad JSON with `iat` and the configured
  subject field, default `sub`;
- set the signature header to `sha256=<base64url-no-pad HMAC-SHA256>`, signing
  the exact identity header value;
- keep `/healthz`, `/readyz`, `/metrics`, and the MCP path behind the same
  signing behavior when proxy mode is enabled.

Example identity JSON before base64url encoding:

```json
{"sub":"user-123","email":"user@example.com","iat":1784097600}
```

Nova rejects missing, malformed, oversized, stale, or badly signed envelopes
with `401 Unauthorized`. Verified identity is recorded as a SHA-256 subject hash
in logs only; metrics do not add identity labels.

## JWT Identity Mode

Use this mode when the caller presents a bearer token issued by a known issuer
and Nova should verify it directly at the hosted HTTP boundary.

Nova config:

```bash
export DBT_NOVA_AUTH_MODE=jwt
export DBT_NOVA_AUTH_REQUIRED=true
export DBT_NOVA_JWT_ISSUER=https://issuer.example
export DBT_NOVA_JWT_AUDIENCE=dbt-nova
export DBT_NOVA_JWT_JWKS_URL=https://issuer.example/.well-known/jwks.json
export DBT_NOVA_JWT_ALGORITHMS=RS256
export DBT_NOVA_JWT_CLOCK_SKEW_SECS=60
```

JWT mode requires `Authorization: Bearer <token>` on every hosted HTTP route,
including `/healthz`, `/readyz`, `/metrics`, and the MCP path. Tokens must
include `kid`, `iss`, `aud`, `exp`, `nbf`, and the configured subject claim
(`sub` by default). Nova accepts only asymmetric/EdDSA algorithms in the
explicit allowlist (`RS*`, `PS*`, `ES*`, `EdDSA`); `none` and HS* algorithms are
rejected.

Nova fetches JWKS at startup and caches it in process. On unknown `kid` or
signature failure, Nova refreshes JWKS once and retries to support key rotation.
If JWKS is unavailable at startup or refresh, JWT mode fails closed. Verified
identity is recorded as a SHA-256 subject hash in logs only; metrics do not add
identity labels.

## Request Correlation

Hosted HTTP mode adds request correlation without implementing identity. Nova
accepts a proxy-provided `X-Request-ID` first, then `X-Correlation-ID`, when the
value is a short printable identifier. If neither header is present or the value
is unsafe, Nova generates a UUID. Every hosted HTTP response echoes the effective
ID in `X-Request-ID`.

Example probe:

```bash
curl -i \
  -H 'X-Request-ID: deploy-smoke-001' \
  http://127.0.0.1:8080/healthz
```

With `DBT_NOVA_LOG=info DBT_NOVA_LOG_FORMAT=json`, request logs include the
request ID, method, path, status, and duration. MCP tool logs include request
ID, tool name, duration, and success/failure. These logs intentionally omit
query strings, request bodies, raw SQL, credentials, manifests, tokens, and
private URIs. When `DBT_NOVA_TRACE_TOOL_CALLS_PATH` is enabled, MCP trace rows
also include `request_id` so a redacted trace artifact can be matched back to
hosted request logs.

## Hosted SQL-Enabled Profile

Start from the SQL-trusted hosted preset, then provide SQL provider credentials:

```bash
export DBT_NOVA_PRESET=hosted-sql-trusted
export DBT_NOVA_SQL_PROVIDER=duckdb
# Set the provider-specific warehouse/catalog credentials needed by your target.
```

Use least-privilege warehouse credentials, keep row/byte/poll limits bounded, and
serve the endpoint only through the same authenticating proxy requirement as the
discovery-only profile.

`hosted-sql-trusted` leaves `execute_sql` eligible through the `analyst` tool
profile, but still denies recipe execution, eval execution/writes, trace
replay/write, manifest lifecycle, config inspection, and storage-admin tools.

Storage note:

- `/tmp/dbt-nova` is suitable for ephemeral containers.
- For persistence across restarts, mount a writable volume and point `DBT_NOVA_STORAGE_DIR` and `DBT_NOVA_EMBEDDINGS_CACHE_DIR` at that volume instead.

## Probe Endpoints

When `DBT_NOVA_SERVER_TRANSPORT=streamable_http`, Nova exposes:

- `GET /healthz`
  - process liveness probe
  - returns `200 OK` with a small JSON payload when the server loop is alive
- `GET /readyz`
  - manifest/search readiness probe
  - returns:
    - `200 OK` when Nova is ready for traffic
    - `503 Service Unavailable` when Nova is not ready for traffic
- `GET /metrics`
  - Prometheus-compatible text metrics
  - returns `200 OK` when `DBT_NOVA_METRICS_ENABLED=true`
  - returns `404 Not Found` when `DBT_NOVA_METRICS_ENABLED=false`

In practice that usually means:

- `ready` -> `200 OK`
- `refreshing` -> `200 OK` when an active searcher is still traffic-ready
- `degraded`, `loading`, `failed` -> `503 Service Unavailable`

`/readyz` reflects the same manifest status that powers the MCP `health` tool,
including refresh stats and active index diagnostics when available.

`/metrics` uses the same readiness signal and the same in-process tool metrics
recorder as the MCP `health` tool. Histogram buckets in the scrape are
cumulative Prometheus buckets; the health JSON bucket map remains
non-cumulative for backward compatibility.

## Docker Build

Build the generic container image:

```bash
docker build -t dbt-nova:latest .
```

Pull the published release image instead:

```bash
docker pull ghcr.io/joe-broadhead/dbt-nova:v<version>
```

Release images are published as a multi-arch manifest for `linux/amd64` and
`linux/arm64`.

Run a local container probe smoke without publishing the service port:

```bash
docker run -d --rm --name dbt-nova-local-smoke \
  -e PORT= \
  -e DBT_NOVA_HTTP_HOST=127.0.0.1 \
  -e DBT_NOVA_HTTP_PORT=8080 \
  -e DBT_NOVA_BOOTSTRAP_URI='https://example.invalid/bootstrap.json' \
  -e DBT_NOVA_ARTIFACT_FETCH_POLICY=if_missing \
  dbt-nova:latest
```

Then verify liveness from inside the container:

```bash
docker exec dbt-nova-local-smoke curl -fsS http://127.0.0.1:8080/healthz
docker rm -f dbt-nova-local-smoke
```

With a real manifest/bootstrap configured, readiness is available at
`/readyz`. The MCP endpoint remains mounted at `DBT_NOVA_HTTP_PATH`:

```text
http://127.0.0.1:8080/mcp
```

Release OCI images are published to GitHub Container Registry on every release tag.
Use one of these tags:

- `ghcr.io/joe-broadhead/dbt-nova:vX.Y.Z`
- `ghcr.io/joe-broadhead/dbt-nova:sha-<git-sha>`

Pin `vX.Y.Z` for downstream deployments. Use the `sha-...` tag when you need an
immutable rollback target tied to a specific release commit.

The published Dockerfile also defines a container `HEALTHCHECK` against
`/healthz`. Platform-level probes should still be configured explicitly:
`/healthz` for liveness and `/readyz` for traffic readiness.

## Cloud Run Notes

For hosted bootstrap consumers, prefer this producer setup:

- stable bootstrap alias from the reusable asset workflow
- `models_distribution_mode=publish_and_bootstrap` when you want fully remote model hydration

That avoids two common hosted pitfalls:

1. `#130`: strict read-only mode plus `DBT_NOVA_ARTIFACT_FETCH_POLICY=if_missing` on a cold container
2. `#131`: missing ONNX/model cache files when bootstrap does not include `models_artifact_uri`

If bootstrap omits `models_artifact_uri`, you must do one of these:

- pre-warm models into the image or mounted cache dir
- allow on-demand model download at runtime
- disable vector/sparse/reranker search layers

## Minimal Deployment Checklist

1. Publish prebuilt assets and a stable bootstrap alias.
2. Set `DBT_NOVA_BOOTSTRAP_URI` to that stable alias.
3. Put dbt-nova behind an authenticating reverse proxy and set `DBT_NOVA_HTTP_EXPECT_AUTH_PROXY=true`.
4. Set `DBT_NOVA_HTTP_ALLOWED_HOSTS` to the public/proxy hostnames clients use.
5. Keep `execute_sql` and `run_recipe` denied unless this is an intentional SQL-enabled endpoint.
6. Keep `DBT_NOVA_STORAGE_DIR` and `DBT_NOVA_EMBEDDINGS_CACHE_DIR` writable.
7. Use `/healthz` for liveness and `/readyz` for readiness.
8. Do not enable strict read-only mode until local artifacts already exist.
