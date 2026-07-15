# Manifest Sources & Auth

Nova can load `manifest.json` from local files or remote storage. Configure via:

- `DBT_MANIFEST_PATH` (local path)
- `DBT_NOVA_MANIFEST_URI` (remote URI, optional)

Supported URI schemes:

- `file://` — local file
- `http://` / `https://` — remote URL
- `dbfs://` — Databricks DBFS
- `s3://` — Amazon S3
- `gs://` — Google Cloud Storage

For compatibility, legacy `dbfs:/...` manifests are also accepted and normalized
to `dbfs://...`.

## Provider Notes

Manifest sources are resolved via a provider registry keyed by URI scheme. Adding
new storage backends means registering a new provider for its scheme.

## Auth Matrix (Quick Reference)

| Source | Default mode | Auth env vars | Notes |
|---|---|---|---|
| `file://` / local path | Local file | — | No auth |
| `http(s)://` | HTTPS | — | Use public or presigned URLs |
| `dbfs://` | Databricks API | `DATABRICKS_HOST`, `DATABRICKS_ACCESS_TOKEN` | Uses DBFS REST |
| `s3://` | HTTPS | `DBT_NOVA_S3_ENDPOINT` (optional) | Public/presigned URL |
| `s3://` (SDK) | AWS SDK | `AWS_REGION`, `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` (or `AWS_PROFILE`) | Set `DBT_NOVA_S3_MODE=sdk` |
| `gs://` | HTTPS | `DBT_NOVA_GCS_ENDPOINT` (optional) | Public/presigned URL |
| `gs://` (SDK) | GCS SDK | `GOOGLE_APPLICATION_CREDENTIALS` (or ADC) | Set `DBT_NOVA_GCS_MODE=sdk` |

## Local File (default)

No auth required.

```bash
export DBT_MANIFEST_PATH=/path/to/manifest.json
```

## HTTP / HTTPS

Nova does not send custom auth headers. Use a public URL or a presigned URL.
`https://` is allowed by default. `http://` is blocked unless explicitly enabled.

```bash
export DBT_NOVA_MANIFEST_URI=https://example.com/manifest.json
```

To allow insecure `http://` manifests:

```bash
export DBT_NOVA_MANIFEST_ALLOW_HTTP=true
```

## Databricks DBFS (`dbfs://`)

Requires Databricks API credentials:

```bash
export DBT_NOVA_MANIFEST_URI=dbfs:///mnt/analytics/manifest.json
export DATABRICKS_HOST=https://<workspace>
export DATABRICKS_ACCESS_TOKEN=...
```

Legacy form also works:

```bash
export DBT_NOVA_MANIFEST_URI=dbfs:/mnt/analytics/manifest.json
```

## Amazon S3 (`s3://`)

Default mode is HTTPS (public or presigned URLs).

```bash
export DBT_NOVA_MANIFEST_URI=s3://my-bucket/path/manifest.json
```

Optional endpoint override:

```bash
export DBT_NOVA_S3_ENDPOINT=s3.amazonaws.com
```

### S3 SDK mode (credentialed)

```bash
export DBT_NOVA_S3_MODE=sdk
export AWS_REGION=eu-west-1
export AWS_ACCESS_KEY_ID=...
export AWS_SECRET_ACCESS_KEY=...
```

Nova uses the AWS default credential chain, so `AWS_PROFILE` and session tokens
also work if configured.

## Google Cloud Storage (`gs://`)

Default mode is HTTPS (public or presigned URLs).

```bash
export DBT_NOVA_MANIFEST_URI=gs://my-bucket/path/manifest.json
```

Optional endpoint override:

```bash
export DBT_NOVA_GCS_ENDPOINT=storage.googleapis.com
```

### GCS SDK mode (credentialed)

```bash
export DBT_NOVA_GCS_MODE=sdk
export GOOGLE_APPLICATION_CREDENTIALS=/path/to/service-account.json
```

Application Default Credentials (ADC) are also supported (e.g. `gcloud auth application-default login`).

The same `GOOGLE_APPLICATION_CREDENTIALS` / ADC flow is also supported by the
BigQuery SQL provider (`DBT_NOVA_SQL_PROVIDER=bigquery`).

## Caching & Refresh

Remote manifests are cached under:

```
<storage_root>/manifests/
```

Configure:

- `DBT_NOVA_MANIFEST_CACHE_DIR` — override cache location
- `DBT_NOVA_MANIFEST_MAX_BYTES` — cap remote manifest size (`0` = unlimited)
- `DBT_NOVA_MANIFEST_REFRESH_SECS` — refresh interval (0 = never refresh)

### Reloading without restart

If you need to switch manifest sources at runtime (e.g., engineer local → analyst DBFS),
start the MCP server with `DBT_NOVA_MCP_ENABLE_MANIFEST_RELOAD=1` and use the
`reload_manifest` tool:

```json
{"name":"reload_manifest","arguments":{"manifest_uri":"dbfs:///mnt/analytics/manifest.json","refresh_secs":300}}
```

`reload_manifest` rebuilds indexes in the background and swaps atomically when
ready. Without that opt-in, MCP reloads can refresh only the current source.

## Versioned Indexes & Atomic Swaps

Nova keeps versioned index directories per manifest content hash:

```
<storage_root>/instances/<instance_id>/versions/<hash>/
```

The active version is tracked in:

```
<storage_root>/instances/<instance_id>/manifest.current.json
```

When `DBT_NOVA_MANIFEST_REFRESH_SECS` is enabled, Nova:

1. Resolves the manifest source (local or cached remote).
2. Computes the content hash.
3. Builds new indexes in a new version directory in the background.
4. Atomically swaps the active version once ready (no downtime).
5. Keeps the previous version available for in-flight requests.

Concurrent loaders do not need the build lock when a complete matching version
already exists. If another process is building a newer version, Nova serves the
published `manifest.current.json` version until the new build is complete.

## Multi‑repo / Multi‑manifest Workflow

Nova isolates indexes by manifest path or URI. If you work across multiple repos or
multiple manifests:

- Use distinct `DBT_MANIFEST_PATH` / `DBT_NOVA_MANIFEST_URI` values per process.
- Override `DBT_NOVA_STORAGE_INSTANCE_ID` if you need explicit isolation.
- For remote manifests, the URI itself becomes the instance seed, so different URIs
  produce different instance directories automatically.
