use std::collections::{HashMap, HashSet};

use serde_json::Value as JsonValue;
use sqlparser::ast::{
    Expr, FunctionArg, FunctionArgExpr, FunctionArguments, SelectItem, SetExpr, Statement,
};
use sqlparser::dialect::GenericDialect;
use sqlparser::parser::Parser;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SqlColumnDefinition {
    pub(crate) expression: String,
    pub(crate) identifiers: Vec<String>,
    pub(crate) confidence: &'static str,
}

/// Return the SQL string used for column lineage matching.
pub(crate) fn sql_for_matching(sql_entity: &JsonValue) -> Option<&str> {
    sql_entity
        .get("compiled_code")
        .and_then(|s| s.as_str())
        .or_else(|| sql_entity.get("raw_code").and_then(|s| s.as_str()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RefCall {
    pub package: Option<String>,
    pub name: String,
}

/// Extract dbt `ref(...)` targets from SQL/Jinja text.
pub(crate) fn extract_ref_calls(raw_sql: &str) -> Vec<RefCall> {
    let mut out = Vec::new();
    let mut seen: HashSet<(Option<String>, String)> = HashSet::new();
    for call in extract_macro_calls(raw_sql, "ref") {
        if call.args.is_empty() {
            continue;
        }
        let name = call.args.last().cloned().unwrap_or_default();
        let name = name.trim().to_string();
        if name.is_empty() {
            continue;
        }
        let package = if call.args.len() >= 2 {
            call.args
                .get(call.args.len().saturating_sub(2))
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        } else {
            None
        };
        let key = (package.clone(), name.clone());
        if seen.insert(key) {
            out.push(RefCall { package, name });
        }
    }
    out
}

/// Parse SQL and return alias -> source identifier mappings.
pub(crate) fn find_sql_aliases(raw_sql: &str) -> HashMap<String, String> {
    let dialect = GenericDialect {};
    let mut aliases = HashMap::new();
    let normalized_sql = normalize_sql_for_parser(raw_sql);

    if let Ok(statements) = Parser::parse_sql(&dialect, &normalized_sql) {
        for stmt in statements {
            visit_select_items(&stmt, &mut |item| {
                if let SelectItem::ExprWithAlias { expr, alias } = item {
                    let mut identifiers = Vec::new();
                    collect_identifiers(expr, &mut identifiers);
                    if identifiers.is_empty() {
                        return;
                    }
                    let alias_key = alias.value.to_lowercase();
                    for ident in identifiers {
                        let ident_key = ident.to_lowercase();
                        aliases
                            .entry(ident_key.clone())
                            .or_insert(alias_key.clone());
                        aliases.entry(alias_key.clone()).or_insert(ident_key);
                    }
                }
            });
        }
    }

    aliases
}

/// Extract the defining select-list expression for a projected column.
///
/// This intentionally inspects only the outer projection. It is evidence for
/// unresolved column lineage, not a full SQL optimizer or semantic lineage graph.
pub(crate) fn find_select_column_definition(
    raw_sql: &str,
    column_name: &str,
) -> Option<SqlColumnDefinition> {
    let target = column_name.trim();
    if target.is_empty() {
        return None;
    }

    let dialect = GenericDialect {};
    let normalized_sql = normalize_sql_for_parser(raw_sql);
    if let Ok(statements) = Parser::parse_sql(&dialect, &normalized_sql) {
        for stmt in statements {
            let mut definition = None;
            visit_select_items(&stmt, &mut |item| {
                if definition.is_none() {
                    definition = definition_from_select_item(item, target, "exact");
                }
            });
            if definition.is_some() {
                return definition;
            }
        }
    }

    find_select_column_definition_best_effort(raw_sql, target)
}

fn definition_from_select_item(
    item: &SelectItem,
    target: &str,
    confidence: &'static str,
) -> Option<SqlColumnDefinition> {
    match item {
        SelectItem::ExprWithAlias { expr, alias } if names_equal(&alias.value, target) => {
            let mut identifiers = Vec::new();
            collect_identifiers(expr, &mut identifiers);
            Some(SqlColumnDefinition {
                expression: expr.to_string(),
                identifiers: dedupe_identifiers(identifiers),
                confidence,
            })
        }
        SelectItem::UnnamedExpr(expr) => output_name_for_expr(expr).and_then(|name| {
            if names_equal(&name, target) {
                let mut identifiers = Vec::new();
                collect_identifiers(expr, &mut identifiers);
                Some(SqlColumnDefinition {
                    expression: expr.to_string(),
                    identifiers: dedupe_identifiers(identifiers),
                    confidence,
                })
            } else {
                None
            }
        }),
        _ => None,
    }
}

fn output_name_for_expr(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Identifier(ident) => Some(ident.value.clone()),
        Expr::CompoundIdentifier(idents) => idents.last().map(|ident| ident.value.clone()),
        Expr::Nested(expr) | Expr::Cast { expr, .. } => output_name_for_expr(expr),
        _ => None,
    }
}

fn find_select_column_definition_best_effort(
    raw_sql: &str,
    target: &str,
) -> Option<SqlColumnDefinition> {
    let projection = outer_projection_text(raw_sql)?;
    for item in split_top_level_commas(projection) {
        let Some(expression) = expression_before_alias(item, target) else {
            continue;
        };
        let identifiers = identifiers_from_text(expression);
        return Some(SqlColumnDefinition {
            expression: expression.trim().to_string(),
            identifiers,
            confidence: "best_effort",
        });
    }
    None
}

fn outer_projection_text(raw_sql: &str) -> Option<&str> {
    let lower = raw_sql.to_ascii_lowercase();
    let select_start = lower.find("select")?;
    let projection_start = select_start + "select".len();
    let bytes = raw_sql.as_bytes();
    let mut depth = 0usize;
    let mut in_single = false;
    let mut in_double = false;
    let mut cursor = projection_start;
    while cursor < bytes.len() {
        let b = bytes[cursor];
        if in_single {
            if b == b'\'' {
                in_single = false;
            }
            cursor += 1;
            continue;
        }
        if in_double {
            if b == b'"' {
                in_double = false;
            }
            cursor += 1;
            continue;
        }
        match b {
            b'\'' => in_single = true,
            b'"' => in_double = true,
            b'(' => depth += 1,
            b')' => depth = depth.saturating_sub(1),
            _ if depth == 0 && lower[cursor..].starts_with(" from ") => {
                return Some(&raw_sql[projection_start..cursor]);
            }
            _ => {}
        }
        cursor += 1;
    }
    None
}

fn split_top_level_commas(input: &str) -> Vec<&str> {
    let bytes = input.as_bytes();
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut in_single = false;
    let mut in_double = false;
    let mut start = 0usize;
    for (idx, b) in bytes.iter().enumerate() {
        if in_single {
            if *b == b'\'' {
                in_single = false;
            }
            continue;
        }
        if in_double {
            if *b == b'"' {
                in_double = false;
            }
            continue;
        }
        match *b {
            b'\'' => in_single = true,
            b'"' => in_double = true,
            b'(' => depth += 1,
            b')' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                parts.push(input[start..idx].trim());
                start = idx + 1;
            }
            _ => {}
        }
    }
    let tail = input[start..].trim();
    if !tail.is_empty() {
        parts.push(tail);
    }
    parts
}

fn expression_before_alias<'a>(item: &'a str, target: &str) -> Option<&'a str> {
    let trimmed = item.trim();
    let lower = trimmed.to_ascii_lowercase();
    let target_lower = target.to_ascii_lowercase();
    let as_suffix = format!(" as {target_lower}");
    if lower.ends_with(&as_suffix) {
        return Some(&trimmed[..trimmed.len().saturating_sub(as_suffix.len())]);
    }

    let bare_suffix = format!(" {target_lower}");
    if lower.ends_with(&bare_suffix) {
        return Some(&trimmed[..trimmed.len().saturating_sub(bare_suffix.len())]);
    }

    None
}

fn identifiers_from_text(input: &str) -> Vec<String> {
    const KEYWORDS: &[&str] = &[
        "and",
        "as",
        "case",
        "cast",
        "count",
        "date",
        "distinct",
        "else",
        "end",
        "false",
        "from",
        "if",
        "in",
        "is",
        "null",
        "nullif",
        "or",
        "over",
        "partition",
        "select",
        "sum",
        "then",
        "true",
        "when",
    ];

    let mut identifiers = Vec::new();
    let mut token = String::new();
    for ch in input.chars().chain(std::iter::once(' ')) {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '.' {
            token.push(ch);
            continue;
        }
        if token
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        {
            let normalized = token.trim_matches('.').to_string();
            let lower = normalized.to_ascii_lowercase();
            if !normalized.is_empty() && !KEYWORDS.contains(&lower.as_str()) {
                identifiers.push(normalized);
            }
        }
        token.clear();
    }
    dedupe_identifiers(identifiers)
}

fn dedupe_identifiers(identifiers: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for identifier in identifiers {
        let key = identifier.to_ascii_lowercase();
        if seen.insert(key) {
            out.push(identifier);
        }
    }
    out
}

fn names_equal(left: &str, right: &str) -> bool {
    left.trim_matches(|c: char| matches!(c, '`' | '"' | '[' | ']'))
        .eq_ignore_ascii_case(right.trim_matches(|c: char| matches!(c, '`' | '"' | '[' | ']')))
}

#[derive(Debug, Clone)]
struct MacroCall {
    start: usize,
    end: usize,
    args: Vec<String>,
}

fn normalize_sql_for_parser(raw_sql: &str) -> String {
    let mut replacements: Vec<(usize, usize, String)> = Vec::new();

    for macro_name in ["ref", "source"] {
        for call in extract_macro_calls(raw_sql, macro_name) {
            let replacement = call.args.last().map_or_else(
                || "__dbt_ref__".to_string(),
                |name| sanitize_identifier(name),
            );
            replacements.push((call.start, call.end, replacement));
        }
    }
    replacements.sort_by_key(|(start, _, _)| *start);

    let mut out = String::with_capacity(raw_sql.len());
    let mut cursor = 0usize;
    for (start, end, replacement) in replacements {
        if start < cursor || start > raw_sql.len() || end > raw_sql.len() || end < start {
            continue;
        }
        out.push_str(&raw_sql[cursor..start]);
        out.push_str(&replacement);
        cursor = end;
    }
    out.push_str(&raw_sql[cursor..]);

    out.chars()
        .map(|ch| match ch {
            '{' | '}' | '%' => ' ',
            _ => ch,
        })
        .collect()
}

fn sanitize_identifier(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return "__dbt_ref__".to_string();
    }
    let mut out = String::with_capacity(trimmed.len());
    for ch in trimmed.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        return "__dbt_ref__".to_string();
    }
    if out.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

fn extract_macro_calls(raw_sql: &str, macro_name: &str) -> Vec<MacroCall> {
    let mut calls = Vec::new();
    let lower = raw_sql.to_ascii_lowercase();
    let bytes = raw_sql.as_bytes();
    let lower_bytes = lower.as_bytes();
    let macro_bytes = macro_name.as_bytes();
    let mut cursor = 0usize;

    while cursor < lower_bytes.len() {
        let Some(found_rel) = lower[cursor..].find(macro_name) else {
            break;
        };
        let start = cursor + found_rel;
        let macro_end = start + macro_bytes.len();
        if !boundary_ok(lower_bytes, start, macro_end) {
            cursor = macro_end;
            continue;
        }

        let mut open_idx = macro_end;
        while open_idx < bytes.len() && bytes[open_idx].is_ascii_whitespace() {
            open_idx += 1;
        }
        if open_idx >= bytes.len() || bytes[open_idx] != b'(' {
            cursor = macro_end;
            continue;
        }

        if let Some(close_idx) = find_matching_paren(bytes, open_idx) {
            let arg_text = &raw_sql[open_idx + 1..close_idx];
            calls.push(MacroCall {
                start,
                end: close_idx + 1,
                args: parse_quoted_args(arg_text),
            });
            cursor = close_idx + 1;
        } else {
            break;
        }
    }

    calls
}

fn boundary_ok(bytes: &[u8], start: usize, end: usize) -> bool {
    let before_ok = if start == 0 {
        true
    } else {
        !is_ident_char(bytes[start - 1])
    };
    let after_ok = if end >= bytes.len() {
        true
    } else {
        !is_ident_char(bytes[end])
    };
    before_ok && after_ok
}

fn is_ident_char(ch: u8) -> bool {
    ch.is_ascii_alphanumeric() || ch == b'_'
}

fn find_matching_paren(bytes: &[u8], open_idx: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;

    for (i, b) in bytes.iter().enumerate().skip(open_idx) {
        if in_single {
            if *b == b'\\' && !escaped {
                escaped = true;
                continue;
            }
            if *b == b'\'' && !escaped {
                in_single = false;
            }
            escaped = false;
            continue;
        }
        if in_double {
            if *b == b'\\' && !escaped {
                escaped = true;
                continue;
            }
            if *b == b'"' && !escaped {
                in_double = false;
            }
            escaped = false;
            continue;
        }

        match *b {
            b'\'' => in_single = true,
            b'"' => in_double = true,
            b'(' => depth += 1,
            b')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }

    None
}

fn parse_quoted_args(input: &str) -> Vec<String> {
    let bytes = input.as_bytes();
    let mut args = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        let quote = bytes[i];
        if quote != b'\'' && quote != b'"' {
            i += 1;
            continue;
        }
        i += 1;
        let start = i;
        let mut escaped = false;
        while i < bytes.len() {
            let b = bytes[i];
            if b == b'\\' && !escaped {
                escaped = true;
                i += 1;
                continue;
            }
            if b == quote && !escaped {
                args.push(input[start..i].to_string());
                i += 1;
                break;
            }
            escaped = false;
            i += 1;
        }
    }
    args
}

#[allow(clippy::too_many_lines)]
fn collect_identifiers(expr: &Expr, out: &mut Vec<String>) {
    match expr {
        Expr::Identifier(ident) => {
            out.push(ident.value.clone());
        }
        Expr::CompoundIdentifier(idents) => {
            if let Some(last) = idents.last() {
                out.push(last.value.clone());
            }
            let full = idents
                .iter()
                .map(|ident| ident.value.as_str())
                .collect::<Vec<_>>()
                .join(".");
            if !full.is_empty() {
                out.push(full);
            }
        }
        Expr::Function(func) => {
            if let FunctionArguments::List(list) = &func.args {
                for arg in &list.args {
                    match arg {
                        FunctionArg::Unnamed(FunctionArgExpr::Expr(inner))
                        | FunctionArg::Named {
                            arg: FunctionArgExpr::Expr(inner),
                            ..
                        } => {
                            collect_identifiers(inner, out);
                        }
                        _ => {}
                    }
                }
            }
        }
        Expr::Cast { expr, .. }
        | Expr::Extract { expr, .. }
        | Expr::Substring { expr, .. }
        | Expr::Nested(expr)
        | Expr::UnaryOp { expr, .. }
        | Expr::IsNull(expr)
        | Expr::IsNotNull(expr)
        | Expr::IsTrue(expr)
        | Expr::IsFalse(expr)
        | Expr::IsUnknown(expr)
        | Expr::IsNotTrue(expr)
        | Expr::IsNotFalse(expr)
        | Expr::IsNotUnknown(expr)
        | Expr::Convert { expr, .. } => {
            collect_identifiers(expr, out);
        }
        Expr::BinaryOp { left, right, .. }
        | Expr::IsDistinctFrom(left, right)
        | Expr::IsNotDistinctFrom(left, right)
        | Expr::Like {
            expr: left,
            pattern: right,
            ..
        }
        | Expr::ILike {
            expr: left,
            pattern: right,
            ..
        }
        | Expr::SimilarTo {
            expr: left,
            pattern: right,
            ..
        } => {
            collect_identifiers(left, out);
            collect_identifiers(right, out);
        }
        Expr::JsonAccess { value, .. } => {
            collect_identifiers(value, out);
        }
        Expr::CompoundFieldAccess { root, .. } => {
            collect_identifiers(root, out);
        }
        Expr::Between {
            expr, low, high, ..
        } => {
            collect_identifiers(expr, out);
            collect_identifiers(low, out);
            collect_identifiers(high, out);
        }
        Expr::InList { expr, list, .. } => {
            collect_identifiers(expr, out);
            for item in list {
                collect_identifiers(item, out);
            }
        }
        Expr::Case {
            operand,
            conditions,
            else_result,
            ..
        } => {
            if let Some(operand) = operand {
                collect_identifiers(operand, out);
            }
            for case_when in conditions {
                collect_identifiers(&case_when.condition, out);
                collect_identifiers(&case_when.result, out);
            }
            if let Some(else_result) = else_result {
                collect_identifiers(else_result, out);
            }
        }
        _ => {}
    }
}

fn visit_select_items<F: FnMut(&SelectItem)>(stmt: &Statement, f: &mut F) {
    if let Statement::Query(query) = stmt
        && let SetExpr::Select(select) = &*query.body
    {
        for item in &select.projection {
            f(item);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{extract_ref_calls, find_select_column_definition, find_sql_aliases};

    #[test]
    fn extract_ref_calls_handles_single_and_qualified_refs() {
        let sql = "select * from {{ ref('base_orders') }} join { ref('pkg', 'int_orders') }";
        let refs = extract_ref_calls(sql);
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].name, "base_orders");
        assert_eq!(refs[0].package.as_deref(), None);
        assert_eq!(refs[1].name, "int_orders");
        assert_eq!(refs[1].package.as_deref(), Some("pkg"));
    }

    #[test]
    fn find_sql_aliases_parses_templated_ref_sql() {
        let sql = r"
            with base as (
              select * from { ref('stg__events') }
            )
            select
              sum(order_completed) / nullif(count(distinct new_session_id), 0) as session_conversion_rate
            from base
        ";
        let aliases = find_sql_aliases(sql);
        assert_eq!(
            aliases.get("order_completed").map(String::as_str),
            Some("session_conversion_rate")
        );
        assert_eq!(
            aliases.get("session_conversion_rate").map(String::as_str),
            Some("order_completed")
        );
    }

    #[test]
    fn find_select_column_definition_extracts_aggregate_expression() {
        let sql = "select channel, count(distinct li.order_id) as orders_count from line_items li group by 1";
        let definition = find_select_column_definition(sql, "orders_count").expect("definition");
        assert_eq!(definition.expression, "count(DISTINCT li.order_id)");
        assert_eq!(definition.identifiers, vec!["order_id", "li.order_id"]);
        assert_eq!(definition.confidence, "exact");
    }

    #[test]
    fn find_select_column_definition_extracts_ratio_expression() {
        let sql = "select sum(gross_amount) / nullif(count(distinct order_id), 0) as average_order_value from orders";
        let definition =
            find_select_column_definition(sql, "average_order_value").expect("definition");
        assert!(
            definition
                .expression
                .contains("sum(gross_amount) / nullif(count(DISTINCT order_id), 0)")
        );
        assert_eq!(definition.identifiers, vec!["gross_amount", "order_id"]);
    }
}
