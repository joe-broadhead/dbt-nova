use std::cmp::Ordering;
use std::collections::HashSet;

use crate::config::SearchConfig;
use crate::manifest::entity::{ArchivedNovaMeasure, ArchivedNovaMeta, ArchivedNovaMetric};
use crate::utils::tokenize_alnum_lowercase;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum SemanticMatchType {
    Name,
    Synonym,
    Field,
    Description,
    Expression,
}

impl SemanticMatchType {
    #[must_use]
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Synonym => "synonym",
            Self::Field => "field",
            Self::Description => "description",
            Self::Expression => "expression",
        }
    }

    #[must_use]
    pub(crate) fn multiplier(self, config: &SearchConfig) -> f32 {
        match self {
            Self::Name => config.nova_semantic_name_match_multiplier,
            Self::Synonym => config.nova_semantic_synonym_match_multiplier,
            Self::Field | Self::Description | Self::Expression => {
                config.nova_semantic_definition_match_multiplier
            }
        }
    }

    fn rank(self) -> u8 {
        match self {
            Self::Name => 5,
            Self::Synonym => 4,
            Self::Field => 3,
            Self::Description => 2,
            Self::Expression => 1,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SemanticPreviewItem {
    pub name: String,
    pub description: Option<String>,
    pub expression: Option<String>,
    pub field: Option<String>,
    pub canonical: bool,
    pub match_type: SemanticMatchType,
    pub matched_token_count: usize,
    pub query_token_count: usize,
}

impl SemanticPreviewItem {
    #[must_use]
    pub(crate) fn query_coverage(&self) -> f32 {
        if self.query_token_count == 0 {
            return 0.0;
        }
        count_to_f32(self.matched_token_count) / count_to_f32(self.query_token_count)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SemanticMatchInfo {
    match_type: SemanticMatchType,
    matched_token_count: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct NovaSemanticMatches {
    pub measures: Vec<SemanticPreviewItem>,
    pub metrics: Vec<SemanticPreviewItem>,
}

impl NovaSemanticMatches {
    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.measures.is_empty() && self.metrics.is_empty()
    }

    #[must_use]
    pub(crate) fn has_measure_match(&self) -> bool {
        !self.measures.is_empty()
    }

    #[must_use]
    pub(crate) fn has_metric_match(&self) -> bool {
        !self.metrics.is_empty()
    }

    #[must_use]
    pub(crate) fn has_canonical_match(&self) -> bool {
        self.measures.iter().any(|item| item.canonical)
            || self.metrics.iter().any(|item| item.canonical)
    }

    #[must_use]
    pub(crate) fn strongest_match_type(&self) -> Option<SemanticMatchType> {
        self.measures
            .iter()
            .chain(self.metrics.iter())
            .map(|item| item.match_type)
            .max_by_key(|match_type| match_type.rank())
    }

    #[must_use]
    pub(crate) fn best_query_coverage(&self) -> f32 {
        self.measures
            .iter()
            .chain(self.metrics.iter())
            .map(SemanticPreviewItem::query_coverage)
            .fold(0.0f32, f32::max)
    }
}

#[must_use]
pub(crate) fn match_nova_semantics(
    nova: &ArchivedNovaMeta,
    token_set: &HashSet<&str>,
    min_word_len: usize,
) -> NovaSemanticMatches {
    if token_set.is_empty() {
        return NovaSemanticMatches::default();
    }

    let measures = nova
        .measures
        .iter()
        .filter_map(|measure| match_measure(measure, nova.canonical, token_set, min_word_len))
        .collect();
    let mut metrics = Vec::new();

    if let Some(metric) = nova.metric.as_ref()
        && let Some(item) = match_metric(metric, nova.canonical, token_set, min_word_len)
    {
        metrics.push(item);
    }
    metrics.extend(
        nova.metrics
            .iter()
            .filter_map(|metric| match_metric(metric, nova.canonical, token_set, min_word_len)),
    );

    let mut matches = NovaSemanticMatches { measures, metrics };
    matches.measures.sort_by(compare_preview_items);
    matches.metrics.sort_by(compare_preview_items);
    matches
}

fn count_to_f32(count: usize) -> f32 {
    f32::from(u16::try_from(count).unwrap_or(u16::MAX))
}

fn compare_preview_items(left: &SemanticPreviewItem, right: &SemanticPreviewItem) -> Ordering {
    right
        .match_type
        .rank()
        .cmp(&left.match_type.rank())
        .then_with(|| {
            right
                .matched_token_count
                .cmp(&left.matched_token_count)
                .then_with(|| right.query_token_count.cmp(&left.query_token_count))
        })
        .then_with(|| right.canonical.cmp(&left.canonical))
        .then_with(|| left.name.cmp(&right.name))
}

fn match_measure(
    measure: &ArchivedNovaMeasure,
    entity_canonical: bool,
    token_set: &HashSet<&str>,
    min_word_len: usize,
) -> Option<SemanticPreviewItem> {
    let match_info = best_match_info(
        token_set,
        min_word_len,
        &[
            (Some(measure.name.as_str()), SemanticMatchType::Name),
            (
                measure
                    .field
                    .as_ref()
                    .map(rkyv::string::ArchivedString::as_str),
                SemanticMatchType::Field,
            ),
            (
                measure
                    .description
                    .as_ref()
                    .map(rkyv::string::ArchivedString::as_str),
                SemanticMatchType::Description,
            ),
            (
                measure
                    .expression
                    .as_ref()
                    .map(rkyv::string::ArchivedString::as_str),
                SemanticMatchType::Expression,
            ),
        ],
        measure
            .synonyms
            .iter()
            .map(rkyv::string::ArchivedString::as_str),
    )?;

    Some(SemanticPreviewItem {
        name: measure.name.as_str().to_string(),
        description: measure
            .description
            .as_ref()
            .map(|value| value.as_str().to_string()),
        expression: measure
            .expression
            .as_ref()
            .map(|value| value.as_str().to_string()),
        field: measure
            .field
            .as_ref()
            .map(|value| value.as_str().to_string()),
        canonical: entity_canonical || measure.canonical,
        match_type: match_info.match_type,
        matched_token_count: match_info.matched_token_count,
        query_token_count: token_set.len(),
    })
}

fn match_metric(
    metric: &ArchivedNovaMetric,
    entity_canonical: bool,
    token_set: &HashSet<&str>,
    min_word_len: usize,
) -> Option<SemanticPreviewItem> {
    let match_info = best_match_info(
        token_set,
        min_word_len,
        &[
            (Some(metric.name.as_str()), SemanticMatchType::Name),
            (
                metric
                    .description
                    .as_ref()
                    .map(rkyv::string::ArchivedString::as_str),
                SemanticMatchType::Description,
            ),
            (
                metric
                    .expression
                    .as_ref()
                    .map(rkyv::string::ArchivedString::as_str),
                SemanticMatchType::Expression,
            ),
        ],
        metric
            .synonyms
            .iter()
            .map(rkyv::string::ArchivedString::as_str),
    )?;

    Some(SemanticPreviewItem {
        name: metric.name.as_str().to_string(),
        description: metric
            .description
            .as_ref()
            .map(|value| value.as_str().to_string()),
        expression: metric
            .expression
            .as_ref()
            .map(|value| value.as_str().to_string()),
        field: None,
        canonical: entity_canonical || metric.canonical,
        match_type: match_info.match_type,
        matched_token_count: match_info.matched_token_count,
        query_token_count: token_set.len(),
    })
}

fn best_match_info<'a>(
    token_set: &HashSet<&str>,
    min_word_len: usize,
    direct_values: &[(Option<&'a str>, SemanticMatchType)],
    synonyms: impl Iterator<Item = &'a str>,
) -> Option<SemanticMatchInfo> {
    let name_match = direct_values
        .iter()
        .filter(|(_, kind)| *kind == SemanticMatchType::Name)
        .filter_map(|(value, kind)| {
            value.and_then(|value| {
                let matched_token_count = token_match_count(value, token_set, min_word_len);
                (matched_token_count > 0).then_some(SemanticMatchInfo {
                    match_type: *kind,
                    matched_token_count,
                })
            })
        })
        .max_by_key(|info| info.matched_token_count);
    if name_match.is_some() {
        return name_match;
    }

    let synonym_match = synonyms
        .into_iter()
        .filter_map(|value| {
            let matched_token_count = token_match_count(value, token_set, min_word_len);
            (matched_token_count > 0).then_some(SemanticMatchInfo {
                match_type: SemanticMatchType::Synonym,
                matched_token_count,
            })
        })
        .max_by_key(|info| info.matched_token_count);
    if synonym_match.is_some() {
        return synonym_match;
    }

    direct_values
        .iter()
        .filter_map(|(value, kind)| value.map(|value| (value, *kind)))
        .filter(|(_, kind)| !matches!(kind, SemanticMatchType::Name | SemanticMatchType::Synonym))
        .filter_map(|(value, kind)| {
            let matched_token_count = token_match_count(value, token_set, min_word_len);
            (matched_token_count > 0).then_some(SemanticMatchInfo {
                match_type: kind,
                matched_token_count,
            })
        })
        .max_by(|left, right| {
            left.match_type
                .rank()
                .cmp(&right.match_type.rank())
                .then_with(|| left.matched_token_count.cmp(&right.matched_token_count))
        })
}

fn token_match_count(value: &str, token_set: &HashSet<&str>, min_word_len: usize) -> usize {
    if value.is_empty() || token_set.is_empty() {
        return 0;
    }
    let value_tokens: HashSet<String> = tokenize_alnum_lowercase(value, min_word_len)
        .iter()
        .cloned()
        .collect();
    token_set
        .iter()
        .filter(|token| value_tokens.contains(**token))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::entity::{NovaMeasure, NovaMeta, NovaMetric};

    #[test]
    fn match_nova_semantics_prefers_name_matches_over_description_matches() {
        let nova = rkyv::to_bytes::<rkyv::rancor::Error>(&NovaMeta {
            role: None,
            semantic_type: None,
            synonyms: Vec::new(),
            domains: Vec::new(),
            use_cases: Vec::new(),
            example_values: Vec::new(),
            canonical: false,
            tier: None,
            grain: None,
            measures: vec![
                NovaMeasure {
                    name: "gmv".to_string(),
                    measure_type: None,
                    expression: Some("sum(gmv_amount)".to_string()),
                    description: Some("Gross merchandise value".to_string()),
                    field: Some("gmv_amount".to_string()),
                    synonyms: vec!["gross merchandise value".to_string()],
                    canonical: false,
                },
                NovaMeasure {
                    name: "margin".to_string(),
                    measure_type: None,
                    expression: Some("sum(gross_margin)".to_string()),
                    description: Some("GMV comparison helper".to_string()),
                    field: Some("gross_margin".to_string()),
                    synonyms: Vec::new(),
                    canonical: false,
                },
            ],
            metric: None,
            metrics: Vec::new(),
            governance: None,
            search: None,
        })
        .expect("serialize nova meta");
        let archived =
            rkyv::access::<crate::manifest::entity::ArchivedNovaMeta, rkyv::rancor::Error>(&nova)
                .expect("access nova meta");
        let tokens: HashSet<&str> = ["gmv"].into_iter().collect();

        let matches = match_nova_semantics(archived, &tokens, 2);

        assert_eq!(matches.measures.len(), 2);
        assert_eq!(matches.measures[0].name, "gmv");
        assert_eq!(matches.measures[0].match_type, SemanticMatchType::Name);
        assert_eq!(
            matches.measures[1].match_type,
            SemanticMatchType::Description
        );
    }

    #[test]
    fn match_nova_semantics_uses_measure_level_canonical_flags() {
        let nova = rkyv::to_bytes::<rkyv::rancor::Error>(&NovaMeta {
            role: None,
            semantic_type: None,
            synonyms: Vec::new(),
            domains: Vec::new(),
            use_cases: Vec::new(),
            example_values: Vec::new(),
            canonical: false,
            tier: None,
            grain: None,
            measures: Vec::new(),
            metric: Some(NovaMetric {
                name: "aov".to_string(),
                description: Some("Average order value".to_string()),
                expression: Some("sum(gmv) / nullif(count(order_id), 0)".to_string()),
                synonyms: vec!["average order value".to_string()],
                template: false,
                grain: None,
                recommended_filters: Vec::new(),
                canonical: true,
            }),
            metrics: Vec::new(),
            governance: None,
            search: None,
        })
        .expect("serialize nova meta");
        let archived =
            rkyv::access::<crate::manifest::entity::ArchivedNovaMeta, rkyv::rancor::Error>(&nova)
                .expect("access nova meta");
        let tokens: HashSet<&str> = ["aov"].into_iter().collect();

        let matches = match_nova_semantics(archived, &tokens, 2);

        assert_eq!(matches.metrics.len(), 1);
        assert!(matches.metrics[0].canonical);
        assert!(matches.has_canonical_match());
    }

    #[test]
    fn match_nova_semantics_prefers_higher_query_coverage_with_same_match_type() {
        let nova = rkyv::to_bytes::<rkyv::rancor::Error>(&NovaMeta {
            role: None,
            semantic_type: None,
            synonyms: Vec::new(),
            domains: Vec::new(),
            use_cases: Vec::new(),
            example_values: Vec::new(),
            canonical: false,
            tier: None,
            grain: None,
            measures: Vec::new(),
            metric: None,
            metrics: vec![
                NovaMetric {
                    name: "checkout_completion_rate".to_string(),
                    description: Some(
                        "Share of checkout-start sessions that reach order completion.".to_string(),
                    ),
                    expression: Some(
                        "sum(order_completed) / nullif(sum(checkout_process_commenced), 0)"
                            .to_string(),
                    ),
                    template: true,
                    grain: None,
                    recommended_filters: Vec::new(),
                    synonyms: vec!["checkout success rate".to_string()],
                    canonical: true,
                },
                NovaMetric {
                    name: "add_to_cart_to_order_completion_rate".to_string(),
                    description: Some(
                        "Share of add-to-cart sessions that reach order completion.".to_string(),
                    ),
                    expression: Some(
                        "sum(order_completed) / nullif(sum(product_added_to_cart), 0)".to_string(),
                    ),
                    template: true,
                    grain: None,
                    recommended_filters: Vec::new(),
                    synonyms: vec!["cart conversion rate".to_string()],
                    canonical: true,
                },
            ],
            governance: None,
            search: None,
        })
        .expect("serialize nova meta");
        let archived =
            rkyv::access::<crate::manifest::entity::ArchivedNovaMeta, rkyv::rancor::Error>(&nova)
                .expect("access nova meta");
        let tokens: HashSet<&str> = ["checkout", "completion", "rate"].into_iter().collect();

        let matches = match_nova_semantics(archived, &tokens, 2);

        assert_eq!(matches.metrics.len(), 2);
        assert_eq!(matches.metrics[0].name, "checkout_completion_rate");
        assert_eq!(matches.metrics[0].matched_token_count, 3);
        assert_eq!(
            matches.metrics[1].name,
            "add_to_cart_to_order_completion_rate"
        );
        assert_eq!(matches.metrics[1].matched_token_count, 2);
    }
}
