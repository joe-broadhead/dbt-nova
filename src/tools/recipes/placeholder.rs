use std::collections::HashMap;

use serde_json::Value as JsonValue;

use crate::error::{DbtNovaError, Result};

pub(super) fn normalize_recipe_query_key(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

pub(super) fn resolve_placeholder_value_type<'a>(
    key: &str,
    parameter_types: Option<&'a HashMap<String, String>>,
) -> Option<&'a str> {
    let lookup = [
        key.to_string(),
        key.to_ascii_lowercase(),
        key.to_ascii_uppercase(),
        normalize_recipe_query_key(key),
    ];

    parameter_types.and_then(|types| {
        for candidate in &lookup {
            if let Some(value) = types.get(candidate) {
                return Some(value.as_str());
            }
        }
        None
    })
}

fn get_parameter_value<'a>(
    key: &str,
    parameters: &'a HashMap<String, JsonValue>,
) -> Option<&'a JsonValue> {
    let lookup = [
        key.to_string(),
        key.to_ascii_lowercase(),
        key.to_ascii_uppercase(),
        normalize_recipe_query_key(key),
    ];
    for candidate in &lookup {
        if let Some(value) = parameters.get(candidate) {
            return Some(value);
        }
    }
    None
}

fn is_token_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

pub(super) fn parse_placeholder_at(text: &str, index: usize) -> Option<(usize, usize, String)> {
    let bytes = text.as_bytes();
    let len = bytes.len();
    if index + 1 >= len || bytes[index] != b'_' || bytes[index + 1] != b'_' {
        return None;
    }
    let name_start = index + 2;
    if name_start >= len || !is_token_char(bytes[name_start]) {
        return None;
    }

    let mut name_end = name_start;
    while name_end + 1 < len {
        if bytes[name_end] == b'_' && bytes[name_end + 1] == b'_' {
            break;
        }
        if !is_token_char(bytes[name_end]) {
            return None;
        }
        name_end += 1;
    }

    if name_end + 1 >= len || bytes[name_end] != b'_' || bytes[name_end + 1] != b'_' {
        return None;
    }

    let token = std::str::from_utf8(&bytes[name_start..name_end]).ok()?;
    Some((index, name_end + 2, token.to_string()))
}

fn is_wrapped_by_quote(text: &str, start: usize, end: usize, quote: u8) -> bool {
    start > 0
        && end < text.len()
        && text.as_bytes()[start - 1] == quote
        && text.as_bytes()[end] == quote
}

fn escape_sql_single_quote(value: &str) -> String {
    value.replace('\'', "''")
}

fn quote_or_escape_string(value: &str, is_stringly_quoted: bool) -> String {
    if is_stringly_quoted {
        escape_sql_single_quote(value)
    } else {
        format!("'{}'", escape_sql_single_quote(value))
    }
}

fn sanitize_sql_identifier(value: &str) -> Result<String> {
    let normalized = value.trim();
    if normalized.is_empty() {
        return Err(DbtNovaError::InvalidParams(
            "Identifier parameter is empty".to_string(),
        ));
    }
    if !normalized
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '`' | '"'))
    {
        return Err(DbtNovaError::InvalidParams(format!(
            "Identifier parameter contains invalid characters: {normalized}"
        )));
    }
    Ok(normalized.to_string())
}

pub(super) fn coerce_placeholder_value(
    value: &JsonValue,
    placeholder_type: &str,
    is_quoted: bool,
) -> Result<String> {
    match placeholder_type {
        "identifier" | "ident" | "id" => match value {
            JsonValue::String(text) => Ok(sanitize_sql_identifier(text)?),
            JsonValue::Number(number) => Ok(number.to_string()),
            JsonValue::Bool(flag) => Ok(flag.to_string()),
            _ => Err(DbtNovaError::InvalidParams(format!(
                "Identifier placeholder expects a string or numeric JSON value: {value}"
            ))),
        },
        "number" | "numeric" | "int" | "integer" | "float" | "decimal" => match value {
            JsonValue::Number(number) => Ok(number.to_string()),
            JsonValue::String(text) => {
                if text.parse::<f64>().is_ok() {
                    Ok(text.clone())
                } else {
                    Err(DbtNovaError::InvalidParams(format!(
                        "Expected numeric value for placeholder, got: {text}"
                    )))
                }
            }
            JsonValue::Bool(flag) => Ok(if *flag {
                "1".to_string()
            } else {
                "0".to_string()
            }),
            _ => Err(DbtNovaError::InvalidParams(format!(
                "Expected numeric value for placeholder: {value}"
            ))),
        },
        "boolean" | "bool" => match value {
            JsonValue::Bool(flag) => Ok(if *flag {
                "true".to_string()
            } else {
                "false".to_string()
            }),
            JsonValue::String(text) => {
                let value = text.trim().to_ascii_lowercase();
                match value.as_str() {
                    "true" | "t" | "1" => Ok("true".to_string()),
                    "false" | "f" | "0" => Ok("false".to_string()),
                    _ => Err(DbtNovaError::InvalidParams(format!(
                        "Expected boolean string for placeholder, got: {text}"
                    ))),
                }
            }
            JsonValue::Number(number) => Ok(if number.to_string() == "0" {
                "false"
            } else {
                "true"
            }
            .to_string()),
            _ => Err(DbtNovaError::InvalidParams(format!(
                "Expected boolean value for placeholder: {value}"
            ))),
        },
        "raw" | "expression" | "sql" => match value {
            JsonValue::String(text) => Ok(text.clone()),
            _ => Ok(value.to_string()),
        },
        _ => match value {
            JsonValue::String(text) => Ok(quote_or_escape_string(text, is_quoted)),
            JsonValue::Bool(flag) => Ok(if *flag {
                "true".to_string()
            } else {
                "false".to_string()
            }),
            JsonValue::Number(number) => Ok(number.to_string()),
            JsonValue::Null => Ok("NULL".to_string()),
            _ => Ok(quote_or_escape_string(&value.to_string(), is_quoted)),
        },
    }
}

pub(super) fn apply_runtime_parameter_substitution(
    sql: &str,
    parameters: &HashMap<String, JsonValue>,
    parameter_types: Option<&HashMap<String, String>>,
) -> Result<String> {
    if parameters.is_empty() {
        return Ok(sql.to_string());
    }

    let mut output = String::with_capacity(sql.len());
    let bytes = sql.as_bytes();
    let mut i = 0usize;

    while i < bytes.len() {
        if let Some((start, end, name)) = parse_placeholder_at(sql, i) {
            let placeholder_value = get_parameter_value(&name, parameters).ok_or_else(|| {
                DbtNovaError::InvalidParams(format!(
                    "Missing runtime parameter for placeholder '__{name}__'"
                ))
            })?;
            let placeholder_type = resolve_placeholder_value_type(&name, parameter_types)
                .unwrap_or("auto")
                .to_ascii_lowercase();
            let quoted = is_wrapped_by_quote(sql, start, end, b'\'')
                || is_wrapped_by_quote(sql, start, end, b'"');
            let replacement =
                coerce_placeholder_value(placeholder_value, &placeholder_type, quoted)?;
            output.push_str(&replacement);
            i = end;
            continue;
        }

        let Some(next_char) = sql[i..].chars().next() else {
            return Err(DbtNovaError::ServerError(
                "Invalid SQL payload encoding".to_string(),
            ));
        };
        let next_width = next_char.len_utf8();
        output.push_str(&sql[i..i + next_width]);
        i += next_width;
    }

    Ok(output)
}

pub(super) fn contains_query_placeholders(sql: &str) -> bool {
    let bytes = sql.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if parse_placeholder_at(sql, i).is_some() {
            return true;
        }

        let Some(next_char) = sql[i..].chars().next() else {
            break;
        };
        i += next_char.len_utf8();
    }
    false
}
