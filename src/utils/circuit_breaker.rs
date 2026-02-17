use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// Circuit breaker states for optional subsystems.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitBreakerState {
    Closed,
    Open,
    HalfOpen,
}

#[derive(Debug)]
struct CircuitState {
    failures: usize,
    opened_at: Option<Instant>,
    state: CircuitBreakerState,
    half_open_trial: bool,
}

/// Simple async circuit breaker with half-open probing.
#[derive(Debug)]
pub struct CircuitBreaker {
    failure_threshold: usize,
    open_duration: Duration,
    state: Mutex<CircuitState>,
}

impl CircuitBreaker {
    /// Create a new circuit breaker with failure threshold and open duration.
    #[must_use]
    pub fn new(failure_threshold: usize, open_duration: Duration) -> Self {
        Self {
            failure_threshold: failure_threshold.max(1),
            open_duration,
            state: Mutex::new(CircuitState {
                failures: 0,
                opened_at: None,
                state: CircuitBreakerState::Closed,
                half_open_trial: false,
            }),
        }
    }

    /// Whether a request is allowed under the current breaker state.
    #[must_use]
    pub async fn allow(&self) -> bool {
        let mut state = self.state.lock().await;
        match state.state {
            CircuitBreakerState::Closed => true,
            CircuitBreakerState::Open => {
                if let Some(opened_at) = state.opened_at {
                    if opened_at.elapsed() >= self.open_duration {
                        state.state = CircuitBreakerState::HalfOpen;
                        state.half_open_trial = false;
                    } else {
                        return false;
                    }
                } else {
                    return false;
                }
                if state.half_open_trial {
                    false
                } else {
                    state.half_open_trial = true;
                    true
                }
            }
            CircuitBreakerState::HalfOpen => {
                if state.half_open_trial {
                    false
                } else {
                    state.half_open_trial = true;
                    true
                }
            }
        }
    }

    /// Record a successful operation and close the breaker.
    pub async fn on_success(&self) {
        let mut state = self.state.lock().await;
        state.failures = 0;
        state.opened_at = None;
        state.state = CircuitBreakerState::Closed;
        state.half_open_trial = false;
    }

    /// Record a failure and open the breaker if thresholds are exceeded.
    pub async fn on_failure(&self) {
        let mut state = self.state.lock().await;
        state.failures = state.failures.saturating_add(1);
        if state.state == CircuitBreakerState::HalfOpen {
            state.state = CircuitBreakerState::Open;
            state.opened_at = Some(Instant::now());
            state.half_open_trial = false;
            return;
        }
        if state.failures >= self.failure_threshold {
            state.state = CircuitBreakerState::Open;
            state.opened_at = Some(Instant::now());
        }
    }

    /// Snapshot current state, failure count, and open duration.
    #[must_use]
    pub async fn state(&self) -> (CircuitBreakerState, usize, Option<Duration>) {
        let state = self.state.lock().await;
        let elapsed = state.opened_at.map(|opened_at| opened_at.elapsed());
        (state.state, state.failures, elapsed)
    }
}
