# Hosted Deployment

Use streamable HTTP mode when you want to run `dbt-nova` as a containerized MCP
service on platforms like Cloud Run.

## Authentication Requirement

Hosted `streamable_http` mode has no built-in authentication or authorization.
If you expose it beyond loopback, you must place it behind an authenticating
reverse proxy or platform auth layer first.

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
