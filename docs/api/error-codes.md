# Error Codes Reference

All dbt-nova errors include a machine-readable `error_code` for programmatic handling.
Nova-owned MCP error response bodies also include the top-level `api` response
contract marker described in [Response Format](response-format.md#api-contract-marker).

## Error Codes

| Code | HTTP Equiv | Description | Recovery |
|------|------------|-------------|----------|
| `NOT_FOUND` | 404 | Entity, column, or SQL not found | Check spelling, use search |
| `AMBIGUOUS` | 300 | Multiple entities match the name | Use `unique_id` instead |
| `INVALID_PARAMS` | 400 | Invalid request parameters | Check parameter types |
| `RATE_LIMITED` | 429 | Too many requests | Wait and retry |
| `INDEX_BUILDING` | 503 | Manifest still loading | Retry after delay |
| `INITIALIZATION_ERROR` | 500 | Failed to load manifest | Check path/permissions |
| `SERVER_ERROR` | 500 | Internal server error | Check logs |

## Response Format

### Success Response

```json
{
  "success": true,
  "count": 5,
  "data": { ... }
}
```

### Error Response

```json
{
  "api": {
    "envelope": "nova.response.v1",
    "nova_version": "0.0.6"
  },
  "success": false,
  "error": "Entity 'customer' not found",
  "error_code": "NOT_FOUND"
}
```

### Ambiguous Name Response

When a name matches multiple entities:

```json
{
  "success": false,
  "error": "Name 'customers' matches 2 entities",
  "error_code": "AMBIGUOUS",
  "matches": [
    "model.analytics.customers",
    "source.raw.customers"
  ],
  "count": 2
}
```

!!! tip "Avoiding Ambiguity"
    Use `unique_id` (e.g., `model.analytics.customers`) instead of just the name for unambiguous lookups.

### Rate Limited Response

```json
{
  "success": false,
  "error": "Rate limit exceeded for tool 'search'",
  "error_code": "RATE_LIMITED"
}
```

### Index Building Response

Returned when the manifest is still being indexed:

```json
{
  "success": false,
  "error": "Index build in progress",
  "error_code": "INDEX_BUILDING",
  "elapsed_ms": 1234
}
```

## Error Details by Code

### NOT_FOUND

Returned when an entity, column, or resource cannot be found.

**Common Causes:**

- Typo in entity name
- Entity doesn't exist in manifest
- Wrong resource type specified

**Example:**

```json
{
  "success": false,
  "error": "Entity 'custmers' not found",
  "error_code": "NOT_FOUND",
  "available_resource_types": ["model", "source", "seed"]
}
```

**Recovery:**

1. Use the `search` tool to find similar entities
2. Check spelling
3. Verify the entity exists in your manifest

### AMBIGUOUS

Returned when a name matches multiple entities across different resource types.

**Example:**

```json
{
  "success": false,
  "error": "Name 'users' matches 3 entities",
  "error_code": "AMBIGUOUS",
  "matches": [
    "model.analytics.users",
    "source.raw.users",
    "seed.analytics.users"
  ],
  "count": 3
}
```

**Recovery:**

1. Use the full `unique_id` from the matches list
2. Specify `resource_type` parameter to disambiguate

### INVALID_PARAMS

Returned when request parameters are invalid.

**Common Causes:**

- Missing required parameters
- Wrong parameter types
- Invalid enum values

**Example:**

```json
{
  "success": false,
  "error": "Invalid parameter: direction must be 'upstream' or 'downstream'",
  "error_code": "INVALID_PARAMS"
}
```

Some validation-heavy tools also return structured `details` for targeted
recovery (for example `run_recipe` preflight):

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

### INDEX_BUILDING

Returned during server startup while the manifest is being indexed.

**Recovery:**

Wait a few seconds and retry. The server will be ready once indexing completes.

## Handling Errors in Code

### Python Example

```python
import time

def call_with_retry(client, tool, params, max_retries=3):
    for attempt in range(max_retries):
        response = client.call_tool(tool, params)

        if response.get("success"):
            return response

        code = response.get("error_code")

        if code == "AMBIGUOUS":
            # Use the first match's unique_id
            unique_id = response["matches"][0]
            params["id_or_name"] = unique_id
            continue

        elif code == "INDEX_BUILDING":
            # Wait and retry
            time.sleep(2)
            continue

        elif code == "RATE_LIMITED":
            # Exponential backoff
            time.sleep(2 ** attempt)
            continue

        else:
            # Non-recoverable error
            raise Exception(f"Error: {response.get('error')}")

    raise Exception("Max retries exceeded")
```

### TypeScript Example

```typescript
async function callWithRetry(
  client: MCPClient,
  tool: string,
  params: Record<string, any>,
  maxRetries = 3
): Promise<any> {
  for (let attempt = 0; attempt < maxRetries; attempt++) {
    const response = await client.callTool(tool, params);

    if (response.success) {
      return response;
    }

    switch (response.error_code) {
      case "AMBIGUOUS":
        params.id_or_name = response.matches[0];
        continue;

      case "INDEX_BUILDING":
      case "RATE_LIMITED":
        await sleep(2000 * Math.pow(2, attempt));
        continue;

      default:
        throw new Error(response.error);
    }
  }

  throw new Error("Max retries exceeded");
}
```

## See Also

- [Response Format](response-format.md) - Full response structure documentation
- [Tools Reference](tools.md) - Tool-specific parameters and responses
- [Configuration](../configuration/reference.md) - Rate limiting configuration
