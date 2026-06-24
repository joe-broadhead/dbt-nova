# Response Format

All tools return a consistent envelope.

## Success Response

```json
{
  "success": true,
  "count": 5,
  "total_available": 100,
  "truncated": true,
  "data": [...]
}
```

| Field | Description |
|---|---|
| `success` | Always `true` for successful responses |
| `count` | Number of primary records returned (tool-defined for object payloads) |
| `total_available` | Total matching items before limit |
| `truncated` | `true` when results were limited/truncated |
| `data` | Array of results or a single object |

Notes:
- For array payloads, `count` equals `data.len()`.
- For object payloads, each tool defines primary records explicitly.
- `get_undocumented` sets `count = entities_returned + columns_returned`.

Search responses may include suggestions when no results are found:

```json
{
  "success": true,
  "count": 0,
  "suggestions": ["customers", "dim_customers"],
  "data": []
}
```

Search responses may also include analyst guidance hints when top candidates are near ties:

```json
{
  "success": true,
  "count": 10,
  "persona": "analyst",
  "analysis_hints": [
    "Top candidates `fct_sessions` and `fct_orders` are close (7.2% score gap). Use `get_entity` or `get_context` to verify metric definition, grain, and date/country dimensions before final SQL."
  ],
  "data": [...]
}
```

### Metadata Score Example

```json
{
  "success": true,
  "count": 1,
  "data": {
    "unique_id": "model.jaffle_shop.orders",
    "scope": "entity",
    "persona": "analyst",
    "overall_score": 72,
    "grade": "C",
    "categories": {
      "documentation": { "score": 85, "weight": 0.2, "weighted": 17.0 },
      "semantic": { "score": 65, "weight": 0.45, "weighted": 29.25 },
      "governance": { "score": 40, "weight": 0.15, "weighted": 6.0 },
      "quality": { "score": 98, "weight": 0.2, "weighted": 19.6 }
    },
    "breakdown": {
      "documentation": {
        "description": { "score": 24, "max": 30, "length": 78 },
        "column_descriptions": {
          "score": 32,
          "max": 40,
          "columns_total": 12,
          "columns_described": 10,
          "quality_ratio": 0.8
        },
        "doc_blocks": { "score": 15, "max": 15, "present": true },
        "owner": { "score": 15, "max": 15, "present": true }
      }
    },
    "recommendations": [
      {
        "category": "governance",
        "priority": "high",
        "impact": 25,
        "message": "Add meta.nova.governance.pii classification for sensitive fields",
        "field": "meta.nova.governance.pii"
      }
    ]
  }
}
```

When `detail=standard`, tools return persona‑optimized summaries.  
For `search`, standard summaries include a compact `persona_payload` contract tailored to
`analyst`, `engineer`, or `governance` workflows.  
When `detail=full`, tools return the complete entity payload (same as `get_entity` with `detail=full`).  
For `search`, `include_sql=false` will omit raw/compiled SQL from full payloads.

## Error Response

```json
{
  "success": false,
  "error": "Entity 'foo' not found (resource_type: model) Available resource types: analysis, exposure, macro, metric, model, seed, snapshot, source, test",
  "error_code": "NOT_FOUND",
  "searched": "foo",
  "resource_type": "model",
  "available_resource_types": ["analysis", "exposure", "macro", "metric", "model", "seed", "snapshot", "source", "test"]
}
```

Some tools (for example `run_recipe` preflight validation) also return a
structured `details` object on `INVALID_PARAMS` responses:

```json
{
  "success": false,
  "error": "Invalid parameter: Missing required recipe parameters: COUNTRY_CODE",
  "error_code": "INVALID_PARAMS",
  "details": {
    "missing_parameters": ["COUNTRY_CODE"],
    "unused_parameters": [],
    "type_mismatches": [],
    "by_query": []
  }
}
```

## CLI JSON Envelope

CLI subcommands that use `--json` return a command envelope (different from MCP tool envelopes):

```json
{
  "command": "manifest load",
  "status": "success",
  "data": { "...": "..." },
  "meta": {
    "elapsed_ms": 123,
    "timestamp_ms": 1772304167827,
    "version": "0.0.6"
  },
  "error": null
}
```

For errors, `status` is `"error"` and `error` contains the standard Nova error object.

### CLI Exit Codes

| Code | Meaning |
|---|---|
| `0` | Success |
| `1` | Invalid parameters |
| `2` | Manifest/index lifecycle failures |
| `3` | Runtime/provider/server failures |
