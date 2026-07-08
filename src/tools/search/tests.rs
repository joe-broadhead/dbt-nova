use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::error::DbtNovaError;

use super::columns::resource_type_allowed_for_search;
use super::full::{
    SEMANTIC_TIMEOUT_RESERVE_MS, SearchDeadline, SemanticWorkResult, analyst_near_tie_hint,
    run_semantic_blocking,
};
use super::types::SearchCandidate;

#[test]
fn analyst_near_tie_hint_present_for_close_scores() {
    let candidates = vec![
        SearchCandidate {
            unique_id: "model.analytics.orders".to_string(),
            entity: None,
            score: 10.0,
            support_signals: None,
            indicator_parent_score: None,
            explain: None,
        },
        SearchCandidate {
            unique_id: "model.analytics.sessions".to_string(),
            entity: None,
            score: 9.4,
            support_signals: None,
            indicator_parent_score: None,
            explain: None,
        },
    ];
    let hint = analyst_near_tie_hint(&candidates).expect("near tie hint");
    assert!(hint.contains("Top candidates"));
    assert!(hint.contains("get_entity"));
}

#[test]
fn analyst_near_tie_hint_absent_for_clear_winner() {
    let candidates = vec![
        SearchCandidate {
            unique_id: "model.analytics.orders".to_string(),
            entity: None,
            score: 10.0,
            support_signals: None,
            indicator_parent_score: None,
            explain: None,
        },
        SearchCandidate {
            unique_id: "model.analytics.sessions".to_string(),
            entity: None,
            score: 6.5,
            support_signals: None,
            indicator_parent_score: None,
            explain: None,
        },
    ];
    assert!(analyst_near_tie_hint(&candidates).is_none());
}

#[test]
fn resource_type_filter_enforced() {
    let mut allowed: HashSet<String> = HashSet::new();
    allowed.insert("model".to_string());
    assert!(resource_type_allowed_for_search(
        Some("model"),
        Some(&allowed)
    ));
    assert!(!resource_type_allowed_for_search(
        Some("doc"),
        Some(&allowed)
    ));
    assert!(!resource_type_allowed_for_search(None, Some(&allowed)));
}

#[test]
fn resource_type_filter_skipped_when_not_provided() {
    assert!(resource_type_allowed_for_search(Some("model"), None));
    assert!(resource_type_allowed_for_search(Some("doc"), None));
    assert!(resource_type_allowed_for_search(None, None));
}

#[test]
fn search_deadline_reserves_time_before_semantic_work() {
    let deadline = SearchDeadline {
        started_at: Instant::now()
            .checked_sub(Duration::from_millis(95))
            .expect("test instant subtraction should fit"),
        timeout_ms: 100,
    };

    assert!(deadline.semantic_remaining().is_none());
}

#[tokio::test]
async fn semantic_blocking_skips_when_deadline_is_exhausted() {
    let ran = Arc::new(AtomicBool::new(false));
    let deadline = SearchDeadline {
        started_at: Instant::now()
            .checked_sub(Duration::from_millis(95))
            .expect("test instant subtraction should fit"),
        timeout_ms: 100,
    };
    let ran_in_task = Arc::clone(&ran);

    let result = run_semantic_blocking(deadline, "vector search", "test", move || {
        ran_in_task.store(true, Ordering::SeqCst);
        Ok::<_, DbtNovaError>(())
    })
    .await
    .expect("deadline helper should not fail");

    assert!(matches!(result, SemanticWorkResult::SkippedDeadline));
    assert!(!ran.load(Ordering::SeqCst));
}

#[tokio::test]
async fn semantic_blocking_reports_timeout_after_start() {
    let deadline = SearchDeadline {
        started_at: Instant::now(),
        timeout_ms: SEMANTIC_TIMEOUT_RESERVE_MS + 5,
    };

    let result = run_semantic_blocking(deadline, "vector search", "test", move || {
        std::thread::sleep(Duration::from_millis(50));
        Ok::<_, DbtNovaError>(())
    })
    .await
    .expect("deadline helper should not fail");

    assert!(matches!(result, SemanticWorkResult::TimedOut));
}
