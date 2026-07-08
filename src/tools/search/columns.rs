use super::{
    Arc, ArchivedColumnMetaSummary, ArchivedEntity, ArchivedNovaMeta, ArchivedString,
    ColumnInventoryParams, ColumnInventoryRow, ColumnSearchMatch, ColumnSearchRow, DbtNovaError,
    EntityStore, HashMap, HashSet, JsonValue, ManifestSearch, MetadataSupportSignals, Result,
    SearchColumnsParams, SearchDeadline, SuccessResponse, check_scan_deadline,
    compare_column_inventory_rows, compare_column_search_rows, fully_covered_token_count,
    token_overlap_count, tokenize_alnum_lowercase, usize_to_f32,
};

impl ManifestSearch {
    /// Search columns across entities using names, semantic hints, and example values.
    ///
    /// # Errors
    /// Returns an error if the query is invalid, filters are invalid, or
    /// pagination exceeds configured limits.
    #[tracing::instrument(skip(self, params), fields(tool = "search_columns", query_len = params.query.len(), limit = ?params.pagination.limit, offset = params.pagination.offset))]
    pub async fn search_columns(&self, params: &SearchColumnsParams) -> Result<JsonValue> {
        if params.query.chars().count() > self.config.search.max_query_length {
            return Err(DbtNovaError::InvalidParams(format!(
                "Query exceeds maximum length of {} characters",
                self.config.search.max_query_length
            )));
        }
        if params.pagination.offset > self.config.search.max_offset {
            return Err(DbtNovaError::InvalidParams(format!(
                "Offset exceeds maximum of {}",
                self.config.search.max_offset
            )));
        }

        let min_word_len = self.config.search.min_word_length.max(1);
        let tokens = tokenize_alnum_lowercase(&params.query, min_word_len);
        if tokens.is_empty() {
            return Err(DbtNovaError::InvalidParams(
                "Query too short or invalid".to_string(),
            ));
        }

        let deadline = SearchDeadline::from_config(&self.config.search);
        let resource_filter = normalized_resource_type_filter(&params.resource_types);
        let role_filter = normalized_value_filter(&params.roles);
        let semantic_type_filter = normalized_value_filter(&params.semantic_types);
        let mut rows = self
            .search_column_rows_blocking(
                tokens,
                min_word_len,
                resource_filter.clone(),
                role_filter.clone(),
                semantic_type_filter.clone(),
                deadline,
            )
            .await?;
        if let Some(min_score) = params.min_score {
            rows.retain(|row| row.score >= min_score);
        }

        let total = rows.len();
        let limit = self.page_limit(params.pagination.limit);
        let start = params.pagination.offset.min(total);
        let end = (start + limit).min(total);
        let results: Vec<JsonValue> = rows
            .into_iter()
            .skip(start)
            .take(end.saturating_sub(start))
            .map(serde_json::to_value)
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|error| DbtNovaError::ServerError(error.to_string()))?;
        let count = results.len();
        let mut response = SuccessResponse::new(results, count).with_total(total);
        if total > end {
            response = response.with_truncated(true);
        }
        Ok(serde_json::to_value(response)?)
    }

    /// List columns deterministically across entities.
    ///
    /// # Errors
    /// Returns an error if filters are invalid or pagination exceeds configured limits.
    #[tracing::instrument(skip(self, params), fields(tool = "column_inventory", limit = ?params.pagination.limit, offset = params.pagination.offset))]
    pub async fn column_inventory(&self, params: &ColumnInventoryParams) -> Result<JsonValue> {
        if params.pagination.offset > self.config.search.max_offset {
            return Err(DbtNovaError::InvalidParams(format!(
                "Offset exceeds maximum of {}",
                self.config.search.max_offset
            )));
        }

        let deadline = SearchDeadline::from_config(&self.config.search);
        let resource_filter = normalized_resource_type_filter(&params.resource_types);
        let role_filter = normalized_value_filter(&params.roles);
        let semantic_type_filter = normalized_value_filter(&params.semantic_types);
        let rows = self
            .column_inventory_rows_blocking(
                resource_filter.clone(),
                role_filter.clone(),
                semantic_type_filter.clone(),
                params.annotated_only,
                deadline,
            )
            .await?;
        let total = rows.len();
        let limit = self.page_limit(params.pagination.limit);
        let start = params.pagination.offset.min(total);
        let end = (start + limit).min(total);
        let results: Vec<JsonValue> = rows
            .into_iter()
            .skip(start)
            .take(end.saturating_sub(start))
            .map(serde_json::to_value)
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|error| DbtNovaError::ServerError(error.to_string()))?;
        let count = results.len();
        let mut response = SuccessResponse::new(results, count).with_total(total);
        if total > end {
            response = response.with_truncated(true);
        }
        Ok(serde_json::to_value(response)?)
    }

    pub(super) async fn search_column_rows_blocking(
        &self,
        tokens: Vec<String>,
        min_word_len: usize,
        resource_filter: Option<HashSet<String>>,
        role_filter: Option<HashSet<String>>,
        semantic_type_filter: Option<HashSet<String>>,
        deadline: SearchDeadline,
    ) -> Result<Vec<ColumnSearchRow>> {
        let entities = Arc::clone(&self.entities);
        tokio::task::spawn_blocking(move || {
            let token_set: HashSet<&str> = tokens.iter().map(String::as_str).collect();
            Self::search_column_rows_from_store(
                &entities,
                &token_set,
                min_word_len,
                resource_filter.as_ref(),
                role_filter.as_ref(),
                semantic_type_filter.as_ref(),
                deadline,
            )
        })
        .await
        .map_err(|err| {
            DbtNovaError::ServerError(format!("join failure while scanning columns: {err}"))
        })?
    }

    pub(super) fn search_column_rows_from_store(
        entities: &EntityStore,
        token_set: &HashSet<&str>,
        min_word_len: usize,
        resource_filter: Option<&HashSet<String>>,
        role_filter: Option<&HashSet<String>>,
        semantic_type_filter: Option<&HashSet<String>>,
        deadline: SearchDeadline,
    ) -> Result<Vec<ColumnSearchRow>> {
        let mut rows = Vec::new();
        let mut scanned = 0usize;

        for unique_id in entities.ids() {
            check_scan_deadline(deadline, &mut scanned)?;
            let Some(entity) = entities.get_archived(unique_id)? else {
                continue;
            };
            if !resource_type_allowed_for_search(entity.resource_type_str(), resource_filter) {
                continue;
            }
            let nova = entity.nova_meta();
            let column_meta = entity_column_meta_lookup(entity);

            for column_name in entity.column_names_iter() {
                check_scan_deadline(deadline, &mut scanned)?;
                let summary = column_meta.get(column_name).copied();
                if !column_matches_filters(summary, role_filter, semantic_type_filter, false) {
                    continue;
                }
                let Some(search_match) =
                    best_column_search_match(column_name, summary, token_set, min_word_len)
                else {
                    continue;
                };
                rows.push(build_column_search_row(
                    unique_id,
                    entity,
                    nova,
                    column_name,
                    summary,
                    search_match,
                    token_set,
                    min_word_len,
                ));
            }
        }

        rows.sort_by(compare_column_search_rows);
        Ok(rows)
    }

    pub(super) async fn column_inventory_rows_blocking(
        &self,
        resource_filter: Option<HashSet<String>>,
        role_filter: Option<HashSet<String>>,
        semantic_type_filter: Option<HashSet<String>>,
        annotated_only: bool,
        deadline: SearchDeadline,
    ) -> Result<Vec<ColumnInventoryRow>> {
        let entities = Arc::clone(&self.entities);
        tokio::task::spawn_blocking(move || {
            Self::column_inventory_rows_from_store(
                &entities,
                resource_filter.as_ref(),
                role_filter.as_ref(),
                semantic_type_filter.as_ref(),
                annotated_only,
                deadline,
            )
        })
        .await
        .map_err(|err| {
            DbtNovaError::ServerError(format!(
                "join failure while scanning column inventory: {err}"
            ))
        })?
    }

    pub(super) fn column_inventory_rows_from_store(
        entities: &EntityStore,
        resource_filter: Option<&HashSet<String>>,
        role_filter: Option<&HashSet<String>>,
        semantic_type_filter: Option<&HashSet<String>>,
        annotated_only: bool,
        deadline: SearchDeadline,
    ) -> Result<Vec<ColumnInventoryRow>> {
        let mut rows = Vec::new();
        let mut scanned = 0usize;

        for unique_id in entities.ids() {
            check_scan_deadline(deadline, &mut scanned)?;
            let Some(entity) = entities.get_archived(unique_id)? else {
                continue;
            };
            if !resource_type_allowed_for_search(entity.resource_type_str(), resource_filter) {
                continue;
            }
            let nova = entity.nova_meta();
            let column_meta = entity_column_meta_lookup(entity);

            for column_name in entity.column_names_iter() {
                check_scan_deadline(deadline, &mut scanned)?;
                let summary = column_meta.get(column_name).copied();
                if !column_matches_filters(
                    summary,
                    role_filter,
                    semantic_type_filter,
                    annotated_only,
                ) {
                    continue;
                }
                rows.push(build_column_inventory_row(
                    unique_id,
                    entity,
                    nova,
                    column_name,
                    summary,
                ));
            }
        }

        rows.sort_by(compare_column_inventory_rows);
        Ok(rows)
    }
}
pub(super) fn build_column_inventory_row(
    unique_id: &str,
    entity: &ArchivedEntity,
    nova: Option<&ArchivedNovaMeta>,
    column_name: &str,
    summary: Option<&ArchivedColumnMetaSummary>,
) -> ColumnInventoryRow {
    ColumnInventoryRow {
        column_name: column_name.to_string(),
        annotated: column_is_annotated(summary),
        description: summary
            .and_then(|summary| summary.description.as_ref().map(ArchivedString::as_str))
            .map(str::to_string),
        role: summary
            .and_then(|summary| summary.role.as_ref().map(ArchivedString::as_str))
            .map(str::to_string),
        semantic_type: summary
            .and_then(|summary| summary.semantic_type.as_ref().map(ArchivedString::as_str))
            .map(str::to_string),
        synonyms: summary.map_or_else(Vec::new, |summary| {
            summary
                .synonyms
                .iter()
                .map(ArchivedString::as_str)
                .map(str::to_string)
                .collect()
        }),
        example_values: summary.map_or_else(Vec::new, |summary| {
            summary
                .example_values
                .iter()
                .map(ArchivedString::as_str)
                .map(str::to_string)
                .collect()
        }),
        primary_key: summary.is_some_and(|summary| summary.primary_key),
        parent_unique_id: unique_id.to_string(),
        parent_name: entity.name_str().unwrap_or(unique_id).to_string(),
        parent_resource_type: entity.resource_type_str().unwrap_or("unknown").to_string(),
        relation_name: entity.relation_name_str().map(str::to_string),
        domains: nova.map_or_else(Vec::new, |nova| {
            nova.domains
                .iter()
                .map(ArchivedString::as_str)
                .map(str::to_string)
                .collect()
        }),
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_column_search_row(
    unique_id: &str,
    entity: &ArchivedEntity,
    nova: Option<&ArchivedNovaMeta>,
    column_name: &str,
    summary: Option<&ArchivedColumnMetaSummary>,
    search_match: ColumnSearchMatch<'_>,
    token_set: &HashSet<&str>,
    min_word_len: usize,
) -> ColumnSearchRow {
    let mut row = ColumnSearchRow {
        column_name: column_name.to_string(),
        match_type: search_match.match_type.to_string(),
        score: search_match.score
            + column_parent_context_bonus(entity, nova, token_set, min_word_len),
        matched_value: search_match.matched_value.map(str::to_string),
        annotated: column_is_annotated(summary),
        description: summary
            .and_then(|summary| summary.description.as_ref().map(ArchivedString::as_str))
            .map(str::to_string),
        role: summary
            .and_then(|summary| summary.role.as_ref().map(ArchivedString::as_str))
            .map(str::to_string),
        semantic_type: summary
            .and_then(|summary| summary.semantic_type.as_ref().map(ArchivedString::as_str))
            .map(str::to_string),
        synonyms: summary.map_or_else(Vec::new, |summary| {
            summary
                .synonyms
                .iter()
                .map(ArchivedString::as_str)
                .map(str::to_string)
                .collect()
        }),
        example_values: summary.map_or_else(Vec::new, |summary| {
            summary
                .example_values
                .iter()
                .map(ArchivedString::as_str)
                .map(str::to_string)
                .collect()
        }),
        primary_key: summary.is_some_and(|summary| summary.primary_key),
        parent_unique_id: unique_id.to_string(),
        parent_name: entity.name_str().unwrap_or(unique_id).to_string(),
        parent_resource_type: entity.resource_type_str().unwrap_or("unknown").to_string(),
        relation_name: entity.relation_name_str().map(str::to_string),
        domains: nova.map_or_else(Vec::new, |nova| {
            nova.domains
                .iter()
                .map(ArchivedString::as_str)
                .map(str::to_string)
                .collect()
        }),
    };
    row.score = row.score.max(0.0);
    row
}

pub(super) fn collect_column_metadata_support_signals(
    column: &crate::manifest::entity::ArchivedColumnMetaSummary,
    query_token_set: &HashSet<&str>,
    min_word_len: usize,
    max_values_per_field: usize,
    signals: &mut MetadataSupportSignals,
) {
    if fully_covered_token_count(column.name.as_str(), query_token_set, min_word_len).is_some() {
        push_unique_string(
            &mut signals.column_names,
            column.name.as_str().to_string(),
            max_values_per_field,
        );
    }
    if let Some(role) = column.role.as_ref().map(ArchivedString::as_str)
        && token_overlap_count(role, query_token_set, min_word_len).is_some()
    {
        push_unique_string(
            &mut signals.column_roles,
            role.to_string(),
            max_values_per_field,
        );
    }
    if let Some(semantic_type) = column.semantic_type.as_ref().map(ArchivedString::as_str)
        && token_overlap_count(semantic_type, query_token_set, min_word_len).is_some()
    {
        push_unique_string(
            &mut signals.column_semantic_types,
            semantic_type.to_string(),
            max_values_per_field,
        );
    }
    for value in column.example_values.iter().map(ArchivedString::as_str) {
        if token_overlap_count(value, query_token_set, min_word_len).is_some() {
            push_unique_string(
                &mut signals.example_values,
                value.to_string(),
                max_values_per_field,
            );
        }
    }
}

pub(super) fn push_unique_string(
    values: &mut Vec<String>,
    value: String,
    max_values_per_field: usize,
) {
    if values.len() < max_values_per_field && !values.iter().any(|existing| existing == &value) {
        values.push(value);
    }
}

pub(super) fn merge_signal_values(
    target: &mut Vec<String>,
    source: &[String],
    max_values_per_field: usize,
) {
    for value in source {
        push_unique_string(target, value.clone(), max_values_per_field);
    }
}

pub(super) fn entity_column_meta_lookup(
    entity: &ArchivedEntity,
) -> HashMap<&str, &ArchivedColumnMetaSummary> {
    entity
        .column_meta()
        .iter()
        .map(|summary| (summary.name.as_str(), summary))
        .collect()
}

pub(super) fn column_is_annotated(summary: Option<&ArchivedColumnMetaSummary>) -> bool {
    summary.is_some_and(|summary| {
        summary.role.is_some()
            || summary.semantic_type.is_some()
            || !summary.synonyms.is_empty()
            || !summary.example_values.is_empty()
    })
}

pub(super) fn column_matches_filters(
    summary: Option<&ArchivedColumnMetaSummary>,
    role_filter: Option<&HashSet<String>>,
    semantic_type_filter: Option<&HashSet<String>>,
    annotated_only: bool,
) -> bool {
    if annotated_only && !column_is_annotated(summary) {
        return false;
    }
    if let Some(filter) = role_filter {
        let Some(role) = summary
            .and_then(|summary| summary.role.as_ref().map(ArchivedString::as_str))
            .map(str::trim)
            .map(str::to_lowercase)
        else {
            return false;
        };
        if !filter.contains(&role) {
            return false;
        }
    }
    if let Some(filter) = semantic_type_filter {
        let Some(semantic_type) = summary
            .and_then(|summary| summary.semantic_type.as_ref().map(ArchivedString::as_str))
            .map(str::trim)
            .map(str::to_lowercase)
        else {
            return false;
        };
        if !filter.contains(&semantic_type) {
            return false;
        }
    }
    true
}

pub(super) fn best_column_search_match<'a>(
    column_name: &'a str,
    summary: Option<&'a ArchivedColumnMetaSummary>,
    token_set: &HashSet<&str>,
    min_word_len: usize,
) -> Option<ColumnSearchMatch<'a>> {
    let mut best: Option<ColumnSearchMatch<'a>> = None;

    let mut consider = |candidate: ColumnSearchMatch<'a>| {
        if let Some(current) = best
            && candidate.score <= current.score
        {
            return;
        }
        best = Some(candidate);
    };

    if let Some(overlap) = token_overlap_count(column_name, token_set, min_word_len) {
        consider(ColumnSearchMatch {
            match_type: "name",
            matched_value: Some(column_name),
            score: 6.0 + usize_to_f32(overlap),
        });
    }

    if let Some(summary) = summary {
        for synonym in summary.synonyms.iter().map(ArchivedString::as_str) {
            if let Some(overlap) = token_overlap_count(synonym, token_set, min_word_len) {
                consider(ColumnSearchMatch {
                    match_type: "synonym",
                    matched_value: Some(synonym),
                    score: 5.5 + usize_to_f32(overlap),
                });
            }
        }
        if let Some(description) = summary.description.as_ref().map(ArchivedString::as_str)
            && let Some(overlap) = token_overlap_count(description, token_set, min_word_len)
        {
            consider(ColumnSearchMatch {
                match_type: "description",
                matched_value: Some(description),
                score: 2.5 + usize_to_f32(overlap),
            });
        }
        if let Some(role) = summary.role.as_ref().map(ArchivedString::as_str)
            && let Some(overlap) = token_overlap_count(role, token_set, min_word_len)
        {
            consider(ColumnSearchMatch {
                match_type: "role",
                matched_value: Some(role),
                score: 4.0 + usize_to_f32(overlap),
            });
        }
        if let Some(semantic_type) = summary.semantic_type.as_ref().map(ArchivedString::as_str)
            && let Some(overlap) = token_overlap_count(semantic_type, token_set, min_word_len)
        {
            consider(ColumnSearchMatch {
                match_type: "semantic_type",
                matched_value: Some(semantic_type),
                score: 4.5 + usize_to_f32(overlap),
            });
        }
        for example_value in summary.example_values.iter().map(ArchivedString::as_str) {
            if let Some(overlap) = token_overlap_count(example_value, token_set, min_word_len) {
                consider(ColumnSearchMatch {
                    match_type: "example_value",
                    matched_value: Some(example_value),
                    score: 4.25 + usize_to_f32(overlap),
                });
            }
        }
    }

    best
}

pub(super) fn column_parent_context_bonus(
    entity: &ArchivedEntity,
    nova: Option<&ArchivedNovaMeta>,
    token_set: &HashSet<&str>,
    min_word_len: usize,
) -> f32 {
    let name_bonus = entity
        .name_str()
        .and_then(|name| token_overlap_count(name, token_set, min_word_len))
        .map_or(0.0, |count| usize_to_f32(count) * 0.2);
    let relation_bonus = entity
        .relation_name_str()
        .and_then(|name| token_overlap_count(name, token_set, min_word_len))
        .map_or(0.0, |count| usize_to_f32(count) * 0.1);
    let domain_bonus = nova.map_or(0.0, |nova| {
        nova.domains
            .iter()
            .filter_map(|value| token_overlap_count(value.as_str(), token_set, min_word_len))
            .map(|count| usize_to_f32(count) * 0.15)
            .sum::<f32>()
    });
    let use_case_bonus = nova.map_or(0.0, |nova| {
        nova.use_cases
            .iter()
            .filter_map(|value| token_overlap_count(value.as_str(), token_set, min_word_len))
            .map(|count| usize_to_f32(count) * 0.1)
            .sum::<f32>()
    });

    (name_bonus + relation_bonus + domain_bonus + use_case_bonus).min(1.5)
}

pub(super) fn normalized_resource_type_filter(
    resource_types: &[String],
) -> Option<HashSet<String>> {
    if resource_types.is_empty() {
        return None;
    }
    Some(
        resource_types
            .iter()
            .map(|rt| rt.trim().to_lowercase())
            .filter(|rt| !rt.is_empty())
            .collect(),
    )
}

pub(super) fn normalized_value_filter(values: &[String]) -> Option<HashSet<String>> {
    if values.is_empty() {
        return None;
    }
    Some(
        values
            .iter()
            .map(|value| value.trim().to_lowercase())
            .filter(|value| !value.is_empty())
            .collect(),
    )
}

pub(super) fn resource_type_allowed_for_search(
    resource_type: Option<&str>,
    allowed_resource_types: Option<&HashSet<String>>,
) -> bool {
    let Some(allowed) = allowed_resource_types else {
        return true;
    };
    let Some(resource_type) = resource_type else {
        return false;
    };
    allowed.contains(&resource_type.trim().to_lowercase())
}
