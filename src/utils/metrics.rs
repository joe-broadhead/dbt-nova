use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;
use tracing::warn;

const BUCKET_EDGES_MS: [u64; 6] = [5, 10, 50, 100, 500, 1000];
const BUCKET_COUNT: usize = BUCKET_EDGES_MS.len() + 1;

#[derive(Debug)]
pub struct ToolMetrics {
    calls: AtomicU64,
    errors: AtomicU64,
    total_ms: AtomicU64,
    max_ms: AtomicU64,
    buckets: [AtomicU64; BUCKET_COUNT],
}

impl ToolMetrics {
    #[must_use]
    pub fn new() -> Self {
        Self {
            calls: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            total_ms: AtomicU64::new(0),
            max_ms: AtomicU64::new(0),
            buckets: std::array::from_fn(|_| AtomicU64::new(0)),
        }
    }

    pub fn record(&self, duration_ms: u64, success: bool) {
        self.calls.fetch_add(1, Ordering::Relaxed);
        if !success {
            self.errors.fetch_add(1, Ordering::Relaxed);
        }
        self.total_ms.fetch_add(duration_ms, Ordering::Relaxed);
        self.update_max(duration_ms);
        let idx = bucket_index(duration_ms);
        self.buckets[idx].fetch_add(1, Ordering::Relaxed);
    }

    #[must_use]
    pub fn snapshot(&self) -> Value {
        let calls = self.calls.load(Ordering::Relaxed);
        let errors = self.errors.load(Ordering::Relaxed);
        let total_ms = self.total_ms.load(Ordering::Relaxed);
        let max_ms = self.max_ms.load(Ordering::Relaxed);
        let avg_ms = total_ms.checked_div(calls).unwrap_or(0);
        let p95_ms = percentile_ms(&self.buckets, calls, 95, 100, max_ms);
        let p99_ms = percentile_ms(&self.buckets, calls, 99, 100, max_ms);
        let error_rate_bps = errors
            .saturating_mul(10_000)
            .checked_div(calls)
            .unwrap_or(0);

        let mut buckets = serde_json::Map::new();
        for (idx, count) in self.buckets.iter().enumerate() {
            let label = bucket_label(idx);
            buckets.insert(
                label.to_string(),
                Value::from(count.load(Ordering::Relaxed)),
            );
        }

        serde_json::json!({
            "calls": calls,
            "errors": errors,
            "error_rate_bps": error_rate_bps,
            "total_ms": total_ms,
            "avg_ms": avg_ms,
            "p95_ms": p95_ms,
            "p99_ms": p99_ms,
            "max_ms": max_ms,
            "buckets": buckets
        })
    }

    fn update_max(&self, value: u64) {
        let mut current = self.max_ms.load(Ordering::Relaxed);
        while value > current {
            match self
                .max_ms
                .compare_exchange(current, value, Ordering::Relaxed, Ordering::Relaxed)
            {
                Ok(_) => break,
                Err(next) => current = next,
            }
        }
    }
}

impl Default for ToolMetrics {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Default)]
pub struct ToolMetricsStore {
    inner: Mutex<HashMap<String, Arc<ToolMetrics>>>,
}

#[derive(Debug)]
pub struct ToolRateLimiter {
    window: Duration,
    limits: HashMap<String, u64>,
    default_limit: Option<u64>,
    state: Mutex<HashMap<String, RateState>>,
}

#[derive(Debug, Clone)]
struct RateState {
    window_start: Instant,
    count: u64,
}

impl ToolRateLimiter {
    #[must_use]
    pub fn new(window: Duration, limits: HashMap<String, u64>, default_limit: Option<u64>) -> Self {
        Self {
            window,
            limits,
            default_limit,
            state: Mutex::new(HashMap::new()),
        }
    }

    #[must_use]
    pub fn allow(&self, tool: &str) -> bool {
        let tool_key = tool.to_lowercase();
        let limit = match self.limits.get(&tool_key) {
            Some(limit) => Some(*limit),
            None => self.default_limit,
        };

        let Some(limit) = limit else {
            return true;
        };
        if limit == 0 {
            return true;
        }

        let mut guard = self.state.lock().unwrap_or_else(|poisoned| {
            warn!("Rate limiter lock poisoned, recovering");
            poisoned.into_inner()
        });
        let entry = guard.entry(tool_key).or_insert_with(|| RateState {
            window_start: Instant::now(),
            count: 0,
        });
        let now = Instant::now();
        if now.duration_since(entry.window_start) >= self.window {
            entry.window_start = now;
            entry.count = 0;
        }
        if entry.count >= limit {
            return false;
        }
        entry.count += 1;
        true
    }

    #[must_use]
    pub fn parse_limits(raw: &str) -> (HashMap<String, u64>, Option<u64>) {
        let mut limits = HashMap::new();
        let mut default_limit = None;
        for pair in raw.split(',') {
            let pair = pair.trim();
            if pair.is_empty() {
                continue;
            }
            let mut parts = pair.splitn(2, '=');
            let key = parts.next().unwrap_or("").trim().to_lowercase();
            let value = parts.next().unwrap_or("").trim();
            let Ok(limit) = value.parse::<u64>() else {
                continue;
            };
            if key == "default" {
                default_limit = Some(limit);
            } else {
                limits.insert(key, limit);
            }
        }
        (limits, default_limit)
    }
}

impl ToolMetricsStore {
    pub fn record(&self, key: &str, duration_ms: u64, success: bool) {
        let metrics = {
            let mut guard = self.inner.lock().unwrap_or_else(|poisoned| {
                warn!("Tool metrics lock poisoned, recovering");
                poisoned.into_inner()
            });
            guard
                .entry(key.to_string())
                .or_insert_with(|| Arc::new(ToolMetrics::new()))
                .clone()
        };
        metrics.record(duration_ms, success);
    }

    #[must_use]
    pub fn snapshot(&self) -> Value {
        let guard = self.inner.lock().unwrap_or_else(|poisoned| {
            warn!("Tool metrics lock poisoned, recovering");
            poisoned.into_inner()
        });
        let mut out = serde_json::Map::new();
        for (key, metrics) in guard.iter() {
            out.insert(key.clone(), metrics.snapshot());
        }
        Value::Object(out)
    }
}

fn bucket_index(duration_ms: u64) -> usize {
    for (idx, edge) in BUCKET_EDGES_MS.iter().enumerate() {
        if duration_ms <= *edge {
            return idx;
        }
    }
    BUCKET_EDGES_MS.len()
}

fn bucket_label(idx: usize) -> &'static str {
    match idx {
        0 => "<=5ms",
        1 => "<=10ms",
        2 => "<=50ms",
        3 => "<=100ms",
        4 => "<=500ms",
        5 => "<=1000ms",
        _ => ">1000ms",
    }
}

fn percentile_ms(
    buckets: &[AtomicU64; BUCKET_COUNT],
    calls: u64,
    numerator: u64,
    denominator: u64,
    max_ms: u64,
) -> Option<u64> {
    if calls == 0 {
        return None;
    }
    let target = calls.saturating_mul(numerator).div_ceil(denominator);
    let mut cumulative = 0u64;
    for (idx, bucket) in buckets.iter().enumerate() {
        cumulative = cumulative.saturating_add(bucket.load(Ordering::Relaxed));
        if cumulative >= target {
            return if idx < BUCKET_EDGES_MS.len() {
                Some(BUCKET_EDGES_MS[idx])
            } else {
                Some(max_ms)
            };
        }
    }
    Some(max_ms)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::thread;
    use std::time::Duration;

    use super::{ToolMetrics, ToolMetricsStore, ToolRateLimiter};

    #[test]
    fn tool_metrics_snapshot_tracks_counts_and_buckets() {
        let metrics = ToolMetrics::new();
        metrics.record(4, true);
        metrics.record(1250, false);

        let snapshot = metrics.snapshot();
        assert_eq!(snapshot["calls"].as_u64(), Some(2));
        assert_eq!(snapshot["errors"].as_u64(), Some(1));
        assert_eq!(snapshot["total_ms"].as_u64(), Some(1254));
        assert_eq!(snapshot["avg_ms"].as_u64(), Some(627));
        assert_eq!(snapshot["max_ms"].as_u64(), Some(1250));
        assert_eq!(snapshot["error_rate_bps"].as_u64(), Some(5000));
        assert_eq!(snapshot["buckets"]["<=5ms"].as_u64(), Some(1));
        assert_eq!(snapshot["buckets"][">1000ms"].as_u64(), Some(1));
        assert_eq!(snapshot["p95_ms"].as_u64(), Some(1250));
        assert_eq!(snapshot["p99_ms"].as_u64(), Some(1250));
    }

    #[test]
    fn tool_metrics_store_records_multiple_keys() {
        let store = ToolMetricsStore::default();
        store.record("search", 10, true);
        store.record("search", 20, false);
        store.record("health", 5, true);

        let snapshot = store.snapshot();
        assert_eq!(snapshot["search"]["calls"].as_u64(), Some(2));
        assert_eq!(snapshot["search"]["errors"].as_u64(), Some(1));
        assert_eq!(snapshot["health"]["calls"].as_u64(), Some(1));
        assert_eq!(snapshot["health"]["errors"].as_u64(), Some(0));
    }

    #[test]
    fn rate_limiter_parses_and_enforces_limits_with_window_reset() {
        let (limits, default_limit) =
            ToolRateLimiter::parse_limits("search=2, list_tags=3, default=1, bad=NaN");
        assert_eq!(limits.get("search"), Some(&2));
        assert_eq!(limits.get("list_tags"), Some(&3));
        assert_eq!(default_limit, Some(1));

        let limiter = ToolRateLimiter::new(Duration::from_millis(25), limits, default_limit);

        assert!(limiter.allow("search"));
        assert!(limiter.allow("search"));
        assert!(!limiter.allow("search"));

        assert!(limiter.allow("list_tags"));
        assert!(limiter.allow("list_tags"));
        assert!(limiter.allow("list_tags"));
        assert!(!limiter.allow("list_tags"));

        assert!(limiter.allow("unknown_tool"));
        assert!(!limiter.allow("unknown_tool"));

        thread::sleep(Duration::from_millis(35));
        assert!(limiter.allow("search"));
        assert!(limiter.allow("unknown_tool"));
    }

    #[test]
    fn rate_limiter_without_limits_allows_all_calls() {
        let limiter = ToolRateLimiter::new(Duration::from_secs(1), HashMap::new(), None);
        for _ in 0..10 {
            assert!(limiter.allow("any_tool"));
        }
    }
}
