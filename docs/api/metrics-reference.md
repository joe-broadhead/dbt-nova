# Metrics Reference

## Tool Metrics

Each tool call records metrics exposed by the `health` tool under `tool_metrics`.

### Schema

```json
{
  "tool_metrics": {
    "search": {
      "calls": 1250,
      "errors": 3,
      "error_rate_bps": 24,
      "total_ms": 45230,
      "avg_ms": 36,
      "p95_ms": 500,
      "p99_ms": 892,
      "max_ms": 892,
      "buckets": {
        "<=5ms": 234,
        "<=10ms": 456,
        "<=50ms": 389,
        "<=100ms": 98,
        "<=500ms": 67,
        "<=1000ms": 5,
        ">1000ms": 1
      }
    }
  }
}
```

### Fields

| Field | Type | Description |
|---|---|---|
| `calls` | integer | Total calls |
| `errors` | integer | Total failed calls |
| `error_rate_bps` | integer | Error rate in basis points (`10000 * errors / calls`) |
| `total_ms` | integer | Cumulative latency |
| `avg_ms` | integer | Mean latency |
| `p95_ms` | integer/null | Approximate p95 latency |
| `p99_ms` | integer/null | Approximate p99 latency |
| `max_ms` | integer | Maximum single-call latency |
| `buckets` | object | Histogram buckets |

## Search Concurrency

`health` also reports search saturation under `search_concurrency`:

```json
{
  "search_concurrency": {
    "enabled": true,
    "max_concurrent": 4,
    "available_slots": 2,
    "in_flight": 2,
    "saturated": false,
    "max_queue": 8,
    "available_queue": 8,
    "queued": 0,
    "queue_saturated": false
  }
}
```

Use this to detect queue pressure and tune:

- `DBT_NOVA_SEARCH_MAX_CONCURRENT`
- `DBT_NOVA_SEARCH_MAX_QUEUE`
