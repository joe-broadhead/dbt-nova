use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use sqlparser::ast::{
    BinaryOperator, Expr, GroupByExpr, Join, JoinConstraint, JoinOperator, Query, Select,
    SelectItem, SetExpr, Statement, TableFactor, TableWithJoins,
};
use sqlparser::dialect::{
    BigQueryDialect, DatabricksDialect, DuckDbDialect, GenericDialect, SnowflakeDialect,
};
use sqlparser::parser::Parser;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct SqlStructureSignature {
    pub(crate) tables: Vec<String>,
    pub(crate) joins: Vec<String>,
    pub(crate) select: Vec<String>,
    pub(crate) filters: Vec<String>,
    pub(crate) group_by: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub(crate) struct SqlStructureDiff {
    pub(crate) missing_tables: Vec<String>,
    pub(crate) unexpected_tables: Vec<String>,
    pub(crate) missing_joins: Vec<String>,
    pub(crate) unexpected_joins: Vec<String>,
    pub(crate) missing_select: Vec<String>,
    pub(crate) unexpected_select: Vec<String>,
    pub(crate) missing_filters: Vec<String>,
    pub(crate) unexpected_filters: Vec<String>,
    pub(crate) missing_group_by: Vec<String>,
    pub(crate) unexpected_group_by: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct SqlStructureComparison {
    pub(crate) matches: bool,
    pub(crate) expected: SqlStructureSignature,
    pub(crate) actual: SqlStructureSignature,
    pub(crate) diff: SqlStructureDiff,
}

impl SqlStructureDiff {
    #[must_use]
    pub(crate) fn changed_clauses(&self) -> Vec<&'static str> {
        let mut clauses = Vec::new();
        if !self.missing_select.is_empty() || !self.unexpected_select.is_empty() {
            clauses.push("SELECT");
        }
        if !self.missing_tables.is_empty() || !self.unexpected_tables.is_empty() {
            clauses.push("FROM");
        }
        if !self.missing_joins.is_empty() || !self.unexpected_joins.is_empty() {
            clauses.push("JOIN");
        }
        if !self.missing_filters.is_empty() || !self.unexpected_filters.is_empty() {
            clauses.push("WHERE");
        }
        if !self.missing_group_by.is_empty() || !self.unexpected_group_by.is_empty() {
            clauses.push("GROUP BY");
        }
        clauses
    }

    fn is_empty(&self) -> bool {
        self.missing_tables.is_empty()
            && self.unexpected_tables.is_empty()
            && self.missing_joins.is_empty()
            && self.unexpected_joins.is_empty()
            && self.missing_select.is_empty()
            && self.unexpected_select.is_empty()
            && self.missing_filters.is_empty()
            && self.unexpected_filters.is_empty()
            && self.missing_group_by.is_empty()
            && self.unexpected_group_by.is_empty()
    }
}

pub(crate) fn sql_structure_signature(sql: &str) -> Result<SqlStructureSignature, String> {
    let statement = parse_single_statement(sql)?;
    let Statement::Query(query) = statement else {
        return Err("query structure grading supports SELECT queries only".to_string());
    };
    signature_for_query(&query)
}

pub(crate) fn sql_structure_summary_json(sql: &str) -> Result<JsonValue, String> {
    serde_json::to_value(sql_structure_signature(sql)?)
        .map_err(|error| format!("failed to serialize SQL structure summary: {error}"))
}

pub(crate) fn compare_sql_structure(
    actual_sql: &str,
    expected_sql: &str,
) -> Result<SqlStructureComparison, String> {
    let actual = sql_structure_signature(actual_sql)?;
    let expected = sql_structure_signature(expected_sql)?;
    Ok(compare_sql_structure_signatures(actual, expected))
}

pub(crate) fn compare_sql_structure_signatures(
    actual: SqlStructureSignature,
    expected: SqlStructureSignature,
) -> SqlStructureComparison {
    let diff = SqlStructureDiff {
        missing_tables: missing_values(&actual.tables, &expected.tables),
        unexpected_tables: unexpected_values(&actual.tables, &expected.tables),
        missing_joins: missing_values(&actual.joins, &expected.joins),
        unexpected_joins: unexpected_values(&actual.joins, &expected.joins),
        missing_select: missing_values(&actual.select, &expected.select),
        unexpected_select: unexpected_values(&actual.select, &expected.select),
        missing_filters: missing_values(&actual.filters, &expected.filters),
        unexpected_filters: unexpected_values(&actual.filters, &expected.filters),
        missing_group_by: missing_values(&actual.group_by, &expected.group_by),
        unexpected_group_by: unexpected_values(&actual.group_by, &expected.group_by),
    };
    SqlStructureComparison {
        matches: diff.is_empty(),
        expected,
        actual,
        diff,
    }
}

fn parse_single_statement(sql: &str) -> Result<Statement, String> {
    let trimmed = sql.trim();
    if trimmed.is_empty() {
        return Err("SQL must be non-empty".to_string());
    }

    let parse_attempts: [(&str, Result<Vec<Statement>, sqlparser::parser::ParserError>); 5] = [
        ("generic", Parser::parse_sql(&GenericDialect {}, trimmed)),
        (
            "snowflake",
            Parser::parse_sql(&SnowflakeDialect {}, trimmed),
        ),
        ("bigquery", Parser::parse_sql(&BigQueryDialect {}, trimmed)),
        ("duckdb", Parser::parse_sql(&DuckDbDialect {}, trimmed)),
        (
            "databricks",
            Parser::parse_sql(&DatabricksDialect {}, trimmed),
        ),
    ];
    let mut errors = Vec::new();
    for (dialect, result) in parse_attempts {
        match result {
            Ok(statements) if statements.len() == 1 => {
                return statements
                    .into_iter()
                    .next()
                    .ok_or_else(|| "SQL parser returned no statements".to_string());
            }
            Ok(statements) => errors.push(format!(
                "{dialect}: expected exactly one statement, parsed {}",
                statements.len()
            )),
            Err(error) => errors.push(format!("{dialect}: {error}")),
        }
    }
    Err(format!(
        "failed to parse SQL for query structure grading: {}",
        errors.join("; ")
    ))
}

fn signature_for_query(query: &Query) -> Result<SqlStructureSignature, String> {
    match query.body.as_ref() {
        SetExpr::Select(select) => signature_for_select(select),
        SetExpr::Query(query) => signature_for_query(query),
        _ => Err("query structure grading supports simple SELECT query bodies only".to_string()),
    }
}

fn signature_for_select(select: &Select) -> Result<SqlStructureSignature, String> {
    let aliases = qualifier_aliases(select);
    let mut tables = BTreeSet::new();
    let mut joins = BTreeSet::new();
    for table in &select.from {
        collect_table_with_joins(table, &aliases, &mut tables, &mut joins)?;
    }

    let select_items = select
        .projection
        .iter()
        .map(|item| normalize_select_item(item, &aliases))
        .collect::<BTreeSet<_>>();
    let mut filters = BTreeSet::new();
    if let Some(selection) = select.selection.as_ref() {
        let mut conjuncts = Vec::new();
        collect_and_conjuncts(selection, &mut conjuncts);
        for conjunct in conjuncts {
            filters.insert(normalize_expr(conjunct, &aliases));
        }
    }
    if let Some(prewhere) = select.prewhere.as_ref() {
        filters.insert(format!("prewhere {}", normalize_expr(prewhere, &aliases)));
    }
    if let Some(having) = select.having.as_ref() {
        filters.insert(format!("having {}", normalize_expr(having, &aliases)));
    }
    if let Some(qualify) = select.qualify.as_ref() {
        filters.insert(format!("qualify {}", normalize_expr(qualify, &aliases)));
    }

    Ok(SqlStructureSignature {
        tables: tables.into_iter().collect(),
        joins: joins.into_iter().collect(),
        select: select_items.into_iter().collect(),
        filters: filters.into_iter().collect(),
        group_by: group_by_signature(&select.group_by, &aliases),
    })
}

fn qualifier_aliases(select: &Select) -> BTreeMap<String, String> {
    let mut aliases = BTreeMap::new();
    for table in &select.from {
        register_table_factor_aliases(&table.relation, &mut aliases);
        for join in &table.joins {
            register_table_factor_aliases(&join.relation, &mut aliases);
        }
    }
    preserve_reused_table_aliases(&mut aliases);
    aliases
}

fn preserve_reused_table_aliases(aliases: &mut BTreeMap<String, String>) {
    let mut counts = BTreeMap::new();
    for canonical in aliases.values() {
        *counts.entry(canonical.clone()).or_insert(0usize) += 1;
    }
    for (alias, canonical) in aliases.iter_mut() {
        if counts.get(canonical).copied().unwrap_or_default() > 2 && alias != canonical {
            canonical.clone_from(alias);
        }
    }
}

fn register_table_factor_aliases(factor: &TableFactor, aliases: &mut BTreeMap<String, String>) {
    match factor {
        TableFactor::Table { name, alias, .. } => {
            let relation = normalize_relation_name(&name.to_string());
            let leaf = relation_leaf(&relation);
            aliases.insert(relation, leaf.clone());
            if let Some(alias) = alias {
                aliases.insert(normalize_identifier(&alias.name.to_string()), leaf);
            }
        }
        TableFactor::Derived { alias, .. }
        | TableFactor::TableFunction { alias, .. }
        | TableFactor::Function { alias, .. }
        | TableFactor::UNNEST { alias, .. } => {
            if let Some(alias) = alias {
                let alias = normalize_identifier(&alias.name.to_string());
                aliases.insert(alias.clone(), alias);
            }
        }
        TableFactor::NestedJoin {
            table_with_joins,
            alias,
        } => {
            register_table_factor_aliases(&table_with_joins.relation, aliases);
            for join in &table_with_joins.joins {
                register_table_factor_aliases(&join.relation, aliases);
            }
            if let Some(alias) = alias {
                let alias = normalize_identifier(&alias.name.to_string());
                aliases.insert(alias.clone(), alias);
            }
        }
        TableFactor::Pivot { table, alias, .. } | TableFactor::Unpivot { table, alias, .. } => {
            register_table_factor_aliases(table, aliases);
            if let Some(alias) = alias {
                let alias = normalize_identifier(&alias.name.to_string());
                aliases.insert(alias.clone(), alias);
            }
        }
        _ => {}
    }
}

fn collect_table_with_joins(
    table: &TableWithJoins,
    aliases: &BTreeMap<String, String>,
    tables: &mut BTreeSet<String>,
    joins: &mut BTreeSet<String>,
) -> Result<(), String> {
    collect_table_factor_refs(&table.relation, aliases, tables, joins)?;
    for join in &table.joins {
        collect_table_factor_refs(&join.relation, aliases, tables, joins)?;
        joins.insert(join_signature(join, aliases));
    }
    Ok(())
}

fn collect_table_factor_refs(
    factor: &TableFactor,
    aliases: &BTreeMap<String, String>,
    tables: &mut BTreeSet<String>,
    joins: &mut BTreeSet<String>,
) -> Result<(), String> {
    match factor {
        TableFactor::Table { name, .. } => {
            tables.insert(normalize_relation_name(&name.to_string()));
        }
        TableFactor::Derived { subquery, .. } => {
            let signature = signature_for_query(subquery)?;
            tables.extend(signature.tables);
            joins.extend(signature.joins);
        }
        TableFactor::NestedJoin {
            table_with_joins, ..
        } => collect_table_with_joins(table_with_joins, aliases, tables, joins)?,
        TableFactor::Pivot { table, .. } | TableFactor::Unpivot { table, .. } => {
            collect_table_factor_refs(table, aliases, tables, joins)?;
        }
        TableFactor::Function { name, .. } => {
            tables.insert(format!(
                "function:{}",
                normalize_relation_name(&name.to_string())
            ));
        }
        TableFactor::TableFunction { expr, .. } => {
            tables.insert(format!("table_function:{}", normalize_expr(expr, aliases)));
        }
        TableFactor::UNNEST { array_exprs, .. } => {
            let exprs = array_exprs
                .iter()
                .map(|expr| normalize_expr(expr, aliases))
                .collect::<Vec<_>>()
                .join(", ");
            tables.insert(format!("unnest:{exprs}"));
        }
        _ => {
            tables.insert(normalize_sql_fragment(&factor.to_string(), aliases));
        }
    }
    Ok(())
}

fn join_signature(join: &Join, aliases: &BTreeMap<String, String>) -> String {
    let relation = table_factor_label(&join.relation, aliases);
    if let JoinOperator::AsOf {
        match_condition,
        constraint,
    } = &join.join_operator
    {
        return join_constraint_signature(
            format!(
                "asof join {relation} match {}",
                normalize_expr(match_condition, aliases)
            ),
            Some(constraint),
            aliases,
        );
    }
    let (kind, constraint) = join_kind_and_constraint(&join.join_operator);
    join_constraint_signature(format!("{kind} {relation}"), constraint, aliases)
}

fn join_constraint_signature(
    prefix: String,
    constraint: Option<&JoinConstraint>,
    aliases: &BTreeMap<String, String>,
) -> String {
    match constraint {
        Some(JoinConstraint::On(expr)) => {
            let mut conjuncts = Vec::new();
            collect_and_conjuncts(expr, &mut conjuncts);
            let constraints = conjuncts
                .into_iter()
                .map(|expr| normalize_expr(expr, aliases))
                .collect::<Vec<_>>()
                .join(" and ");
            format!("{prefix} on {constraints}")
        }
        Some(JoinConstraint::Using(columns)) => {
            let columns = columns
                .iter()
                .map(|column| normalize_sql_fragment(&column.to_string(), aliases))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{prefix} using ({columns})")
        }
        Some(JoinConstraint::Natural) => format!("{prefix} natural"),
        Some(JoinConstraint::None) | None => prefix,
    }
}

fn join_kind_and_constraint(operator: &JoinOperator) -> (&'static str, Option<&JoinConstraint>) {
    match operator {
        JoinOperator::Join(constraint) => ("join", Some(constraint)),
        JoinOperator::Inner(constraint) => ("inner join", Some(constraint)),
        JoinOperator::Left(constraint) | JoinOperator::LeftOuter(constraint) => {
            ("left join", Some(constraint))
        }
        JoinOperator::Right(constraint) | JoinOperator::RightOuter(constraint) => {
            ("right join", Some(constraint))
        }
        JoinOperator::FullOuter(constraint) => ("full join", Some(constraint)),
        JoinOperator::CrossJoin(constraint) => ("cross join", Some(constraint)),
        JoinOperator::Semi(constraint) | JoinOperator::LeftSemi(constraint) => {
            ("semi join", Some(constraint))
        }
        JoinOperator::RightSemi(constraint) => ("right semi join", Some(constraint)),
        JoinOperator::Anti(constraint) | JoinOperator::LeftAnti(constraint) => {
            ("anti join", Some(constraint))
        }
        JoinOperator::RightAnti(constraint) => ("right anti join", Some(constraint)),
        JoinOperator::CrossApply => ("cross apply", None),
        JoinOperator::OuterApply => ("outer apply", None),
        JoinOperator::AsOf { constraint, .. } => ("asof join", Some(constraint)),
        JoinOperator::StraightJoin(constraint) => ("straight join", Some(constraint)),
    }
}

fn table_factor_label(factor: &TableFactor, aliases: &BTreeMap<String, String>) -> String {
    match factor {
        TableFactor::Table { name, .. } => normalize_relation_name(&name.to_string()),
        TableFactor::Derived { alias, .. } => alias.as_ref().map_or_else(
            || "subquery".to_string(),
            |alias| format!("subquery:{}", normalize_identifier(&alias.name.to_string())),
        ),
        TableFactor::Function { name, .. } => {
            format!("function:{}", normalize_relation_name(&name.to_string()))
        }
        TableFactor::TableFunction { expr, .. } => {
            format!("table_function:{}", normalize_expr(expr, aliases))
        }
        TableFactor::UNNEST { array_exprs, .. } => {
            let exprs = array_exprs
                .iter()
                .map(|expr| normalize_expr(expr, aliases))
                .collect::<Vec<_>>()
                .join(", ");
            format!("unnest:{exprs}")
        }
        TableFactor::NestedJoin { .. } => "nested_join".to_string(),
        _ => normalize_sql_fragment(&factor.to_string(), aliases),
    }
}

fn normalize_select_item(item: &SelectItem, aliases: &BTreeMap<String, String>) -> String {
    match item {
        SelectItem::UnnamedExpr(expr) | SelectItem::ExprWithAlias { expr, .. } => {
            normalize_expr(expr, aliases)
        }
        SelectItem::QualifiedWildcard(kind, _) => {
            normalize_sql_fragment(&kind.to_string(), aliases)
        }
        SelectItem::Wildcard(_) => "*".to_string(),
    }
}

fn group_by_signature(group_by: &GroupByExpr, aliases: &BTreeMap<String, String>) -> Vec<String> {
    match group_by {
        GroupByExpr::All(_) => vec!["all".to_string()],
        GroupByExpr::Expressions(exprs, _) => exprs
            .iter()
            .map(|expr| normalize_expr(expr, aliases))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
    }
}

fn collect_and_conjuncts<'a>(expr: &'a Expr, out: &mut Vec<&'a Expr>) {
    match expr {
        Expr::BinaryOp {
            left,
            op: BinaryOperator::And,
            right,
        } => {
            collect_and_conjuncts(left, out);
            collect_and_conjuncts(right, out);
        }
        Expr::Nested(expr) => collect_and_conjuncts(expr, out),
        _ => out.push(expr),
    }
}

fn normalize_expr(expr: &Expr, aliases: &BTreeMap<String, String>) -> String {
    normalize_sql_fragment(&expr.to_string(), aliases)
}

fn normalize_relation_name(value: &str) -> String {
    normalize_sql_fragment(value, &BTreeMap::new())
}

fn normalize_identifier(value: &str) -> String {
    normalize_sql_fragment(value, &BTreeMap::new())
}

fn normalize_sql_fragment(value: &str, aliases: &BTreeMap<String, String>) -> String {
    let masked = mask_sql_literals(value);
    let mut normalized = masked
        .replace(['"', '`', '[', ']'], "")
        .to_ascii_lowercase();
    normalized = collapse_whitespace(&normalized);
    normalized = normalized
        .replace(" . ", ".")
        .replace(" .", ".")
        .replace(". ", ".")
        .replace("( ", "(")
        .replace(" )", ")")
        .replace(" ,", ",");
    strip_single_table_qualifier(&apply_aliases(&normalized, aliases), aliases)
}

fn apply_aliases(value: &str, aliases: &BTreeMap<String, String>) -> String {
    let mut out = value.to_string();
    let mut entries = aliases.iter().collect::<Vec<_>>();
    entries.sort_by(|(left, _), (right, _)| right.len().cmp(&left.len()).then(left.cmp(right)));
    for (alias, canonical) in entries {
        if alias == canonical {
            continue;
        }
        out = replace_qualifier(&out, alias, canonical);
    }
    out
}

fn replace_qualifier(value: &str, alias: &str, canonical: &str) -> String {
    let needle = format!("{alias}.");
    let replacement = if canonical.is_empty() {
        String::new()
    } else {
        format!("{canonical}.")
    };
    let mut out = String::with_capacity(value.len());
    let mut cursor = 0usize;
    while let Some(relative) = value[cursor..].find(&needle) {
        let absolute = cursor + relative;
        if absolute == 0 || is_qualifier_boundary(value[..absolute].chars().next_back()) {
            out.push_str(&value[cursor..absolute]);
            out.push_str(&replacement);
            cursor = absolute + needle.len();
        } else {
            out.push_str(&value[cursor..absolute + needle.len()]);
            cursor = absolute + needle.len();
        }
    }
    out.push_str(&value[cursor..]);
    out
}

fn strip_single_table_qualifier(value: &str, aliases: &BTreeMap<String, String>) -> String {
    let canonical_tables = aliases
        .values()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if canonical_tables.len() != 1 || aliases.len() > 2 {
        return value.to_string();
    }
    replace_qualifier(value, canonical_tables[0], "")
}

fn is_qualifier_boundary(ch: Option<char>) -> bool {
    ch.is_none_or(|ch| !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | '$')))
}

fn mask_sql_literals(value: &str) -> String {
    let chars = value.chars().collect::<Vec<_>>();
    let mut out = String::with_capacity(value.len());
    let mut index = 0usize;
    while index < chars.len() {
        match chars[index] {
            '\'' => {
                out.push('?');
                index += 1;
                while index < chars.len() {
                    if chars[index] == '\'' {
                        if index + 1 < chars.len() && chars[index + 1] == '\'' {
                            index += 2;
                        } else {
                            index += 1;
                            break;
                        }
                    } else {
                        index += 1;
                    }
                }
            }
            '-' | '+' if index + 1 < chars.len() && chars[index + 1].is_ascii_digit() => {
                if let Some(end) = numeric_literal_end(&chars, index) {
                    out.push('?');
                    index = end;
                } else {
                    out.push(chars[index]);
                    index += 1;
                }
            }
            ch if ch.is_ascii_digit() => {
                if let Some(end) = numeric_literal_end(&chars, index) {
                    out.push('?');
                    index = end;
                } else {
                    out.push(ch);
                    index += 1;
                }
            }
            ch => {
                out.push(ch);
                index += 1;
            }
        }
    }
    out
}

fn numeric_literal_end(chars: &[char], start: usize) -> Option<usize> {
    if start > 0 && !is_numeric_boundary(chars[start - 1]) {
        return None;
    }
    let mut index = start;
    if matches!(chars[index], '-' | '+') {
        index += 1;
    }
    while index < chars.len() && chars[index].is_ascii_digit() {
        index += 1;
    }
    if index < chars.len()
        && chars[index] == '.'
        && index + 1 < chars.len()
        && chars[index + 1].is_ascii_digit()
    {
        index += 1;
        while index < chars.len() && chars[index].is_ascii_digit() {
            index += 1;
        }
    }
    if index < chars.len() && matches!(chars[index], 'e' | 'E') {
        let exponent_start = index;
        index += 1;
        if index < chars.len() && matches!(chars[index], '-' | '+') {
            index += 1;
        }
        let digits_start = index;
        while index < chars.len() && chars[index].is_ascii_digit() {
            index += 1;
        }
        if index == digits_start {
            index = exponent_start;
        }
    }
    if index == start || (matches!(chars[start], '-' | '+') && index == start + 1) {
        return None;
    }
    (index == chars.len() || is_numeric_boundary(chars[index])).then_some(index)
}

fn is_numeric_boundary(ch: char) -> bool {
    !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | '$'))
}

fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn relation_leaf(relation: &str) -> String {
    relation.rsplit('.').next().unwrap_or(relation).to_string()
}

fn missing_values(actual: &[String], expected: &[String]) -> Vec<String> {
    let actual = actual.iter().collect::<BTreeSet<_>>();
    expected
        .iter()
        .filter(|value| !actual.contains(value))
        .cloned()
        .collect()
}

fn unexpected_values(actual: &[String], expected: &[String]) -> Vec<String> {
    let expected = expected.iter().collect::<BTreeSet<_>>();
    actual
        .iter()
        .filter(|value| !expected.contains(value))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::compare_sql_structure;

    #[test]
    fn query_structure_masks_string_date_and_numeric_literals() {
        let actual = "
            select o.country, sum(o.amount) as revenue
            from analytics.orders o
            where o.order_date between '2026-03-01' and '2026-03-31'
              and o.country = 'US'
              and o.amount > 100
            group by o.country
        ";
        let expected = "
            select orders.country, sum(orders.amount) as revenue
            from analytics.orders
            where orders.order_date between '2024-01-01' and '2024-01-31'
              and orders.country = 'GB'
              and orders.amount > 200
            group by orders.country
        ";

        let comparison = compare_sql_structure(actual, expected).expect("comparison");

        assert!(comparison.matches, "{comparison:#?}");
    }

    #[test]
    fn query_structure_reports_missing_filter_clause() {
        let actual = "select country, sum(amount) from analytics.orders group by country";
        let expected = "
            select country, sum(amount)
            from analytics.orders
            where country = 'US'
            group by country
        ";

        let comparison = compare_sql_structure(actual, expected).expect("comparison");

        assert!(!comparison.matches);
        assert_eq!(comparison.diff.changed_clauses(), vec!["WHERE"]);
        assert_eq!(comparison.diff.missing_filters, vec!["country = ?"]);
    }

    #[test]
    fn query_structure_reports_wrong_table_clause() {
        let actual = "select country, sum(amount) from analytics.customers group by country";
        let expected = "select country, sum(amount) from analytics.orders group by country";

        let comparison = compare_sql_structure(actual, expected).expect("comparison");

        assert!(!comparison.matches);
        assert_eq!(comparison.diff.changed_clauses(), vec!["FROM"]);
        assert_eq!(comparison.diff.missing_tables, vec!["analytics.orders"]);
        assert_eq!(
            comparison.diff.unexpected_tables,
            vec!["analytics.customers"]
        );
    }

    #[test]
    fn query_structure_preserves_self_join_qualifiers() {
        let actual = "
            select child.id
            from analytics.accounts child
            join analytics.accounts parent on child.parent_id = parent.id
        ";
        let expected = "
            select child.id
            from analytics.accounts child
            join analytics.accounts parent on parent.parent_id = child.id
        ";

        let comparison = compare_sql_structure(actual, expected).expect("comparison");

        assert!(!comparison.matches);
        assert_eq!(comparison.diff.changed_clauses(), vec!["JOIN"]);
    }

    #[test]
    fn query_structure_compares_asof_match_conditions() {
        let actual = "
            select q.symbol
            from analytics.quotes q
            asof join analytics.trades t
              match_condition (q.observed_at >= t.executed_at)
              on q.symbol = t.symbol
        ";
        let expected = "
            select q.symbol
            from analytics.quotes q
            asof join analytics.trades t
              match_condition (q.observed_at <= t.executed_at)
              on q.symbol = t.symbol
        ";

        let comparison = compare_sql_structure(actual, expected).expect("comparison");

        assert!(!comparison.matches);
        assert_eq!(comparison.diff.changed_clauses(), vec!["JOIN"]);
    }
}
