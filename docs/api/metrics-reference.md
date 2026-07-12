# Metrics Reference

## Tool Metrics

Each tool call records metrics exposed by the `health` tool under `tool_metrics`.
When Nova runs in streamable HTTP mode, the same recorder is also exposed as
Prometheus-compatible text at `GET /metrics` unless
`DBT_NOVA_METRICS_ENABLED=false`.

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
| `buckets` | object | Non-cumulative latency buckets retained for backward-compatible health JSON |

## Prometheus Scrape

`GET /metrics` returns `text/plain; version=0.0.4` and includes:

```text
# HELP nova_manifest_ready_for_traffic 1 when the active manifest/search index is ready to serve traffic.
# TYPE nova_manifest_ready_for_traffic gauge
nova_manifest_ready_for_traffic 1
# HELP nova_tool_calls_total Total MCP tool calls by tool and result.
# TYPE nova_tool_calls_total counter
nova_tool_calls_total{tool="search",result="success"} 1247
nova_tool_calls_total{tool="search",result="error"} 3
# HELP nova_tool_call_duration_milliseconds MCP tool call duration histogram in milliseconds.
# TYPE nova_tool_call_duration_milliseconds histogram
nova_tool_call_duration_milliseconds_bucket{tool="search",le="5"} 234
nova_tool_call_duration_milliseconds_bucket{tool="search",le="10"} 690
nova_tool_call_duration_milliseconds_bucket{tool="search",le="50"} 1079
nova_tool_call_duration_milliseconds_bucket{tool="search",le="100"} 1177
nova_tool_call_duration_milliseconds_bucket{tool="search",le="500"} 1244
nova_tool_call_duration_milliseconds_bucket{tool="search",le="1000"} 1249
nova_tool_call_duration_milliseconds_bucket{tool="search",le="+Inf"} 1250
nova_tool_call_duration_milliseconds_sum{tool="search"} 45230
nova_tool_call_duration_milliseconds_count{tool="search"} 1250
```

Prometheus histogram buckets are cumulative prefix sums of Nova's internal
non-overlapping health buckets. For example, `le="50"` includes calls counted in
`<=5ms`, `<=10ms`, and `<=50ms`.

Labels are intentionally limited to tool name and result. Nova does not put
query text, entity names, manifest paths, user IDs, or credentials in metrics
labels.

Useful PromQL examples:

```promql
sum(rate(nova_tool_calls_total[5m]))
sum(rate(nova_tool_calls_total{result="error"}[5m]))
histogram_quantile(0.95, sum by (le, tool) (rate(nova_tool_call_duration_milliseconds_bucket[5m])))
nova_manifest_ready_for_traffic == 0
```

`/metrics` is an operator endpoint. In hosted deployments, restrict it with the
same proxy or network ACL as MCP, or set `DBT_NOVA_METRICS_ENABLED=false`.

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
