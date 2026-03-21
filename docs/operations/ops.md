# Operations & Troubleshooting

This page covers operational checks for manifest refresh, caches, and index storage.

## Refresh & Cache Troubleshooting

!!! tip "First Check"
    Always start with the `health` tool when diagnosing issues. It shows status,
    manifest details, and refresh metrics in one call.

### Quick health check

Use the `health` tool to verify readiness and see the active manifest details:

- `status`: `loading` | `ready` | `refreshing` | `failed`
- `loading`: bootstrapping or rebuild in progress, no active index yet
- `ready`: healthy active index serving traffic
- `refreshing`: rebuilding in background while serving the existing active index
- `failed`: initial manifest load failed (common after bad JSON, bad permissions, or temporary source failure)
- `manifest.hash`: content hash of the active manifest (`ready` / `refreshing`)
- `manifest.version`: version directory name (hash prefix)
- `manifest.age_ms`: age of the manifest file based on its `modified_ms`
- `manifest.loaded_at_ms`: when this index version was built
- `manifest.loaded_age_ms`: time since build

Refresh stats (if enabled):

- `refresh.attempts`: refresh attempts (manifest changed)
- `refresh.successes`: successful refresh swaps
- `refresh.failures`: failed refresh swaps
- `refresh.last_attempt_ms`: timestamp of last refresh attempt
- `refresh.last_success_ms`: timestamp of last success
- `refresh.last_failure_ms`: timestamp of last failure
- `refresh.last_error`: last error string (if any)

Tool metrics:

- `tool_metrics.<tool>.calls`: total calls per tool
- `tool_metrics.<tool>.errors`: total error responses per tool
- `tool_metrics.<tool>.error_rate_bps`: error ratio in basis points (`10000 * errors / calls`)
- `tool_metrics.<tool>.total_ms`: cumulative latency
- `tool_metrics.<tool>.avg_ms`: average duration in ms
- `tool_metrics.<tool>.p95_ms`: approximate p95 latency in ms
- `tool_metrics.<tool>.p99_ms`: approximate p99 latency in ms
- `tool_metrics.<tool>.max_ms`: maximum duration in ms
- `tool_metrics.<tool>.buckets`: latency buckets (<=5ms, <=10ms, <=50ms, <=100ms, <=500ms, <=1000ms, >1000ms)

Search concurrency:

- `search_concurrency.enabled`: whether queue/concurrency controls are active
- `search_concurrency.max_concurrent`: configured slot limit
- `search_concurrency.in_flight`: active searches
- `search_concurrency.saturated`: true when no execution slots are free
- `search_concurrency.max_queue`: configured queue length
- `search_concurrency.queued`: searches waiting in queue
- `search_concurrency.queue_saturated`: true when slots and queue are both full

Cache stats:

- `manifest_cache.hits`: number of times a cached manifest was used
- `manifest_cache.misses`: number of times a remote fetch was attempted

Artifact consumer stats (when prebuilt artifact mode is configured):

- `artifact_consumer.enabled`: whether remote artifact mode is active
- `artifact_consumer.storage_read_only`: whether runtime is in strict read-only mode
- `artifact_consumer.consumer_mode_hint`: `writable_hydration` or `strict_read_only_reuse`
- `artifact_consumer.guidance`: operator hint for the active consumer mode
- `artifact_consumer.fetch_policy`: `if_missing` | `always` | `never`
- `artifact_consumer.metadata_validated`: metadata contract was parsed and validated
- `artifact_consumer.storage_materialized`: storage archive was materialized in this load cycle
- `artifact_consumer.models_materialized`: models archive was materialized in this load cycle
- `artifact_consumer.last_evaluated_at_ms`: timestamp for last artifact-consumer decision
- `artifact_consumer.last_materialized_at_ms`: timestamp for last successful materialization (if any)

## Rate limiting

Nova applies tool rate limits by default (`search=60,execute_sql=20,default=120` per
60 seconds). It returns `RATE_LIMITED` when a tool exceeds its configured calls per
window. Consider increasing limits with per-tool overrides (e.g.,
`search=120,execute_sql=30,default=60`) or set `DBT_NOVA_TOOL_RATE_LIMITS=` to disable.


If `status=refreshing`, Nova is building new indexes in the background and serving the
active version until the swap completes.

If `status=failed` during startup, Nova keeps retrying refresh attempts on the
configured interval. Once the underlying manifest source becomes valid, status
returns to `ready` automatically without a process restart.

### Force a refresh (local manifest)

1. Edit or replace the local `manifest.json` file.
2. Ensure `DBT_NOVA_MANIFEST_REFRESH_SECS` is set (e.g. `5`).
3. Wait for the next refresh interval; confirm `manifest.hash` changes via `health`.

### Reload manifest source without restart

Use the `reload_manifest` tool to point Nova at a new manifest path/URI and rebuild
indexes in the background:

```json
{"name":"reload_manifest","arguments":{"manifest_path":"/path/to/manifest.json"}}
```

### Force a refresh (remote manifest)

If using `http(s)://`, `s3://`, `gs://`, or `dbfs://`:

1. Update the remote manifest.
2. Set a small refresh interval: `DBT_NOVA_MANIFEST_REFRESH_SECS=5`.
3. If the remote doesn’t expose modified metadata, you can clear cache:

```
rm -rf <storage_root>/manifests
```

Nova will refetch on the next refresh cycle.

### Clear caches with script

Use the helper script to clean cache directories safely:

```bash
# Preview what would be removed
scripts/clean_cache.sh --dry-run

# Remove all cache directories (manifests + instances + embeddings)
scripts/clean_cache.sh --all --yes

# Remove only manifest cache
scripts/clean_cache.sh --manifests --yes
```

By default, the script uses `DBT_NOVA_STORAGE_DIR` (or `.dbt-nova`) as the storage
root. Override paths with:

- `--storage-root <path>`
- `--manifest-cache-dir <path>`
- `--embeddings-cache-dir <path>`

### Cache fallback behavior

If a remote fetch fails, Nova will fall back to the cached manifest (if available)
and log a warning. This prevents downtime when remote storage is temporarily unavailable.

To test fallback behavior:

1. Set `DBT_NOVA_MANIFEST_URI` to a remote source.
2. Ensure a cached copy exists under `<storage_root>/manifests/`.
3. Make the remote temporarily unavailable.
4. Confirm `resolve_manifest` returns `cached=true` and `health` remains `ready`.

## Storage Layout

```
<storage_root>/
  .fastembed_cache/
  manifests/
    <hash>.json
    <hash>.meta.json
  instances/
    <instance_id>/
      manifest.current.json
      versions/
        <manifest_hash>/
          entities.bin
          entities.idx
          entities.checksum.json
          index/
          indexes.rkyv
          embeddings.rkyv.zst
          sparse_embeddings.rkyv.zst
          manifest.signature.json
          .in_use.lock
      .build.lock
```

- **`manifest.current.json`** points at the active version.
- **`versions/`** contains immutable, content-addressed index builds.
- **`.in_use.lock`** protects active versions from pruning.

## Pruning Behavior

Nova prunes old versions when either:
- `DBT_NOVA_STORAGE_MAX_INSTANCES` is exceeded, or
- `DBT_NOVA_STORAGE_MAX_BYTES` is exceeded.

It always keeps at least `DBT_NOVA_STORAGE_MIN_VERSIONS` versions per instance, even if
size limits are exceeded.

## Troubleshooting Checklist

!!! info "Systematic Debugging"
    Work through this checklist in order. Most issues are resolved in steps 1-3.

1. Check `health` for `status`, `manifest.hash`, and `refresh.*` counters.
2. Confirm `DBT_MANIFEST_PATH` / `DBT_NOVA_MANIFEST_URI` points to the correct manifest.
3. If remote, verify auth env vars for the chosen scheme (see [Manifest Sources](../configuration/manifest-sources.md)).
4. Ensure the cache directory is writable and not full (`<storage_root>/manifests`).
5. Clear cache if stale: `rm -rf <storage_root>/manifests`.
6. Confirm storage pruning didn't remove the active version (look at `.in_use.lock`).

---

## See Also

- [Configuration Reference](../configuration/reference.md) - All environment variables
- [Manifest Sources](../configuration/manifest-sources.md) - Remote manifest authentication
- [Performance](performance.md) - Optimization tips
