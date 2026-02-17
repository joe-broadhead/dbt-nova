# Troubleshooting Guide

## Common Issues

### Manifest fails to load

**Symptoms**: Server returns "manifest not found", connection errors, or `status: failed` in `health`.

**Solutions**:
1. Verify the configured path/URI is correct and readable.
2. If using local files, confirm `DBT_MANIFEST_PATH` points to the expected manifest.
3. If using remote sources, confirm `DBT_NOVA_MANIFEST_URI` uses a supported scheme and the source is reachable.
4. Check permissions: file must be readable.
5. For S3: verify AWS credentials and endpoint config are correct.
6. Check for JSON syntax errors: `jq . manifest.json`
7. If `health` reports `status: failed` at startup, fix the manifest/source and wait for automatic recovery:
   `loading` → `ready` should occur on next refresh attempt.

### Search returns no results

**Symptoms**: All searches return empty arrays

**Solutions**:
1. Verify manifest loaded: call `health` tool
2. Check query syntax: avoid special characters
3. Try broader search without filters
4. Check `resource_types` filter isn't too restrictive

### Out of memory errors

**Symptoms**: Process killed, OOM errors in logs

**Solutions**:
1. Reduce `DBT_NOVA_SEARCH_VECTOR_TOP_K` and `DBT_NOVA_SEARCH_SPARSE_TOP_K`
2. Enable quantization
3. Increase container memory limit
4. Split large manifests

### Slow search performance

**Symptoms**: Searches take >1 second

**Solutions**:
1. Check manifest size in `health` response
2. Disable reranker for faster results
3. Reduce `DBT_NOVA_SEARCH_RRF_OVERFETCH`
4. Enable vector quantization

### Column lineage incomplete

**Symptoms**: Missing columns in lineage output

**Solutions**:
1. Verify manifest has column metadata
2. Check `confidence` threshold isn't too high
3. Increase `lineage_depth`
4. SQL might not be parseable - check for dynamic SQL

### SQL execution blocked

**Symptoms**: "Statement type not allowed" errors

**Solutions**:
1. Only SELECT statements are permitted
2. EXPLAIN SELECT is allowed
3. Remove any DDL/DML (DROP, DELETE, INSERT, UPDATE)
4. CTEs are allowed within SELECT

## Getting Help

1. Check server logs for detailed error messages
2. Use the `health` tool to verify server state
3. File issues at: the repository issue tracker
