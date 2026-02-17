/// Calculate Levenshtein edit distance between two strings
/// Returns the minimum number of single-character edits (insertions, deletions, substitutions)
#[must_use]
pub fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a_len = a.chars().count();
    let b_len = b.chars().count();

    if a_len == 0 {
        return b_len;
    }
    if b_len == 0 {
        return a_len;
    }

    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();

    // Use two rows instead of full matrix for memory efficiency
    let mut prev_row: Vec<usize> = (0..=b_len).collect();
    let mut curr_row: Vec<usize> = vec![0; b_len + 1];

    for i in 1..=a_len {
        curr_row[0] = i;

        for j in 1..=b_len {
            let cost = usize::from(a_chars[i - 1] != b_chars[j - 1]);

            curr_row[j] = (prev_row[j] + 1) // deletion
                .min(curr_row[j - 1] + 1) // insertion
                .min(prev_row[j - 1] + cost); // substitution
        }

        std::mem::swap(&mut prev_row, &mut curr_row);
    }

    prev_row[b_len]
}

/// Calculate normalized similarity score (0.0 to 1.0) from Levenshtein distance
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn levenshtein_similarity(a: &str, b: &str) -> f64 {
    let max_len = a.chars().count().max(b.chars().count());
    if max_len == 0 {
        return 1.0;
    }
    let distance = levenshtein_distance(a, b);
    1.0 - (distance as f64 / max_len as f64)
}

/// Tokenize input into lowercase alphanumeric terms with a minimum length.
#[must_use]
pub fn tokenize_alnum_lowercase(input: &str, min_len: usize) -> Vec<String> {
    input
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= min_len)
        .map(ToString::to_string)
        .collect()
}

/// Check if the query includes explicit syntax (operators, quotes, or wildcards).
#[must_use]
pub fn has_query_syntax(input: &str) -> bool {
    let upper = input.to_uppercase();
    input.contains('"')
        || input.contains('\'')
        || input.contains(':')
        || input.contains('(')
        || input.contains(')')
        || input.contains('*')
        || input.contains('~')
        || input.contains('+')
        || input.contains('[')
        || input.contains(']')
        || input.contains('{')
        || input.contains('}')
        || upper.contains(" AND ")
        || upper.contains(" OR ")
        || upper.contains(" NOT ")
        || input.contains(" -")
}
