# Manifest Refresh

Use this after rebuilding, uploading, or changing the manifest so Nova reflects the latest project state.

## Steps

1) Compile or build your dbt project
- Ensure `manifest.json` is up-to-date.
- Upload to your chosen manifest location (local, dbfs, s3, gcs, http).

2) Reload in Nova

```json
{"name":"reload_manifest","arguments":{"manifest_uri":"dbfs:///path/to/manifest.json","refresh_secs":300}}
```

You can also use a local path:

```json
{"name":"reload_manifest","arguments":{"manifest_path":"/abs/path/to/manifest.json","refresh_secs":300}}
```

3) Check readiness

```json
{"name":"health","arguments":{}}
```

Wait until status is `ready` before trusting search, scoring, or downstream validation.
