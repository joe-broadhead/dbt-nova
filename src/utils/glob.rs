/// Simple glob pattern matching
/// Supports: * (matches anything except /), ** (matches anything including /), ? (single char)
#[must_use]
pub fn glob_match(pattern: &str, path: &str) -> bool {
    glob_match_internal(pattern, path, true)
}

/// Simple glob matching for * and ?
#[must_use]
pub fn glob_match_simple(pattern: &str, text: &str) -> bool {
    glob_match_internal(pattern, text, false)
}

#[derive(Debug, Clone)]
pub struct GlobPattern {
    tokens: Vec<GlobToken>,
    literal: Option<String>,
}

#[must_use]
pub fn compile_glob(pattern: &str, allow_globstar: bool) -> GlobPattern {
    let pattern = strip_leading_dot_slash(pattern);
    let has_wildcards = pattern.chars().any(|c| c == '*' || c == '?');
    if !has_wildcards {
        return GlobPattern {
            tokens: Vec::new(),
            literal: Some(pattern.to_string()),
        };
    }
    let tokens = tokenize(pattern, allow_globstar);
    GlobPattern {
        tokens,
        literal: None,
    }
}

#[must_use]
pub fn glob_match_compiled(pattern: &GlobPattern, text: &str) -> bool {
    let text = strip_leading_dot_slash(text);
    if let Some(literal) = &pattern.literal {
        return literal == text;
    }
    glob_match_tokens(&pattern.tokens, text)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GlobToken {
    Char(char),
    Star,
    GlobStar,
    Question,
}

fn glob_match_internal(pattern: &str, text: &str, allow_globstar: bool) -> bool {
    let compiled = compile_glob(pattern, allow_globstar);
    glob_match_compiled(&compiled, text)
}

fn glob_match_tokens(tokens: &[GlobToken], text: &str) -> bool {
    let t: Vec<char> = text.chars().collect();
    let m = t.len();

    let mut prev = vec![false; m + 1];
    prev[0] = true;

    for token in tokens {
        let mut curr = vec![false; m + 1];
        match token {
            GlobToken::Star | GlobToken::GlobStar => {
                curr[0] = prev[0];
            }
            _ => {}
        }
        for j in 1..=m {
            curr[j] = match token {
                GlobToken::Star => prev[j] || (t[j - 1] != '/' && curr[j - 1]),
                GlobToken::GlobStar => prev[j] || curr[j - 1],
                GlobToken::Question => t[j - 1] != '/' && prev[j - 1],
                GlobToken::Char(c) => *c == t[j - 1] && prev[j - 1],
            };
        }
        prev = curr;
    }

    prev[m]
}

fn strip_leading_dot_slash(value: &str) -> &str {
    if let Some(stripped) = value.strip_prefix("./") {
        stripped
    } else {
        value
    }
}

fn tokenize(pattern: &str, allow_globstar: bool) -> Vec<GlobToken> {
    let mut tokens = Vec::new();
    let mut chars = pattern.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '*' => {
                if allow_globstar {
                    let mut star_count = 1;
                    while let Some('*') = chars.peek() {
                        chars.next();
                        star_count += 1;
                    }
                    if star_count >= 2 {
                        tokens.push(GlobToken::GlobStar);
                    } else {
                        tokens.push(GlobToken::Star);
                    }
                } else {
                    tokens.push(GlobToken::Star);
                }
            }
            '?' => tokens.push(GlobToken::Question),
            c => tokens.push(GlobToken::Char(c)),
        }
    }

    tokens
}
