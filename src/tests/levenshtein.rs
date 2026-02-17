//! Tests for Levenshtein distance and similarity.
use super::common::*;

// Levenshtein Distance Tests
#[test]
fn test_levenshtein_distance_identical() {
    assert_eq!(levenshtein_distance("hello", "hello"), 0);
    assert_eq!(levenshtein_distance("", ""), 0);
}
#[test]
fn test_levenshtein_distance_empty() {
    assert_eq!(levenshtein_distance("hello", ""), 5);
    assert_eq!(levenshtein_distance("", "world"), 5);
}
#[test]
fn test_levenshtein_distance_substitution() {
    // One substitution: kitten -> sitten
    assert_eq!(levenshtein_distance("kitten", "sitten"), 1);
}
#[test]
fn test_levenshtein_distance_insertion_deletion() {
    // "cat" to "cats" = 1 insertion
    assert_eq!(levenshtein_distance("cat", "cats"), 1);
    // "cats" to "cat" = 1 deletion
    assert_eq!(levenshtein_distance("cats", "cat"), 1);
}
#[test]
fn test_levenshtein_distance_complex() {
    // Classic example: kitten -> sitting = 3
    assert_eq!(levenshtein_distance("kitten", "sitting"), 3);
}
#[test]
fn test_levenshtein_similarity() {
    // Identical strings = 1.0
    assert!((levenshtein_similarity("hello", "hello") - 1.0).abs() < 0.001);
    // Completely different = low similarity
    let sim = levenshtein_similarity("abc", "xyz");
    assert!(sim < 0.5);
    // Similar strings = high similarity
    let sim = levenshtein_similarity("customer_id", "customer_ids");
    assert!(sim > 0.9);
}
#[test]
fn test_levenshtein_column_matching() {
    // Simulate what column lineage does
    let source = "customer_id";
    let candidates = ["customer_ids", "customerid", "cust_id", "order_id"];
    let mut matches: Vec<(&str, f64)> = candidates
        .iter()
        .map(|c| (*c, levenshtein_similarity(source, c)))
        .filter(|(_, sim)| *sim >= 0.75)
        .collect();
    matches.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    // customer_ids should be best match (only 1 char different)
    assert!(!matches.is_empty());
    assert_eq!(matches[0].0, "customer_ids");
}
