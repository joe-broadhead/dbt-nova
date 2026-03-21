# Hosted Deployment

Use streamable HTTP mode when you want to run `dbt-nova` as a containerized MCP
service on platforms like Cloud Run.

## Hosted Profile

Recommended runtime env:

```bash
export DBT_NOVA_SERVER_TRANSPORT=streamable_http
export PORT=8080
export DBT_NOVA_HTTP_PATH=/mcp
export DBT_NOVA_STORAGE_DIR=/tmp/dbt-nova
export DBT_NOVA_EMBEDDINGS_CACHE_DIR=/tmp/dbt-nova/models
export DBT_NOVA_BOOTSTRAP_URI='https://example.invalid/bootstrap.json'
export DBT_NOVA_ARTIFACT_FETCH_POLICY=if_missing
unset DBT_NOVA_STORAGE_READ_ONLY
```

Why these defaults matter:

- `PORT` lets Nova bind correctly on Cloud Run-style platforms.
- `DBT_NOVA_STORAGE_DIR=/tmp/dbt-nova` gives artifact hydration a writable local filesystem.
- `DBT_NOVA_EMBEDDINGS_CACHE_DIR=/tmp/dbt-nova/models` keeps model cache resolution deterministic.
- `DBT_NOVA_BOOTSTRAP_URI` should point at the stable bootstrap alias published by the reusable asset workflow.
- First start should **not** use strict read-only mode. Nova may need to materialize prebuilt assets locally before it can serve traffic.

## Probe Endpoints

When `DBT_NOVA_SERVER_TRANSPORT=streamable_http`, Nova exposes:

- `GET /healthz`
  - process liveness probe
  - returns `200 OK` with a small JSON payload when the server loop is alive
- `GET /readyz`
  - manifest/search readiness probe
  - returns:
    - `200 OK` when status is `ready` or `refreshing`
    - `503 Service Unavailable` when status is `loading` or `failed`

`/readyz` reflects the same manifest status that powers the MCP `health` tool,
including refresh stats and active index diagnostics when available.

## Docker Build

Build the generic container image:

```bash
docker build -t dbt-nova:latest .
```

Pull the published release image instead:

```bash
docker pull ghcr.io/joe-broadhead/dbt-nova:v<version>
```

Run it locally:

```bash
docker run --rm -p 8080:8080 \
  -e DBT_NOVA_BOOTSTRAP_URI='https://example.invalid/bootstrap.json' \
  -e DBT_NOVA_ARTIFACT_FETCH_POLICY=if_missing \
  dbt-nova:latest
```

Then verify:

```bash
curl -fsS http://127.0.0.1:8080/healthz
curl -fsS http://127.0.0.1:8080/readyz
```

The MCP endpoint remains mounted at `DBT_NOVA_HTTP_PATH`:

```text
http://127.0.0.1:8080/mcp
```

Release OCI images are published to GitHub Container Registry on every release tag.
Use one of these tags:

- `ghcr.io/joe-broadhead/dbt-nova:vX.Y.Z`
- `ghcr.io/joe-broadhead/dbt-nova:sha-<git-sha>`

Pin `vX.Y.Z` for downstream deployments. Use the `sha-...` tag when you need an
immutable rollback target tied to a specific release commit.

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
3. Keep `DBT_NOVA_STORAGE_DIR` and `DBT_NOVA_EMBEDDINGS_CACHE_DIR` writable.
4. Use `/healthz` for liveness and `/readyz` for readiness.
5. Do not enable strict read-only mode until local artifacts already exist.
