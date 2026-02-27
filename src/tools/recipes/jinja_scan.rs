#[derive(Clone, Copy)]
enum RecipeSqlScanState {
    Normal,
    SingleQuote,
    DoubleQuote,
    DollarQuote,
    LineComment,
    BlockComment,
}

fn parse_dollar_quote_delimiter(sql: &[u8], start: usize) -> Option<Vec<u8>> {
    if start >= sql.len() || sql[start] != b'$' {
        return None;
    }

    let mut i = start + 1;
    while i < sql.len() {
        let ch = sql[i];
        if ch == b'$' {
            return Some(sql[start..=i].to_vec());
        }
        if ch.is_ascii_alphanumeric() || ch == b'_' {
            i += 1;
            continue;
        }
        return None;
    }

    None
}

fn push_unique_jinja_marker(markers: &mut Vec<&'static str>, marker: &'static str) {
    if !markers.contains(&marker) {
        markers.push(marker);
    }
}

fn detect_jinja_marker(bytes: &[u8], i: usize, markers: &mut Vec<&'static str>) {
    if i + 1 >= bytes.len() || bytes[i] != b'{' {
        return;
    }

    match bytes[i + 1] {
        b'{' => push_unique_jinja_marker(markers, "{{"),
        b'%' => push_unique_jinja_marker(markers, "{%"),
        b'#' => push_unique_jinja_marker(markers, "{#"),
        _ => {}
    }
}

fn step_normal_state(
    bytes: &[u8],
    i: usize,
    markers: &mut Vec<&'static str>,
    dollar_quote_delimiter: &mut Option<Vec<u8>>,
    single_quote_backslash_escape: &mut bool,
) -> (usize, RecipeSqlScanState) {
    if i + 1 < bytes.len() && bytes[i] == b'-' && bytes[i + 1] == b'-' {
        return (i + 2, RecipeSqlScanState::LineComment);
    }
    if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
        return (i + 2, RecipeSqlScanState::BlockComment);
    }
    if bytes[i] == b'\'' {
        *single_quote_backslash_escape = allows_backslash_escaped_quote(bytes, i);
        return (i + 1, RecipeSqlScanState::SingleQuote);
    }
    if bytes[i] == b'"' {
        return (i + 1, RecipeSqlScanState::DoubleQuote);
    }
    if let Some(delimiter) = parse_dollar_quote_delimiter(bytes, i) {
        *dollar_quote_delimiter = Some(delimiter.clone());
        return (i + delimiter.len(), RecipeSqlScanState::DollarQuote);
    }

    detect_jinja_marker(bytes, i, markers);
    (i + 1, RecipeSqlScanState::Normal)
}

fn is_identifier_char(ch: u8) -> bool {
    ch.is_ascii_alphanumeric() || ch == b'_'
}

fn allows_backslash_escaped_quote(bytes: &[u8], quote_index: usize) -> bool {
    if quote_index == 0 {
        return false;
    }

    let prefix_index = quote_index - 1;
    if !matches!(bytes[prefix_index], b'e' | b'E') {
        return false;
    }

    if prefix_index == 0 {
        return true;
    }

    !is_identifier_char(bytes[prefix_index - 1])
}

fn step_single_quote_state(
    bytes: &[u8],
    i: usize,
    backslash_escape_enabled: bool,
) -> (usize, RecipeSqlScanState) {
    if i + 1 < bytes.len() && bytes[i] == b'\'' && bytes[i + 1] == b'\'' {
        return (i + 2, RecipeSqlScanState::SingleQuote);
    }
    // Only treat backslash as an escape when the literal explicitly opts in
    // (for example PostgreSQL E'...').
    if backslash_escape_enabled && bytes[i] == b'\\' {
        return ((i + 2).min(bytes.len()), RecipeSqlScanState::SingleQuote);
    }
    if bytes[i] == b'\'' {
        return (i + 1, RecipeSqlScanState::Normal);
    }
    (i + 1, RecipeSqlScanState::SingleQuote)
}

fn step_double_quote_state(bytes: &[u8], i: usize) -> (usize, RecipeSqlScanState) {
    if i + 1 < bytes.len() && bytes[i] == b'"' && bytes[i + 1] == b'"' {
        return (i + 2, RecipeSqlScanState::DoubleQuote);
    }
    if bytes[i] == b'"' {
        return (i + 1, RecipeSqlScanState::Normal);
    }
    (i + 1, RecipeSqlScanState::DoubleQuote)
}

fn step_line_comment_state(bytes: &[u8], i: usize) -> (usize, RecipeSqlScanState) {
    if bytes[i] == b'\n' {
        return (i + 1, RecipeSqlScanState::Normal);
    }
    (i + 1, RecipeSqlScanState::LineComment)
}

fn step_block_comment_state(bytes: &[u8], i: usize) -> (usize, RecipeSqlScanState) {
    if i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b'/' {
        return (i + 2, RecipeSqlScanState::Normal);
    }
    (i + 1, RecipeSqlScanState::BlockComment)
}

fn step_dollar_quote_state(
    bytes: &[u8],
    i: usize,
    dollar_quote_delimiter: &mut Option<Vec<u8>>,
) -> (usize, RecipeSqlScanState) {
    if let Some((delimiter_len, is_end)) = dollar_quote_delimiter.as_ref().map(|delimiter| {
        let delimiter_len = delimiter.len();
        let is_end =
            i + delimiter_len <= bytes.len() && bytes[i..i + delimiter_len] == delimiter[..];
        (delimiter_len, is_end)
    }) && is_end
    {
        *dollar_quote_delimiter = None;
        return (i + delimiter_len, RecipeSqlScanState::Normal);
    }
    (i + 1, RecipeSqlScanState::DollarQuote)
}

pub(super) fn recipe_query_jinja_markers(sql: &str) -> Vec<&'static str> {
    let mut markers = Vec::new();
    let bytes = sql.as_bytes();
    let mut i = 0usize;
    let mut state = RecipeSqlScanState::Normal;
    let mut dollar_quote_delimiter: Option<Vec<u8>> = None;
    let mut single_quote_backslash_escape = false;

    while i < bytes.len() {
        let (next_i, next_state) = match state {
            RecipeSqlScanState::Normal => step_normal_state(
                bytes,
                i,
                &mut markers,
                &mut dollar_quote_delimiter,
                &mut single_quote_backslash_escape,
            ),
            RecipeSqlScanState::SingleQuote => {
                step_single_quote_state(bytes, i, single_quote_backslash_escape)
            }
            RecipeSqlScanState::DoubleQuote => step_double_quote_state(bytes, i),
            RecipeSqlScanState::LineComment => step_line_comment_state(bytes, i),
            RecipeSqlScanState::BlockComment => step_block_comment_state(bytes, i),
            RecipeSqlScanState::DollarQuote => {
                step_dollar_quote_state(bytes, i, &mut dollar_quote_delimiter)
            }
        };
        i = next_i;
        if matches!(state, RecipeSqlScanState::SingleQuote)
            && !matches!(next_state, RecipeSqlScanState::SingleQuote)
        {
            single_quote_backslash_escape = false;
        }
        state = next_state;
    }

    markers
}

pub(super) fn recipe_query_snippet(sql: &str, max_chars: usize) -> String {
    let collapsed = sql.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut iter = collapsed.chars();
    let snippet: String = iter.by_ref().take(max_chars).collect();
    if iter.next().is_some() {
        format!("{snippet}...")
    } else {
        snippet
    }
}
