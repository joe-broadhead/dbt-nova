//! Tests for circuit breaker state transitions.
use std::time::Duration;

use dbt_nova::utils::{CircuitBreaker, CircuitBreakerState};

#[tokio::test(flavor = "multi_thread")]
async fn circuit_breaker_half_open_success_closes() {
    let breaker = CircuitBreaker::new(1, Duration::from_millis(5));

    assert!(breaker.allow().await);
    breaker.on_failure().await;

    let (state, _, _) = breaker.state().await;
    assert_eq!(state, CircuitBreakerState::Open);
    assert!(!breaker.allow().await);

    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(breaker.allow().await);
    assert!(!breaker.allow().await);

    breaker.on_success().await;
    let (state, _, _) = breaker.state().await;
    assert_eq!(state, CircuitBreakerState::Closed);
    assert!(breaker.allow().await);
}

#[tokio::test(flavor = "multi_thread")]
async fn circuit_breaker_half_open_failure_reopens() {
    let breaker = CircuitBreaker::new(1, Duration::from_millis(5));

    assert!(breaker.allow().await);
    breaker.on_failure().await;

    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(breaker.allow().await);
    breaker.on_failure().await;

    let (state, _, _) = breaker.state().await;
    assert_eq!(state, CircuitBreakerState::Open);
    assert!(!breaker.allow().await);
}
