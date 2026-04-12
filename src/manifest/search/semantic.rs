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

fn compare_preview_items(left: &SemanticPreviewItem, right: &SemanticPreviewItem) -> Ordering {
    right
        .match_type
        .rank()
        .cmp(&left.match_type.rank())
        .then_with(|| right.canonical.cmp(&left.canonical))
        .then_with(|| left.name.cmp(&right.name))
}

fn match_measure(
    measure: &ArchivedNovaMeasure,
    entity_canonical: bool,
    token_set: &HashSet<&str>,
    min_word_len: usize,
) -> Option<SemanticPreviewItem> {
    let match_type = best_match_type(
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
        match_type,
    })
}

fn match_metric(
    metric: &ArchivedNovaMetric,
    entity_canonical: bool,
    token_set: &HashSet<&str>,
    min_word_len: usize,
) -> Option<SemanticPreviewItem> {
    let match_type = best_match_type(
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
        match_type,
    })
}

fn best_match_type<'a>(
    token_set: &HashSet<&str>,
    min_word_len: usize,
    direct_values: &[(Option<&'a str>, SemanticMatchType)],
    synonyms: impl Iterator<Item = &'a str>,
) -> Option<SemanticMatchType> {
    if direct_values.iter().any(|(value, kind)| {
        *kind == SemanticMatchType::Name
            && value.is_some_and(|value| tokens_match(value, token_set, min_word_len))
    }) {
        return Some(SemanticMatchType::Name);
    }
    if synonyms
        .into_iter()
        .any(|value| tokens_match(value, token_set, min_word_len))
    {
        return Some(SemanticMatchType::Synonym);
    }
    direct_values
        .iter()
        .filter_map(|(value, kind)| value.map(|value| (value, *kind)))
        .filter(|(_, kind)| !matches!(kind, SemanticMatchType::Name | SemanticMatchType::Synonym))
        .filter(|(value, _)| tokens_match(value, token_set, min_word_len))
        .max_by_key(|(_, kind)| kind.rank())
        .map(|(_, kind)| kind)
}

fn tokens_match(value: &str, token_set: &HashSet<&str>, min_word_len: usize) -> bool {
    if value.is_empty() || token_set.is_empty() {
        return false;
    }
    tokenize_alnum_lowercase(value, min_word_len)
        .iter()
        .any(|token| token_set.contains(token.as_str()))
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
}
