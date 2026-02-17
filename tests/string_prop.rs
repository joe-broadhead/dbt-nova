//! Property tests for string similarity and distance helpers.
use dbt_nova::utils::{levenshtein_distance, levenshtein_similarity};
use proptest::prelude::*;

proptest! {
    #[test]
    fn distance_is_symmetric(a in ".{0,12}", b in ".{0,12}") {
        let d1 = levenshtein_distance(&a, &b);
        let d2 = levenshtein_distance(&b, &a);
        assert_eq!(d1, d2);
    }

    #[test]
    fn distance_respects_length_diff(a in ".{0,12}", b in ".{0,12}") {
        let d = levenshtein_distance(&a, &b) as isize;
        let len_diff = (a.chars().count() as isize - b.chars().count() as isize).abs();
        assert!(d as isize >= len_diff);
    }

    #[test]
    fn similarity_is_symmetric(a in ".{0,12}", b in ".{0,12}") {
        let s1 = levenshtein_similarity(&a, &b);
        let s2 = levenshtein_similarity(&b, &a);
        assert!((s1 - s2).abs() < 1e-9);
    }

    #[test]
    fn similarity_of_identical_is_one(s in ".{0,12}") {
        let sim = levenshtein_similarity(&s, &s);
        assert!((sim - 1.0).abs() < 1e-9);
    }
}
