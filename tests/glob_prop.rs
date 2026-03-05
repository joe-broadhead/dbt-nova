//! Property tests for glob matching behavior.
use dbt_nova::utils::glob_match;
use proptest::prelude::*;

// Patterns without glob metacharacters should behave like strict equality.
proptest! {
    #[test]
    fn exact_pattern_matches_only_identical_paths(
        pattern in "[A-Za-z0-9_./-]{0,20}",
        path in "[A-Za-z0-9_./-]{0,20}",
    ) {
        prop_assume!(!pattern.contains('*'));
        prop_assume!(!pattern.contains('?'));
        prop_assume!(!pattern.contains('['));

        let normalized_pattern = pattern.strip_prefix("./").unwrap_or(&pattern);
        let normalized_path = path.strip_prefix("./").unwrap_or(&path);
        let expected = normalized_pattern == normalized_path;
        let matched = glob_match(&pattern, &path);
        assert_eq!(matched, expected);
    }
}

// Leading "./" on the pattern should not prevent a match against a normalized path.
proptest! {
    #[test]
    fn leading_dot_slash_is_ignored(path in "[A-Za-z0-9_./-]{1,20}") {
        let normalized = path.trim_start_matches("./");
        let pattern = format!("./{}", normalized);
        let matched = glob_match(&pattern, normalized);
        assert!(matched);
    }
}
