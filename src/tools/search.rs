use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use rkyv::string::ArchivedString;
use serde::Serialize;
use serde_json::Value as JsonValue;

use crate::config::PersonaWeights;
use crate::config::search::{
    AnalystSemanticConfig, IndicatorRankingConfig, MetadataSupportConfig, SearchConfig,
};
use crate::error::{DbtNovaError, Result};
use crate::manifest::entity::{
    ArchivedColumnMetaSummary, ArchivedEntity, ArchivedNovaGrain, ArchivedNovaMeasure,
    ArchivedNovaMeta, ArchivedNovaMetric,
};
use crate::manifest::search::{
    ManifestSearch, NovaSemanticMatches, SemanticMatchType, match_nova_semantics,
};
use crate::manifest::tantivy_search::{SearchHit, SearchRequest, SearchScope, TantivySearcher};
use crate::manifest::vector_search::embedding_text_from_archived;
use crate::params::{
    ColumnInventoryParams, DetailLevel, IndicatorInventoryParams, ListEntitiesParams,
    SearchColumnsParams, SearchIndicatorParams, SearchParams,
};
use crate::responses::{SearchResponse, SuccessResponse};
use crate::utils::{SearchPersona, has_query_syntax, tokenize_alnum_lowercase};
use tracing::{debug, instrument, warn};

impl ManifestSearch {
    /// Full-text search across all entities using Tantivy. Searches names, aliases, descriptions, SQL code, file paths, column names, and tags with field boosting and relevance scoring.
    ///
    /// # Errors
    /// Returns an error if the query is invalid or search execution fails.
    #[allow(clippy::too_many_lines)]
    #[instrument(skip(self, params), fields(tool = "search", query_len = params.query.len(), limit = params.pagination.limit, offset = params.pagination.offset, fuzzy = params.fuzzy))]
    pub async fn search(&self, params: &SearchParams) -> Result<JsonValue> {
        debug!(
            query = %params.query,
            resource_types = ?params.resource_types,
            limit = params.pagination.limit,
            offset = params.pagination.offset,
            fuzzy = params.fuzzy,
            include_highlights = params.include_highlights,
            persona = ?params.persona,
            "search request"
        );
        if params.query.chars().count() > self.config.search.max_query_length {
            return Err(DbtNovaError::InvalidParams(format!(
                "Query exceeds maximum length of {} characters",
                self.config.search.max_query_length
            )));
        }

        let min_word_len = self.config.search.min_word_length.max(1);
        let query_has_syntax = has_query_syntax(&params.query);
        let tokens = tokenize_alnum_lowercase(&params.query, min_word_len);
        if tokens.is_empty() && !query_has_syntax {
            return Err(DbtNovaError::InvalidParams(
                "Query too short or invalid".to_string(),
            ));
        }

        if params.pagination.offset > self.config.search.max_offset {
            return Err(DbtNovaError::InvalidParams(format!(
                "Offset exceeds maximum of {}",
                self.config.search.max_offset
            )));
        }

        let limit = self.page_limit(params.pagination.limit);

        let detail = params.detail;
        let persona = params
            .persona
            .as_deref()
            .or(self.config.search.default_persona.as_deref())
            .map_or(SearchPersona::Default, SearchPersona::parse);
        let token_set: HashSet<&str> = tokens.iter().map(String::as_str).collect();
        let persona_weights = persona_weights(persona, &self.config.search);

        let base_limit = limit.saturating_add(params.pagination.offset);
        let overfetch = self.config.search.rrf_overfetch.max(1);
        const MAX_FETCH_LIMIT: usize = 2000;
        let fetch_limit = base_limit.saturating_mul(overfetch).min(MAX_FETCH_LIMIT);

        let primary_results = run_tantivy_search(
            self.tantivy.clone(),
            self.config.search.clone(),
            OwnedSearchRequest {
                query_text: params.query.clone(),
                resource_types: params.resource_types.clone(),
                limit: fetch_limit,
                min_score: params.min_score,
                fuzzy: params.fuzzy,
                include_highlights: params.include_highlights,
                include_ngram_override: None,
                scope: SearchScope::Full,
                persona,
            },
        )
        .await?;

        let highlight_map: HashMap<String, Option<JsonValue>> = primary_results
            .iter()
            .map(|hit| {
                let value = hit
                    .highlights
                    .as_ref()
                    .and_then(|h| serde_json::to_value(h).ok());
                (hit.unique_id.clone(), value)
            })
            .collect();

        let fused_bundle = self
            .build_fused_hits(
                params,
                persona,
                persona_weights,
                query_has_syntax,
                fetch_limit,
                &primary_results,
            )
            .await?;
        let mut fused_hits = fused_bundle.hits;
        let indicator_parent_scores = fused_bundle.indicator_parent_scores;
        let retrieval_explain = fused_bundle.retrieval_explain;
        let retrievers_used = fused_bundle.retrievers_used;
        let mut reranker_scores: HashMap<String, f32> = HashMap::new();
        let mut reranker_applied = false;

        if self.config.search.enable_reranker
            && let Some(reranker) = &self.reranker
        {
            if self.reranker_breaker.allow().await {
                let top_n = self.config.search.rerank_top_n.min(fused_hits.len());
                if top_n > 0 {
                    let mut docs: Vec<String> = Vec::new();
                    let mut ids: Vec<String> = Vec::new();
                    for (id, _) in fused_hits.iter().take(top_n) {
                        if let Some(entity) = self.get_entity_archived(id)? {
                            docs.push(embedding_text_from_archived(entity, &self.config.search));
                            ids.push(id.clone());
                        }
                    }
                    if !docs.is_empty() {
                        let query = params.query.clone();
                        let reranker = Arc::clone(reranker);
                        match tokio::task::spawn_blocking(move || {
                            reranker.rerank(&query, &docs, top_n)
                        })
                        .await
                        .map_err(|e| DbtNovaError::ServerError(e.to_string()))?
                        {
                            Ok(reranked_hits) => {
                                self.reranker_breaker.on_success().await;
                                let mut reordered: Vec<(String, f32)> = Vec::new();
                                let mut seen: HashSet<String> = HashSet::new();
                                for (idx, score) in reranked_hits {
                                    if let Some(id) = ids.get(idx).cloned()
                                        && seen.insert(id.clone())
                                    {
                                        reranker_scores.insert(id.clone(), score);
                                        reordered.push((id, score));
                                    }
                                }
                                for (id, score) in fused_hits.into_iter().skip(top_n) {
                                    if seen.insert(id.clone()) {
                                        reordered.push((id, score));
                                    }
                                }
                                fused_hits = reordered;
                                reranker_applied = true;
                            }
                            Err(err) => {
                                self.reranker_breaker.on_failure().await;
                                warn!(error = %err, "reranker failed; using fused ranking");
                            }
                        }
                    }
                }
            } else {
                debug!("reranker circuit open; skipping rerank");
            }
        }

        let suggestions =
            if fused_hits.is_empty() && self.config.search.enable_suggestions && !params.fuzzy {
                self.build_suggestions(params, persona).await?
            } else {
                Vec::new()
            };

        let persona_label = persona.as_str().to_string();
        let include_sql = params.include_sql;
        let allowed_resource_types = normalized_resource_type_filter(&params.resource_types);

        if fused_hits.is_empty() {
            let mut response = SearchResponse::new(Vec::<JsonValue>::new(), 0, persona_label);
            if !suggestions.is_empty() {
                response = response.with_suggestions(suggestions);
            }
            let mut response_json = serde_json::to_value(response)?;
            if params.explain
                && let Some(obj) = response_json.as_object_mut()
            {
                obj.insert(
                    "explain".to_string(),
                    serde_json::to_value(build_search_explain_payload(
                        &tokens,
                        query_has_syntax,
                        persona,
                        &self.config.search,
                        retrievers_used,
                        reranker_applied,
                    ))?,
                );
            }
            return Ok(response_json);
        }

        let total_hits = fused_hits.len();
        let mut candidates: Vec<SearchCandidate<'_>> = Vec::with_capacity(total_hits);
        let mut prev_score: Option<f32> = None;
        for (id, base_score) in fused_hits {
            if let Some(previous) = prev_score
                && base_score < previous * 0.01
                && candidates.len() >= base_limit
            {
                break;
            }
            prev_score = Some(base_score);
            let entity = self.get_entity_archived(&id)?;
            if !resource_type_allowed_for_search(
                entity.and_then(ArchivedEntity::resource_type_str),
                allowed_resource_types.as_ref(),
            ) {
                continue;
            }
            let support_signals = match entity {
                Some(entity_ref) => entity_ref.nova_meta().and_then(|nova| {
                    collect_metadata_support_signals(
                        entity_ref,
                        nova,
                        &token_set,
                        min_word_len,
                        &self.config.search.metadata_support,
                    )
                }),
                None => None,
            };
            let adjusted = self.adjust_score_with_meta(
                &id,
                base_score,
                entity,
                SearchScoreContext {
                    token_set: &token_set,
                    min_word_len,
                    persona,
                    query_text: &params.query,
                    support_signals: support_signals.as_ref(),
                    has_indicator_parent_scores: !indicator_parent_scores.is_empty(),
                    indicator_parent_score: indicator_parent_scores.get(&id).copied(),
                },
                retrieval_explain.get(&id).cloned(),
                reranker_scores.get(&id).copied(),
                params.explain,
            );
            candidates.push(SearchCandidate {
                indicator_parent_score: indicator_parent_scores.get(&id).copied(),
                unique_id: id,
                entity,
                score: adjusted.score,
                support_signals,
                explain: adjusted.explain,
            });
        }
        candidates.sort_by(compare_search_candidates);
        let analysis_hints: Vec<String> = if persona == SearchPersona::Analyst {
            analyst_near_tie_hint(&candidates).into_iter().collect()
        } else {
            Vec::new()
        };

        let start = params.pagination.offset.min(candidates.len());
        let end = (start + limit).min(candidates.len());

        let mut results: Vec<JsonValue> = Vec::new();
        for candidate in candidates.into_iter().skip(start).take(end - start) {
            let highlight_value = highlight_map
                .get(&candidate.unique_id)
                .cloned()
                .unwrap_or(None);

            if detail == DetailLevel::Full {
                if let Some(entity) = candidate.entity {
                    let mut entity_json =
                        serde_json::from_str(entity.payload_json()).unwrap_or(JsonValue::Null);
                    if !include_sql {
                        strip_sql_fields(&mut entity_json);
                    }
                    ManifestSearch::insert_unique_id(&mut entity_json, &candidate.unique_id);
                    if let Some(obj) = entity_json.as_object_mut() {
                        obj.insert("score".to_string(), JsonValue::from(candidate.score));
                        if let Some(highlights) = highlight_value {
                            obj.insert("highlights".to_string(), highlights);
                        }
                        if let Some(support_signals) = candidate.support_signals {
                            obj.insert(
                                "support_signals".to_string(),
                                serde_json::to_value(support_signals).unwrap_or(JsonValue::Null),
                            );
                        }
                        if let Some(explain) = candidate.explain {
                            obj.insert(
                                "explain".to_string(),
                                serde_json::to_value(explain).unwrap_or(JsonValue::Null),
                            );
                        }
                    }
                    results.push(entity_json);
                }
            } else {
                let mut summary = self.summary_from_archived(
                    &candidate.unique_id,
                    candidate.entity,
                    persona,
                    Some(&tokens),
                );
                if let Some(obj) = summary.as_object_mut() {
                    obj.insert("score".to_string(), JsonValue::from(candidate.score));
                    if let Some(highlights) = highlight_value {
                        obj.insert("highlights".to_string(), highlights);
                    }
                    if let Some(support_signals) = candidate.support_signals {
                        obj.insert(
                            "support_signals".to_string(),
                            serde_json::to_value(support_signals).unwrap_or(JsonValue::Null),
                        );
                    }
                    if let Some(explain) = candidate.explain {
                        obj.insert(
                            "explain".to_string(),
                            serde_json::to_value(explain).unwrap_or(JsonValue::Null),
                        );
                    }
                }
                results.push(summary);
            }
        }

        let count = results.len();
        let total_available = total_hits;
        let mut response =
            SearchResponse::new(results, count, persona_label).with_total(total_available);
        let truncated = total_available > end;
        if truncated {
            response = response.with_truncated(true);
        }
        if !suggestions.is_empty() {
            response = response.with_suggestions(suggestions);
        }
        if !analysis_hints.is_empty() {
            response = response.with_analysis_hints(analysis_hints);
        }
        let mut response_json = serde_json::to_value(response)?;
        if params.explain
            && let Some(obj) = response_json.as_object_mut()
        {
            obj.insert(
                "explain".to_string(),
                serde_json::to_value(build_search_explain_payload(
                    &tokens,
                    query_has_syntax,
                    persona,
                    &self.config.search,
                    retrievers_used,
                    reranker_applied,
                ))?,
            );
        }
        Ok(response_json)
    }

    /// Resolve Nova measures and metrics that match the query.
    ///
    /// # Errors
    /// Returns an error if the query is invalid or indicator filtering is invalid.
    #[instrument(skip(self, params), fields(tool = "search_indicator", query_len = params.query.len(), limit = params.pagination.limit, offset = params.pagination.offset))]
    pub async fn search_indicator(&self, params: &SearchIndicatorParams) -> Result<JsonValue> {
        let (tokens, query_has_syntax, resource_filter, indicator_filter, persona) =
            self.prepare_indicator_search(params)?;
        let mut rows = self.search_indicator_rows(
            &tokens,
            resource_filter.as_ref(),
            indicator_filter.as_ref(),
        )?;
        rows = self
            .rank_indicator_rows(params, query_has_syntax, persona, rows)
            .await?;
        self.build_indicator_search_response(params, persona, query_has_syntax, &tokens, rows)
    }

    /// List Nova measures and metrics deterministically.
    ///
    /// # Errors
    /// Returns an error if indicator filtering is invalid or pagination exceeds configured limits.
    #[instrument(skip(self, params), fields(tool = "indicator_inventory", limit = params.pagination.limit, offset = params.pagination.offset))]
    pub async fn indicator_inventory(
        &self,
        params: &IndicatorInventoryParams,
    ) -> Result<JsonValue> {
        if params.pagination.offset > self.config.search.max_offset {
            return Err(DbtNovaError::InvalidParams(format!(
                "Offset exceeds maximum of {}",
                self.config.search.max_offset
            )));
        }

        let resource_filter = normalized_resource_type_filter(&params.resource_types);
        let indicator_filter = normalized_indicator_type_filter(&params.indicator_types)?;
        let rows = self.indicator_inventory_rows(
            resource_filter.as_ref(),
            indicator_filter.as_ref(),
            params.canonical_only,
        )?;
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

    /// Search columns across entities using names, semantic hints, and example values.
    ///
    /// # Errors
    /// Returns an error if the query is invalid, filters are invalid, or pagination exceeds configured limits.
    #[instrument(skip(self, params), fields(tool = "search_columns", query_len = params.query.len(), limit = params.pagination.limit, offset = params.pagination.offset))]
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

        let resource_filter = normalized_resource_type_filter(&params.resource_types);
        let role_filter = normalized_value_filter(&params.roles);
        let semantic_type_filter = normalized_value_filter(&params.semantic_types);
        let token_set: HashSet<&str> = tokens.iter().map(String::as_str).collect();
        let mut rows = self.search_column_rows(
            &token_set,
            min_word_len,
            resource_filter.as_ref(),
            role_filter.as_ref(),
            semantic_type_filter.as_ref(),
        )?;
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
    #[instrument(skip(self, params), fields(tool = "column_inventory", limit = params.pagination.limit, offset = params.pagination.offset))]
    pub async fn column_inventory(&self, params: &ColumnInventoryParams) -> Result<JsonValue> {
        if params.pagination.offset > self.config.search.max_offset {
            return Err(DbtNovaError::InvalidParams(format!(
                "Offset exceeds maximum of {}",
                self.config.search.max_offset
            )));
        }

        let resource_filter = normalized_resource_type_filter(&params.resource_types);
        let role_filter = normalized_value_filter(&params.roles);
        let semantic_type_filter = normalized_value_filter(&params.semantic_types);
        let rows = self.column_inventory_rows(
            resource_filter.as_ref(),
            role_filter.as_ref(),
            semantic_type_filter.as_ref(),
            params.annotated_only,
        )?;
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

    fn prepare_indicator_search(
        &self,
        params: &SearchIndicatorParams,
    ) -> Result<PreparedIndicatorSearch> {
        if params.query.chars().count() > self.config.search.max_query_length {
            return Err(DbtNovaError::InvalidParams(format!(
                "Query exceeds maximum length of {} characters",
                self.config.search.max_query_length
            )));
        }

        let min_word_len = self.config.search.min_word_length.max(1);
        let query_has_syntax = has_query_syntax(&params.query);
        let tokens = tokenize_alnum_lowercase(&params.query, min_word_len);
        if tokens.is_empty() && !query_has_syntax {
            return Err(DbtNovaError::InvalidParams(
                "Query too short or invalid".to_string(),
            ));
        }
        if params.pagination.offset > self.config.search.max_offset {
            return Err(DbtNovaError::InvalidParams(format!(
                "Offset exceeds maximum of {}",
                self.config.search.max_offset
            )));
        }

        let indicator_filter = normalized_indicator_type_filter(&params.indicator_types)?;
        let resource_filter = normalized_resource_type_filter(&params.resource_types);
        let persona = params
            .persona
            .as_deref()
            .or(self.config.search.default_persona.as_deref())
            .map_or(SearchPersona::Analyst, SearchPersona::parse);
        Ok((
            tokens,
            query_has_syntax,
            resource_filter,
            indicator_filter,
            persona,
        ))
    }

    async fn rank_indicator_rows(
        &self,
        params: &SearchIndicatorParams,
        query_has_syntax: bool,
        persona: SearchPersona,
        mut rows: Vec<IndicatorSearchRow>,
    ) -> Result<Vec<IndicatorSearchRow>> {
        if self.config.search.indicator_ranking.enable_parent_coherence && rows.len() > 1 {
            rows = apply_parent_coherence_bonus(rows, &self.config.search.indicator_ranking);
        }
        if let Some(min_score) = params.min_score {
            rows.retain(|row| row.score >= min_score);
        }
        if self.config.search.enable_rrf && rows.len() > 1 {
            rows = self
                .fuse_indicator_rows(params, persona, query_has_syntax, rows)
                .await?;
        }
        if self.config.search.enable_reranker
            && let Some(reranker) = &self.reranker
        {
            if self.reranker_breaker.allow().await {
                let top_n = self.config.search.rerank_top_n.min(rows.len());
                if top_n > 0 {
                    let docs: Vec<String> = rows
                        .iter()
                        .take(top_n)
                        .map(indicator_embedding_text)
                        .collect();
                    let query = params.query.clone();
                    let reranker = Arc::clone(reranker);
                    match tokio::task::spawn_blocking(move || reranker.rerank(&query, &docs, top_n))
                        .await
                        .map_err(|e| DbtNovaError::ServerError(e.to_string()))?
                    {
                        Ok(reranked_hits) => {
                            self.reranker_breaker.on_success().await;
                            rows = reorder_indicator_rows_with_reranker(
                                rows,
                                top_n,
                                &reranked_hits,
                                &self.config.search.indicator_ranking,
                            );
                        }
                        Err(err) => {
                            self.reranker_breaker.on_failure().await;
                            warn!(
                                error = %err,
                                "indicator reranker failed; using existing indicator ranking"
                            );
                        }
                    }
                }
            } else {
                debug!("reranker circuit open; skipping indicator rerank");
            }
        }
        Ok(rows)
    }

    fn build_indicator_search_response(
        &self,
        params: &SearchIndicatorParams,
        persona: SearchPersona,
        query_has_syntax: bool,
        tokens: &[String],
        rows: Vec<IndicatorSearchRow>,
    ) -> Result<JsonValue> {
        let explain_payload = params.explain.then(|| {
            build_search_explain_payload(
                tokens,
                query_has_syntax,
                persona,
                &self.config.search,
                indicator_retrievers_used(&rows, self.config.search.enable_rrf),
                rows.iter().any(|row| {
                    row.explain
                        .as_ref()
                        .and_then(|explain| explain.reranker_bonus)
                        .is_some()
                }),
            )
        });
        let parent_groups = build_indicator_parent_groups(
            &rows,
            &self.config.search.indicator_ranking,
            &self.config.search.metadata_support,
        );
        let total_available = rows.len();
        let limit = self.page_limit(params.pagination.limit);
        let start = params.pagination.offset.min(rows.len());
        let end = (start + limit).min(rows.len());
        let results: Vec<JsonValue> = rows
            .into_iter()
            .skip(start)
            .take(end.saturating_sub(start))
            .map(serde_json::to_value)
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|error| DbtNovaError::ServerError(error.to_string()))?;

        let count = results.len();
        let mut response = SearchResponse::new(results, count, persona.as_str().to_string())
            .with_total(total_available);
        if total_available > end {
            response = response.with_truncated(true);
        }
        let mut response_json = serde_json::to_value(response)?;
        if let Some(obj) = response_json.as_object_mut() {
            obj.insert(
                "parent_groups".to_string(),
                serde_json::to_value(parent_groups)?,
            );
            if let Some(explain) = explain_payload {
                obj.insert("explain".to_string(), serde_json::to_value(explain)?);
            }
        }
        Ok(response_json)
    }

    async fn fuse_indicator_rows(
        &self,
        params: &SearchIndicatorParams,
        persona: SearchPersona,
        query_has_syntax: bool,
        mut rows: Vec<IndicatorSearchRow>,
    ) -> Result<Vec<IndicatorSearchRow>> {
        let fetch_limit = rows.len().max(1);
        let search_params = SearchParams {
            query: params.query.clone(),
            resource_types: params.resource_types.clone(),
            persona: Some(persona.as_str().to_string()),
            detail: DetailLevel::Standard,
            pagination: crate::params::PaginationParams {
                limit: fetch_limit,
                offset: 0,
            },
            min_score: None,
            fuzzy: false,
            include_highlights: false,
            include_sql: false,
            explain: false,
        };
        let persona_weights = persona_weights(persona, &self.config.search);
        let primary_results = run_tantivy_search(
            self.tantivy.clone(),
            self.config.search.clone(),
            OwnedSearchRequest {
                query_text: params.query.clone(),
                resource_types: params.resource_types.clone(),
                limit: fetch_limit,
                min_score: None,
                fuzzy: false,
                include_highlights: false,
                include_ngram_override: None,
                scope: SearchScope::Full,
                persona,
            },
        )
        .await?;
        let parent_hits = self
            .build_fused_hits(
                &search_params,
                persona,
                persona_weights,
                query_has_syntax,
                fetch_limit,
                &primary_results,
            )
            .await?;

        let local_ranking: Vec<String> = rows.iter().map(indicator_row_key).collect();
        let parent_ranking = expand_indicator_parent_ranking(&rows, &parent_hits.hits);
        if parent_ranking.is_empty() {
            return Ok(rows);
        }

        let fused_scores = weighted_rrf_with_explain(
            &[
                ("indicator_local", local_ranking),
                ("indicator_parent", parent_ranking),
            ],
            &persona_weights,
            self.config.search.rrf_k,
        );
        let fused_score_map: HashMap<String, RetrievalExplain> = fused_scores.into_iter().collect();

        let rrf_weight = self
            .config
            .search
            .indicator_ranking
            .indicator_rrf_score_weight;
        for row in &mut rows {
            if let Some(rrf_explain) = fused_score_map.get(&indicator_row_key(row)) {
                let bonus = rrf_explain.total_score * rrf_weight;
                row.score += bonus;
                if let Some(explain) = &mut row.explain {
                    explain.rrf_bonus = Some(bonus);
                    explain.retrieval = Some(rrf_explain.clone());
                    explain.final_score = row.score;
                }
            }
        }
        rows.sort_by(compare_indicator_rows);
        Ok(rows)
    }

    fn search_indicator_rows(
        &self,
        tokens: &[String],
        resource_filter: Option<&HashSet<String>>,
        indicator_filter: Option<&HashSet<String>>,
    ) -> Result<Vec<IndicatorSearchRow>> {
        let min_word_len = self.config.search.min_word_length.max(1);
        let token_set: HashSet<&str> = tokens.iter().map(String::as_str).collect();
        let query_token_count = token_set.len();
        let mut rows: Vec<IndicatorSearchRow> = Vec::new();

        for unique_id in self.entities.ids() {
            let Some(entity) = self.get_entity_archived(unique_id)? else {
                continue;
            };
            if !resource_type_allowed_for_search(entity.resource_type_str(), resource_filter) {
                continue;
            }
            let Some(nova) = entity.nova_meta() else {
                continue;
            };
            let matches = match_nova_semantics(nova, &token_set, min_word_len);
            if matches.is_empty() {
                continue;
            }

            let support_signals = collect_metadata_support_signals(
                entity,
                nova,
                &token_set,
                min_word_len,
                &self.config.search.metadata_support,
            );
            let context = IndicatorSearchContext {
                unique_id,
                entity,
                nova,
                token_set: &token_set,
                query_token_count,
                min_word_len,
                support_signals,
                indicator_config: &self.config.search.indicator_ranking,
                metadata_config: &self.config.search.metadata_support,
            };

            if indicator_type_selected(indicator_filter, "measure") {
                for matched in &matches.measures {
                    if let Some(measure) = nova
                        .measures
                        .iter()
                        .find(|measure| measure.name.as_str() == matched.name)
                    {
                        rows.push(build_measure_indicator_row(&context, measure, matched));
                    }
                }
            }

            if indicator_type_selected(indicator_filter, "metric") {
                if let Some(metric) = nova.metric.as_ref().filter(|metric| {
                    matches
                        .metrics
                        .iter()
                        .any(|item| item.name == metric.name.as_str())
                }) && let Some(matched) = matches
                    .metrics
                    .iter()
                    .find(|item| item.name == metric.name.as_str())
                {
                    rows.push(build_metric_indicator_row(&context, metric, matched));
                }

                for matched in &matches.metrics {
                    if let Some(metric) = nova
                        .metrics
                        .iter()
                        .find(|metric| metric.name.as_str() == matched.name)
                    {
                        rows.push(build_metric_indicator_row(&context, metric, matched));
                    }
                }
            }
        }

        rows.sort_by(compare_indicator_rows);
        Ok(rows)
    }

    fn indicator_inventory_rows(
        &self,
        resource_filter: Option<&HashSet<String>>,
        indicator_filter: Option<&HashSet<String>>,
        canonical_only: bool,
    ) -> Result<Vec<IndicatorInventoryRow>> {
        let mut rows = Vec::new();

        for unique_id in self.entities.ids() {
            let Some(entity) = self.get_entity_archived(unique_id)? else {
                continue;
            };
            if !resource_type_allowed_for_search(entity.resource_type_str(), resource_filter) {
                continue;
            }
            let Some(nova) = entity.nova_meta() else {
                continue;
            };

            if indicator_type_selected(indicator_filter, "measure") {
                for measure in nova.measures.iter() {
                    let canonical =
                        inventory_indicator_is_canonical(nova.canonical, measure.canonical);
                    if canonical_only && !canonical {
                        continue;
                    }
                    rows.push(build_measure_inventory_row(
                        unique_id, entity, nova, measure, canonical,
                    ));
                }
            }

            if indicator_type_selected(indicator_filter, "metric") {
                if let Some(metric) = nova.metric.as_ref() {
                    let canonical =
                        inventory_indicator_is_canonical(nova.canonical, metric.canonical);
                    if canonical_only && !canonical {
                        // skip
                    } else {
                        rows.push(build_metric_inventory_row(
                            unique_id, entity, nova, metric, canonical,
                        ));
                    }
                }
                for metric in nova.metrics.iter() {
                    let canonical =
                        inventory_indicator_is_canonical(nova.canonical, metric.canonical);
                    if canonical_only && !canonical {
                        continue;
                    }
                    rows.push(build_metric_inventory_row(
                        unique_id, entity, nova, metric, canonical,
                    ));
                }
            }
        }

        rows.sort_by(compare_indicator_inventory_rows);
        Ok(rows)
    }

    fn search_column_rows(
        &self,
        token_set: &HashSet<&str>,
        min_word_len: usize,
        resource_filter: Option<&HashSet<String>>,
        role_filter: Option<&HashSet<String>>,
        semantic_type_filter: Option<&HashSet<String>>,
    ) -> Result<Vec<ColumnSearchRow>> {
        let mut rows = Vec::new();

        for unique_id in self.entities.ids() {
            let Some(entity) = self.get_entity_archived(unique_id)? else {
                continue;
            };
            if !resource_type_allowed_for_search(entity.resource_type_str(), resource_filter) {
                continue;
            }
            let nova = entity.nova_meta();
            let column_meta = entity_column_meta_lookup(entity);

            for column_name in entity.column_names_iter() {
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

    fn column_inventory_rows(
        &self,
        resource_filter: Option<&HashSet<String>>,
        role_filter: Option<&HashSet<String>>,
        semantic_type_filter: Option<&HashSet<String>>,
        annotated_only: bool,
    ) -> Result<Vec<ColumnInventoryRow>> {
        let mut rows = Vec::new();

        for unique_id in self.entities.ids() {
            let Some(entity) = self.get_entity_archived(unique_id)? else {
                continue;
            };
            if !resource_type_allowed_for_search(entity.resource_type_str(), resource_filter) {
                continue;
            }
            let nova = entity.nova_meta();
            let column_meta = entity_column_meta_lookup(entity);

            for column_name in entity.column_names_iter() {
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

    #[allow(clippy::too_many_lines)]
    async fn build_fused_hits(
        &self,
        params: &SearchParams,
        persona: SearchPersona,
        persona_weights: PersonaWeights,
        query_has_syntax: bool,
        fetch_limit: usize,
        primary_results: &[SearchHit],
    ) -> Result<FusedHitBundle> {
        if !self.config.search.enable_rrf {
            let hits: Vec<(String, f32)> = primary_results
                .iter()
                .map(|hit| (hit.unique_id.clone(), hit.score))
                .collect();
            let retrieval_explain = primary_results
                .iter()
                .enumerate()
                .map(|(index, hit)| {
                    let mut retrievers = BTreeMap::new();
                    retrievers.insert(
                        "primary".to_string(),
                        RetrieverContribution {
                            rank: index + 1,
                            score: hit.score,
                        },
                    );
                    (
                        hit.unique_id.clone(),
                        RetrievalExplain {
                            total_score: hit.score,
                            retrievers,
                        },
                    )
                })
                .collect();
            return Ok(FusedHitBundle {
                hits,
                indicator_parent_scores: HashMap::new(),
                retrieval_explain,
                retrievers_used: vec!["primary".to_string()],
            });
        }

        let mut rankings: Vec<(&str, Vec<String>)> = Vec::new();
        let mut indicator_parent_scores = HashMap::new();

        let bm25_exact = run_tantivy_search(
            self.tantivy.clone(),
            self.config.search.clone(),
            OwnedSearchRequest {
                query_text: params.query.clone(),
                resource_types: params.resource_types.clone(),
                limit: fetch_limit,
                min_score: params.min_score,
                fuzzy: false,
                include_highlights: false,
                include_ngram_override: Some(false),
                scope: SearchScope::Full,
                persona,
            },
        )
        .await?;
        rankings.push(("bm25", hits_to_ids(&bm25_exact)));

        if self.config.search.enable_ngram && !query_has_syntax {
            let ngram_hits = run_tantivy_search(
                self.tantivy.clone(),
                self.config.search.clone(),
                OwnedSearchRequest {
                    query_text: params.query.clone(),
                    resource_types: params.resource_types.clone(),
                    limit: fetch_limit,
                    min_score: params.min_score,
                    fuzzy: false,
                    include_highlights: false,
                    include_ngram_override: Some(true),
                    scope: SearchScope::Full,
                    persona,
                },
            )
            .await?;
            rankings.push(("ngram", hits_to_ids(&ngram_hits)));
        }

        if params.fuzzy {
            let fuzzy_hits = run_tantivy_search(
                self.tantivy.clone(),
                self.config.search.clone(),
                OwnedSearchRequest {
                    query_text: params.query.clone(),
                    resource_types: params.resource_types.clone(),
                    limit: fetch_limit,
                    min_score: params.min_score,
                    fuzzy: true,
                    include_highlights: false,
                    include_ngram_override: Some(false),
                    scope: SearchScope::Full,
                    persona,
                },
            )
            .await?;
            rankings.push(("fuzzy", hits_to_ids(&fuzzy_hits)));
        }

        if self.config.search.enable_vector_search
            && let Some(vector_search) = &self.vector_search
        {
            if self.vector_breaker.allow().await {
                let vector_limit = self.config.search.vector_top_k.max(fetch_limit);
                let query = params.query.clone();
                let vector_search = Arc::clone(vector_search);
                match tokio::task::spawn_blocking(move || {
                    vector_search.search(&query, vector_limit)
                })
                .await
                .map_err(|e| DbtNovaError::ServerError(e.to_string()))?
                {
                    Ok(vector_hits) => {
                        self.vector_breaker.on_success().await;
                        rankings.push(("vector", scores_to_ids(&vector_hits)));
                    }
                    Err(err) => {
                        self.vector_breaker.on_failure().await;
                        warn!(error = %err, "vector search failed; skipping vector results");
                    }
                }
            } else {
                debug!("vector search circuit open; skipping vector retriever");
            }
        }

        if self.config.search.enable_sparse_search
            && let Some(sparse_search) = &self.sparse_search
        {
            if self.sparse_breaker.allow().await {
                let sparse_limit = self.config.search.sparse_top_k.max(fetch_limit);
                let query = params.query.clone();
                let sparse_search = Arc::clone(sparse_search);
                match tokio::task::spawn_blocking(move || {
                    sparse_search.search(&query, sparse_limit)
                })
                .await
                .map_err(|e| DbtNovaError::ServerError(e.to_string()))?
                {
                    Ok(sparse_hits) => {
                        self.sparse_breaker.on_success().await;
                        rankings.push(("sparse", scores_to_ids(&sparse_hits)));
                    }
                    Err(err) => {
                        self.sparse_breaker.on_failure().await;
                        warn!(error = %err, "sparse search failed; skipping sparse results");
                    }
                }
            } else {
                debug!("sparse search circuit open; skipping sparse retriever");
            }
        }

        if persona == SearchPersona::Analyst {
            let indicator_tokens =
                tokenize_alnum_lowercase(&params.query, self.config.search.min_word_length.max(1));
            let resource_filter = normalized_resource_type_filter(&params.resource_types);
            let mut indicator_rows =
                self.search_indicator_rows(&indicator_tokens, resource_filter.as_ref(), None)?;
            if self.config.search.indicator_ranking.enable_parent_coherence
                && indicator_rows.len() > 1
            {
                indicator_rows = apply_parent_coherence_bonus(
                    indicator_rows,
                    &self.config.search.indicator_ranking,
                );
            }
            let indicator_ranking = dedupe_indicator_parent_ids(&indicator_rows, fetch_limit);
            if !indicator_ranking.is_empty() {
                rankings.push(("indicator", indicator_ranking));
                indicator_parent_scores = normalized_indicator_parent_scores(
                    &indicator_rows,
                    &self.config.search.indicator_ranking,
                );
            }
        }

        let fusion_limit = fetch_limit.max(1);
        for (_, ranking) in &mut rankings {
            if ranking.len() > fusion_limit {
                ranking.truncate(fusion_limit);
            }
        }

        let retrievers_used = rankings
            .iter()
            .map(|(name, _)| (*name).to_string())
            .collect::<Vec<_>>();
        let weighted =
            weighted_rrf_with_explain(&rankings, &persona_weights, self.config.search.rrf_k);
        let hits = weighted
            .iter()
            .map(|(id, explain)| (id.clone(), explain.total_score))
            .collect();
        let retrieval_explain = weighted.into_iter().collect();

        Ok(FusedHitBundle {
            hits,
            indicator_parent_scores,
            retrieval_explain,
            retrievers_used,
        })
    }

    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    fn adjust_score_with_meta(
        &self,
        unique_id: &str,
        score: f32,
        entity: Option<&ArchivedEntity>,
        context: SearchScoreContext<'_>,
        retrieval_explain: Option<RetrievalExplain>,
        reranker_score: Option<f32>,
        explain: bool,
    ) -> SearchScoreOutcome {
        if score <= 0.0 {
            return SearchScoreOutcome {
                score,
                explain: explain.then_some(SearchScoreExplain {
                    base_score: score,
                    retrieval: retrieval_explain,
                    pre_rerank_retrieval_score: None,
                    reranker_score,
                    exact_match: false,
                    resource_type_multiplier: 1.0,
                    staging_deboost_factor: None,
                    missing_nova_multiplier: None,
                    candidate_false_multiplier: None,
                    measure_match: false,
                    metric_match: false,
                    synonym_match: false,
                    canonical_entity: false,
                    canonical_semantic_match: false,
                    engineer_exact_match_multiplier: None,
                    canonical_entity_multiplier: None,
                    measure_match_multiplier: None,
                    metric_match_multiplier: None,
                    synonym_match_multiplier: None,
                    strongest_match_type: None,
                    semantic_match_multiplier: None,
                    query_coverage_multiplier: None,
                    semantic_canonical_match_multiplier: None,
                    semantic_canonical_match_bonus: None,
                    canonical_match_multiplier: None,
                    canonical_match_bonus: None,
                    analyst_semantic_multiplier: None,
                    semantic_label_precision_factor: None,
                    metadata_support_factor: None,
                    indicator_parent_factor: None,
                    docs_multiplier: None,
                    tests_multiplier: None,
                    tags_multiplier: None,
                    path_multiplier: None,
                    final_score: score,
                }),
            };
        }

        let mut adjusted = score;
        let weights = persona_weights(context.persona, &self.config.search);
        let mut explain_payload = explain.then_some(SearchScoreExplain {
            base_score: score,
            pre_rerank_retrieval_score: retrieval_explain.as_ref().map(|value| value.total_score),
            retrieval: retrieval_explain,
            reranker_score,
            exact_match: false,
            resource_type_multiplier: 1.0,
            staging_deboost_factor: None,
            missing_nova_multiplier: None,
            candidate_false_multiplier: None,
            measure_match: false,
            metric_match: false,
            synonym_match: false,
            canonical_entity: false,
            canonical_semantic_match: false,
            engineer_exact_match_multiplier: None,
            canonical_entity_multiplier: None,
            measure_match_multiplier: None,
            metric_match_multiplier: None,
            synonym_match_multiplier: None,
            strongest_match_type: None,
            semantic_match_multiplier: None,
            query_coverage_multiplier: None,
            semantic_canonical_match_multiplier: None,
            semantic_canonical_match_bonus: None,
            canonical_match_multiplier: None,
            canonical_match_bonus: None,
            analyst_semantic_multiplier: None,
            semantic_label_precision_factor: None,
            metadata_support_factor: None,
            indicator_parent_factor: None,
            docs_multiplier: None,
            tests_multiplier: None,
            tags_multiplier: None,
            path_multiplier: None,
            final_score: score,
        });
        let Some(entity) = entity else {
            return SearchScoreOutcome {
                score: adjusted,
                explain: explain_payload,
            };
        };
        let exact_match = query_exact_match(context.query_text, unique_id, entity);
        if let Some(debug) = &mut explain_payload {
            debug.exact_match = exact_match;
        }

        let deboost = self.config.search.staging_deboost_factor;
        if deboost < 1.0
            && self.layer_for(entity).is_some_and(|layer| {
                matches!(
                    layer.trim().to_ascii_lowercase().as_str(),
                    "staging" | "stage" | "stg"
                )
            })
        {
            adjusted *= deboost;
            if let Some(debug) = &mut explain_payload {
                debug.staging_deboost_factor = Some(deboost);
            }
        }

        if context.persona == SearchPersona::Engineer && exact_match {
            adjusted *= self.config.search.engineer_exact_match_multiplier;
            if let Some(debug) = &mut explain_payload {
                debug.engineer_exact_match_multiplier =
                    Some(self.config.search.engineer_exact_match_multiplier);
            }
        }

        let resource_type = entity.resource_type_str().unwrap_or("");
        let resource_type_multiplier =
            persona_resource_type_multiplier(context.persona, resource_type);
        adjusted *= resource_type_multiplier;
        if let Some(debug) = &mut explain_payload {
            debug.resource_type_multiplier = resource_type_multiplier;
        }

        if context.token_set.is_empty() {
            if let Some(debug) = &mut explain_payload {
                debug.final_score = adjusted;
            }
            return SearchScoreOutcome {
                score: adjusted,
                explain: explain_payload,
            };
        }

        let Some(nova) = entity.nova_meta() else {
            if context.persona == SearchPersona::Analyst {
                adjusted *= 0.93;
                if let Some(debug) = &mut explain_payload {
                    debug.missing_nova_multiplier = Some(0.93);
                }
            }
            if let Some(debug) = &mut explain_payload {
                debug.final_score = adjusted;
            }
            return SearchScoreOutcome {
                score: adjusted,
                explain: explain_payload,
            };
        };

        let candidate_false_multiplier = candidate_false_multiplier(
            context.persona,
            candidate_flag_for_persona(nova, context.persona),
            exact_match,
            &self.config.search,
        );
        adjusted *= candidate_false_multiplier;
        if let Some(debug) = &mut explain_payload {
            debug.candidate_false_multiplier = non_neutral_option(candidate_false_multiplier);
            debug.canonical_entity = nova.canonical;
        }

        let semantic_matches = match_nova_semantics(nova, context.token_set, context.min_word_len);
        let measure_match = semantic_matches.has_measure_match();
        let metric_match = semantic_matches.has_metric_match();

        let mut synonym_match = false;
        for syn in nova.synonyms.iter() {
            if tokens_match(syn.as_str(), context.token_set, context.min_word_len) {
                synonym_match = true;
                break;
            }
        }
        if let Some(debug) = &mut explain_payload {
            debug.measure_match = measure_match;
            debug.metric_match = metric_match;
            debug.synonym_match = synonym_match;
            debug.canonical_semantic_match = semantic_matches.has_canonical_match();
        }

        if measure_match {
            let measure_multiplier =
                self.config.search.nova_measure_match_multiplier * weights.measures;
            adjusted *= measure_multiplier;
            if let Some(debug) = &mut explain_payload {
                debug.measure_match_multiplier = Some(measure_multiplier);
            }
        }
        if metric_match {
            let metric_multiplier =
                self.config.search.nova_metric_match_multiplier * weights.metrics;
            adjusted *= metric_multiplier;
            if let Some(debug) = &mut explain_payload {
                debug.metric_match_multiplier = Some(metric_multiplier);
            }
        }
        if let Some(match_type) = semantic_matches.strongest_match_type() {
            let semantic_match_multiplier =
                persona_semantic_match_multiplier(context.persona, &self.config.search)
                    * match_type.multiplier(&self.config.search);
            adjusted *= semantic_match_multiplier;
            if let Some(debug) = &mut explain_payload {
                debug.strongest_match_type = Some(match_type.as_str().to_string());
                debug.semantic_match_multiplier = Some(semantic_match_multiplier);
            }
        }
        if context.persona == SearchPersona::Analyst {
            let query_coverage_multiplier = analyst_query_coverage_multiplier(
                semantic_matches.best_query_coverage(),
                context.token_set.len(),
            );
            adjusted *= query_coverage_multiplier;
            if let Some(debug) = &mut explain_payload {
                debug.query_coverage_multiplier = Some(query_coverage_multiplier);
            }
        }
        if synonym_match {
            let synonym_match_multiplier =
                self.config.search.nova_synonym_match_multiplier * weights.synonyms;
            adjusted *= synonym_match_multiplier;
            if let Some(debug) = &mut explain_payload {
                debug.synonym_match_multiplier = Some(synonym_match_multiplier);
            }
        }
        let canonical = nova.canonical;
        if canonical {
            adjusted *= self.config.search.nova_canonical_multiplier;
            if let Some(debug) = &mut explain_payload {
                debug.canonical_entity_multiplier =
                    Some(self.config.search.nova_canonical_multiplier);
            }
        }
        if semantic_matches.has_canonical_match() {
            adjusted *= self.config.search.nova_semantic_canonical_match_multiplier;
            adjusted += self.config.search.nova_semantic_canonical_match_bonus;
            if let Some(debug) = &mut explain_payload {
                debug.semantic_canonical_match_multiplier =
                    Some(self.config.search.nova_semantic_canonical_match_multiplier);
                debug.semantic_canonical_match_bonus =
                    Some(self.config.search.nova_semantic_canonical_match_bonus);
            }
        }
        if canonical && (measure_match || metric_match || synonym_match) {
            adjusted *= self.config.search.nova_canonical_match_multiplier;
            adjusted += self.config.search.nova_canonical_match_bonus;
            if let Some(debug) = &mut explain_payload {
                debug.canonical_match_multiplier =
                    Some(self.config.search.nova_canonical_match_multiplier);
                debug.canonical_match_bonus = Some(self.config.search.nova_canonical_match_bonus);
            }
        }

        if context.persona == SearchPersona::Analyst {
            let analyst_semantic_multiplier = analyst_semantic_multiplier(
                nova,
                context.token_set,
                context.min_word_len,
                &self.config.search.analyst_semantic,
            );
            adjusted *= analyst_semantic_multiplier;
            let semantic_label_precision_factor = 1.0
                + semantic_label_precision_bonus(
                    &semantic_matches,
                    context.token_set,
                    context.min_word_len,
                    &self.config.search.indicator_ranking,
                );
            adjusted *= semantic_label_precision_factor;
            let metadata_support_factor = 1.0
                + metadata_support_bonus(
                    context.support_signals,
                    &self.config.search.metadata_support,
                );
            adjusted *= metadata_support_factor;
            if context.has_indicator_parent_scores {
                let ranking_config = &self.config.search.indicator_ranking;
                let indicator_parent_score = context.indicator_parent_score.unwrap_or_default();
                let indicator_parent_factor = ranking_config
                    .search_missing_indicator_parent_multiplier
                    + (indicator_parent_score * ranking_config.search_parent_indicator_bonus_scale);
                adjusted *= indicator_parent_factor;
                if let Some(debug) = &mut explain_payload {
                    debug.indicator_parent_factor = Some(indicator_parent_factor);
                }
            }
            if let Some(debug) = &mut explain_payload {
                debug.analyst_semantic_multiplier = Some(analyst_semantic_multiplier);
                debug.semantic_label_precision_factor =
                    non_neutral_option(semantic_label_precision_factor);
                debug.metadata_support_factor = non_neutral_option(metadata_support_factor);
            }
        }

        if !is_neutral_multiplier(weights.docs) {
            let has_desc = entity
                .description_str()
                .is_some_and(|d| !d.trim().is_empty());
            let has_docs = entity.doc_blocks_present();
            if has_desc || has_docs {
                adjusted *= weights.docs;
                if let Some(debug) = &mut explain_payload {
                    debug.docs_multiplier = Some(weights.docs);
                }
            }
        }

        if !is_neutral_multiplier(weights.tests) && self.tests_by_entity.contains_key(unique_id) {
            adjusted *= weights.tests;
            if let Some(debug) = &mut explain_payload {
                debug.tests_multiplier = Some(weights.tests);
            }
        }

        if !is_neutral_multiplier(weights.tags) {
            let has_tags = entity.tags_iter().next().is_some();
            if has_tags {
                adjusted *= weights.tags;
                if let Some(debug) = &mut explain_payload {
                    debug.tags_multiplier = Some(weights.tags);
                }
            }
        }

        if !is_neutral_multiplier(weights.path) {
            let path = entity.original_file_path_str().unwrap_or("");
            if tokens_match(path, context.token_set, context.min_word_len) {
                adjusted *= weights.path;
                if let Some(debug) = &mut explain_payload {
                    debug.path_multiplier = Some(weights.path);
                }
            }
        }

        if let Some(debug) = &mut explain_payload {
            debug.final_score = adjusted;
        }

        SearchScoreOutcome {
            score: adjusted,
            explain: explain_payload,
        }
    }

    async fn build_suggestions(
        &self,
        params: &SearchParams,
        persona: SearchPersona,
    ) -> Result<Vec<String>> {
        let limit = self.config.search.suggestions_limit;
        if limit == 0 || params.query.trim().is_empty() {
            return Ok(Vec::new());
        }

        let hits = run_tantivy_search(
            self.tantivy.clone(),
            self.config.search.clone(),
            OwnedSearchRequest {
                query_text: params.query.clone(),
                resource_types: params.resource_types.clone(),
                limit,
                min_score: None,
                fuzzy: true,
                include_highlights: false,
                include_ngram_override: None,
                scope: SearchScope::Suggestion,
                persona,
            },
        )
        .await?;

        let mut suggestions = Vec::new();
        let mut seen = HashSet::new();

        for hit in hits {
            if suggestions.len() >= limit {
                break;
            }

            if let Some(entity) = self.get_entity_archived(&hit.unique_id)? {
                let resource_type = entity.resource_type_str();
                if !suggestion_allowed(persona, resource_type) {
                    continue;
                }
                if let Some(name) = entity.name_str()
                    && seen.insert(name.to_string())
                {
                    suggestions.push(name.to_string());
                    if suggestions.len() >= limit {
                        break;
                    }
                }
                if let Some(alias) = entity.alias_str()
                    && seen.insert(alias.to_string())
                {
                    suggestions.push(alias.to_string());
                }
            }
        }

        Ok(suggestions)
    }

    /// List entities by type with optional filtering.
    ///
    /// # Errors
    /// Returns an error if manifest access fails.
    #[instrument(skip(self, params), fields(tool = "list_entities", resource_type = %params.resource_type, limit = params.pagination.limit, offset = params.pagination.offset))]
    #[allow(clippy::too_many_lines)]
    pub async fn list_entities(&self, params: &ListEntitiesParams) -> Result<JsonValue> {
        let resource_type_key = self.normalize_resource_type_key(&params.resource_type)?;
        let candidates = self
            .by_resource_type
            .get(&resource_type_key)
            .map(Vec::as_slice)
            .ok_or_else(|| {
                DbtNovaError::ServerError(format!(
                    "resource_type '{resource_type_key}' resolved but was not indexed"
                ))
            })?;

        let mut allowed: Option<HashSet<String>> = None;

        fn apply_vec_filter(allowed: &mut Option<HashSet<String>>, ids: &[String]) {
            let ids_set: HashSet<String> = ids.iter().cloned().collect();
            if let Some(current) = allowed {
                current.retain(|id| ids_set.contains(id));
            } else {
                *allowed = Some(ids_set);
            }
        }

        fn apply_set_filter(allowed: &mut Option<HashSet<String>>, ids: &HashSet<String>) {
            if let Some(current) = allowed {
                current.retain(|id| ids.contains(id));
            } else {
                *allowed = Some(ids.iter().cloned().collect());
            }
        }

        if let Some(ref pkg) = params.package {
            match self.by_package.get(pkg) {
                Some(ids) => apply_vec_filter(&mut allowed, ids),
                None => {
                    return Ok(serde_json::to_value(SuccessResponse::new(
                        Vec::<JsonValue>::new(),
                        0,
                    ))?);
                }
            }
        }

        if let Some(ref db_schema) = params.database_schema {
            if let Some(ids) = self.by_database_schema.get(db_schema) {
                apply_vec_filter(&mut allowed, ids);
            } else {
                let mut matches: HashSet<String> = HashSet::new();
                for (key, ids) in &self.by_database_schema {
                    if key.contains(db_schema) {
                        matches.extend(ids.iter().cloned());
                    }
                }
                if matches.is_empty() {
                    return Ok(serde_json::to_value(SuccessResponse::new(
                        Vec::<JsonValue>::new(),
                        0,
                    ))?);
                }
                if let Some(ref mut current) = allowed {
                    current.retain(|id| matches.contains(id));
                } else {
                    allowed = Some(matches);
                }
            }
        }

        if !params.tags.is_empty() {
            for tag in &params.tags {
                match self.by_tag.get(tag) {
                    Some(ids) => apply_set_filter(&mut allowed, ids),
                    None => {
                        return Ok(serde_json::to_value(SuccessResponse::new(
                            Vec::<JsonValue>::new(),
                            0,
                        ))?);
                    }
                }
            }
        }

        let detail = params.detail;
        let limit = self.page_limit(params.pagination.limit);
        let mut total = 0usize;
        let mut skipped = 0usize;
        let mut results: Vec<JsonValue> = Vec::with_capacity(limit);

        for id in candidates {
            let allowed_match = match allowed.as_ref() {
                Some(allowed_set) => allowed_set.contains(id),
                None => true,
            };
            if !allowed_match {
                continue;
            }

            total += 1;
            if skipped < params.pagination.offset {
                skipped += 1;
                continue;
            }
            if results.len() >= limit {
                continue;
            }

            if detail == DetailLevel::Full {
                if let Some(entity) = self.get_entity_archived(id)? {
                    results.push(ManifestSearch::with_unique_id(entity.to_json_value(), id));
                }
            } else if let Ok(summary) = self.entity_summary(id) {
                results.push(summary);
            }
        }

        let count = results.len();
        let mut response = SuccessResponse::new(results, count).with_total(total);
        if total > count + params.pagination.offset {
            response = response.with_truncated(true);
        }
        Ok(serde_json::to_value(response)?)
    }
}

#[derive(Debug, Clone, Serialize)]
struct IndicatorGrainSummary {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    primary_key: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    time_field: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    dimensions: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct IndicatorSearchRow {
    indicator_name: String,
    indicator_type: String,
    canonical: bool,
    match_type: String,
    score: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expression: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    field: Option<String>,
    parent_unique_id: String,
    parent_name: String,
    parent_resource_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    relation_name: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    domains: Vec<String>,
    grain: IndicatorGrainSummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    support_signals: Option<MetadataSupportSignals>,
    #[serde(skip_serializing_if = "Option::is_none")]
    explain: Option<IndicatorScoreExplain>,
}

#[derive(Debug, Clone, Serialize)]
struct IndicatorInventoryRow {
    indicator_name: String,
    indicator_type: String,
    canonical: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    synonyms: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expression: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    field: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    measure_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    template: Option<bool>,
    parent_unique_id: String,
    parent_name: String,
    parent_resource_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    relation_name: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    domains: Vec<String>,
    grain: IndicatorGrainSummary,
}

#[derive(Debug, Clone, Serialize)]
struct ColumnInventoryRow {
    column_name: String,
    annotated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    semantic_type: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    synonyms: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    example_values: Vec<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    primary_key: bool,
    parent_unique_id: String,
    parent_name: String,
    parent_resource_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    relation_name: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    domains: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct ColumnSearchRow {
    column_name: String,
    match_type: String,
    score: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    matched_value: Option<String>,
    annotated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    semantic_type: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    synonyms: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    example_values: Vec<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    primary_key: bool,
    parent_unique_id: String,
    parent_name: String,
    parent_resource_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    relation_name: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    domains: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct IndicatorParentGroupItem {
    indicator_name: String,
    indicator_type: String,
    canonical: bool,
    match_type: String,
    score: f32,
}

#[derive(Debug, Clone, Serialize)]
struct IndicatorParentGroup {
    parent_unique_id: String,
    parent_name: String,
    parent_resource_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    relation_name: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    domains: Vec<String>,
    best_score: f32,
    indicator_count: usize,
    grain: IndicatorGrainSummary,
    indicators: Vec<IndicatorParentGroupItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    support_signals: Option<MetadataSupportSignals>,
}

#[derive(Debug, Clone, Default, Serialize)]
struct MetadataSupportSignals {
    #[serde(
        rename = "matched_parent_synonyms",
        skip_serializing_if = "Vec::is_empty"
    )]
    parent_synonyms: Vec<String>,
    #[serde(rename = "matched_domains", skip_serializing_if = "Vec::is_empty")]
    domains: Vec<String>,
    #[serde(rename = "matched_use_cases", skip_serializing_if = "Vec::is_empty")]
    use_cases: Vec<String>,
    #[serde(rename = "matched_dimensions", skip_serializing_if = "Vec::is_empty")]
    dimensions: Vec<String>,
    #[serde(rename = "matched_column_names", skip_serializing_if = "Vec::is_empty")]
    column_names: Vec<String>,
    #[serde(rename = "matched_column_roles", skip_serializing_if = "Vec::is_empty")]
    column_roles: Vec<String>,
    #[serde(
        rename = "matched_column_semantic_types",
        skip_serializing_if = "Vec::is_empty"
    )]
    column_semantic_types: Vec<String>,
    #[serde(
        rename = "matched_example_values",
        skip_serializing_if = "Vec::is_empty"
    )]
    example_values: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct RetrieverContribution {
    rank: usize,
    score: f32,
}

#[derive(Debug, Clone, Default, Serialize)]
struct RetrievalExplain {
    total_score: f32,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    retrievers: BTreeMap<String, RetrieverContribution>,
}

#[derive(Debug, Clone, Serialize)]
#[allow(clippy::struct_excessive_bools)]
struct SearchScoreExplain {
    base_score: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    retrieval: Option<RetrievalExplain>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pre_rerank_retrieval_score: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reranker_score: Option<f32>,
    exact_match: bool,
    resource_type_multiplier: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    staging_deboost_factor: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    missing_nova_multiplier: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    candidate_false_multiplier: Option<f32>,
    measure_match: bool,
    metric_match: bool,
    synonym_match: bool,
    canonical_entity: bool,
    canonical_semantic_match: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    engineer_exact_match_multiplier: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    canonical_entity_multiplier: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    measure_match_multiplier: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metric_match_multiplier: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    synonym_match_multiplier: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    strongest_match_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    semantic_match_multiplier: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    query_coverage_multiplier: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    semantic_canonical_match_multiplier: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    semantic_canonical_match_bonus: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    canonical_match_multiplier: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    canonical_match_bonus: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    analyst_semantic_multiplier: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    semantic_label_precision_factor: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata_support_factor: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    indicator_parent_factor: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    docs_multiplier: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tests_multiplier: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tags_multiplier: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    path_multiplier: Option<f32>,
    final_score: f32,
}

#[derive(Debug, Clone, Serialize)]
struct IndicatorScoreExplain {
    match_base: f32,
    query_coverage: f32,
    base_match_score: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    generic_label_bonus: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_synonym_bonus: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata_support_bonus: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    time_field_bonus: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dimension_bonus: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_coherence_bonus: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rrf_bonus: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reranker_bonus: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    retrieval: Option<RetrievalExplain>,
    final_score: f32,
}

#[derive(Debug, Clone, Serialize)]
struct SearchExplainConfigSnapshot {
    rrf_k: f32,
    rerank_top_n: usize,
    persona_weights: PersonaWeights,
    indicator_ranking: IndicatorRankingConfig,
    metadata_support: MetadataSupportConfig,
}

#[derive(Debug, Clone, Serialize)]
#[allow(clippy::struct_excessive_bools)]
struct SearchExplainPayload {
    query_tokens: Vec<String>,
    query_has_syntax: bool,
    rrf_enabled: bool,
    reranker_enabled: bool,
    reranker_applied: bool,
    retrievers_used: Vec<String>,
    config: SearchExplainConfigSnapshot,
}

impl MetadataSupportSignals {
    fn surface_count(&self) -> usize {
        usize::from(!self.parent_synonyms.is_empty())
            + usize::from(!self.domains.is_empty())
            + usize::from(!self.use_cases.is_empty())
            + usize::from(!self.dimensions.is_empty())
            + usize::from(!self.column_names.is_empty())
            + usize::from(!self.column_roles.is_empty())
            + usize::from(!self.column_semantic_types.is_empty())
            + usize::from(!self.example_values.is_empty())
    }

    fn merge_from(&mut self, other: &Self, max_values_per_field: usize) {
        merge_signal_values(
            &mut self.parent_synonyms,
            &other.parent_synonyms,
            max_values_per_field,
        );
        merge_signal_values(&mut self.domains, &other.domains, max_values_per_field);
        merge_signal_values(&mut self.use_cases, &other.use_cases, max_values_per_field);
        merge_signal_values(
            &mut self.dimensions,
            &other.dimensions,
            max_values_per_field,
        );
        merge_signal_values(
            &mut self.column_names,
            &other.column_names,
            max_values_per_field,
        );
        merge_signal_values(
            &mut self.column_roles,
            &other.column_roles,
            max_values_per_field,
        );
        merge_signal_values(
            &mut self.column_semantic_types,
            &other.column_semantic_types,
            max_values_per_field,
        );
        merge_signal_values(
            &mut self.example_values,
            &other.example_values,
            max_values_per_field,
        );
    }
}

#[derive(Clone)]
struct SearchCandidate<'a> {
    unique_id: String,
    entity: Option<&'a ArchivedEntity>,
    score: f32,
    support_signals: Option<MetadataSupportSignals>,
    indicator_parent_score: Option<f32>,
    explain: Option<SearchScoreExplain>,
}

struct SearchScoreOutcome {
    score: f32,
    explain: Option<SearchScoreExplain>,
}

type PreparedIndicatorSearch = (
    Vec<String>,
    bool,
    Option<HashSet<String>>,
    Option<HashSet<String>>,
    SearchPersona,
);

#[derive(Clone, Copy)]
struct SearchScoreContext<'a> {
    token_set: &'a HashSet<&'a str>,
    min_word_len: usize,
    persona: SearchPersona,
    query_text: &'a str,
    support_signals: Option<&'a MetadataSupportSignals>,
    has_indicator_parent_scores: bool,
    indicator_parent_score: Option<f32>,
}

struct IndicatorSearchContext<'a> {
    unique_id: &'a str,
    entity: &'a ArchivedEntity,
    nova: &'a ArchivedNovaMeta,
    token_set: &'a HashSet<&'a str>,
    query_token_count: usize,
    min_word_len: usize,
    support_signals: Option<MetadataSupportSignals>,
    indicator_config: &'a IndicatorRankingConfig,
    metadata_config: &'a MetadataSupportConfig,
}

#[derive(Clone, Copy)]
struct ColumnSearchMatch<'a> {
    match_type: &'static str,
    matched_value: Option<&'a str>,
    score: f32,
}

struct FusedHitBundle {
    hits: Vec<(String, f32)>,
    indicator_parent_scores: HashMap<String, f32>,
    retrieval_explain: HashMap<String, RetrievalExplain>,
    retrievers_used: Vec<String>,
}

#[derive(Default)]
struct ParentIndicatorCoherence {
    distinct_indicator_names: HashSet<String>,
    canonical_indicator_count: usize,
    strong_match_count: usize,
    support_surface_count: usize,
    has_time_field: bool,
    has_dimensions: bool,
}

impl ParentIndicatorCoherence {
    fn record(&mut self, row: &IndicatorSearchRow) {
        self.distinct_indicator_names
            .insert(row.indicator_name.clone());
        if row.canonical {
            self.canonical_indicator_count += 1;
        }
        if matches!(row.match_type.as_str(), "name" | "synonym") {
            self.strong_match_count += 1;
        }
        if let Some(signals) = &row.support_signals {
            self.support_surface_count = self.support_surface_count.max(signals.surface_count());
        }
        self.has_time_field |= row.grain.time_field.is_some();
        self.has_dimensions |= !row.grain.dimensions.is_empty();
    }

    fn bonus(&self, config: &IndicatorRankingConfig) -> f32 {
        let indicator_diversity_bonus =
            self.distinct_indicator_names.len().saturating_sub(1).min(3);
        let canonical_indicator_count = self.canonical_indicator_count.min(2);
        let strong_match_count = self.strong_match_count.min(3);
        let support_surface_count = self.support_surface_count.min(5);
        let grain_bonus = bool_to_f32(self.has_time_field)
            * config.parent_coherence_time_field_bonus
            + bool_to_f32(self.has_dimensions) * config.parent_coherence_dimension_bonus;

        (usize_to_f32(indicator_diversity_bonus)
            * config.parent_coherence_indicator_diversity_bonus
            + usize_to_f32(canonical_indicator_count)
                * config.parent_coherence_canonical_indicator_bonus
            + usize_to_f32(strong_match_count) * config.parent_coherence_strong_match_bonus
            + usize_to_f32(support_surface_count) * config.parent_coherence_support_surface_bonus
            + grain_bonus)
            .min(config.parent_coherence_max_bonus)
    }
}

fn build_measure_indicator_row(
    context: &IndicatorSearchContext<'_>,
    measure: &ArchivedNovaMeasure,
    matched: &crate::manifest::search::SemanticPreviewItem,
) -> IndicatorSearchRow {
    let explain = indicator_match_explain(context, measure.name.as_str(), matched);
    IndicatorSearchRow {
        indicator_name: measure.name.as_str().to_string(),
        indicator_type: "measure".to_string(),
        canonical: matched.canonical,
        match_type: matched.match_type.as_str().to_string(),
        score: explain.final_score,
        description: measure
            .description
            .as_ref()
            .map(rkyv::string::ArchivedString::as_str)
            .map(str::to_string),
        expression: measure
            .expression
            .as_ref()
            .map(rkyv::string::ArchivedString::as_str)
            .map(str::to_string),
        field: measure
            .field
            .as_ref()
            .map(rkyv::string::ArchivedString::as_str)
            .map(str::to_string),
        parent_unique_id: context.unique_id.to_string(),
        parent_name: context
            .entity
            .name_str()
            .unwrap_or(context.unique_id)
            .to_string(),
        parent_resource_type: context
            .entity
            .resource_type_str()
            .unwrap_or("unknown")
            .to_string(),
        relation_name: context.entity.relation_name_str().map(str::to_string),
        domains: context
            .nova
            .domains
            .iter()
            .map(|value| value.as_str().to_string())
            .collect(),
        grain: grain_summary(context.nova.grain.as_ref()),
        support_signals: context.support_signals.clone(),
        explain: Some(explain),
    }
}

fn build_measure_inventory_row(
    unique_id: &str,
    entity: &ArchivedEntity,
    nova: &ArchivedNovaMeta,
    measure: &ArchivedNovaMeasure,
    canonical: bool,
) -> IndicatorInventoryRow {
    IndicatorInventoryRow {
        indicator_name: measure.name.as_str().to_string(),
        indicator_type: "measure".to_string(),
        canonical,
        synonyms: measure
            .synonyms
            .iter()
            .map(|value| value.as_str().to_string())
            .collect(),
        description: measure
            .description
            .as_ref()
            .map(rkyv::string::ArchivedString::as_str)
            .map(str::to_string),
        expression: measure
            .expression
            .as_ref()
            .map(rkyv::string::ArchivedString::as_str)
            .map(str::to_string),
        field: measure
            .field
            .as_ref()
            .map(rkyv::string::ArchivedString::as_str)
            .map(str::to_string),
        measure_type: measure
            .measure_type
            .as_ref()
            .map(rkyv::string::ArchivedString::as_str)
            .map(str::to_string),
        template: None,
        parent_unique_id: unique_id.to_string(),
        parent_name: entity.name_str().unwrap_or(unique_id).to_string(),
        parent_resource_type: entity.resource_type_str().unwrap_or("unknown").to_string(),
        relation_name: entity.relation_name_str().map(str::to_string),
        domains: nova
            .domains
            .iter()
            .map(|value| value.as_str().to_string())
            .collect(),
        grain: grain_summary(nova.grain.as_ref()),
    }
}

fn build_metric_indicator_row(
    context: &IndicatorSearchContext<'_>,
    metric: &ArchivedNovaMetric,
    matched: &crate::manifest::search::SemanticPreviewItem,
) -> IndicatorSearchRow {
    let preferred_grain = metric.grain.as_ref().or(context.nova.grain.as_ref());
    let explain =
        indicator_match_explain_with_grain(context, metric.name.as_str(), matched, preferred_grain);
    IndicatorSearchRow {
        indicator_name: metric.name.as_str().to_string(),
        indicator_type: "metric".to_string(),
        canonical: matched.canonical,
        match_type: matched.match_type.as_str().to_string(),
        score: explain.final_score,
        description: metric
            .description
            .as_ref()
            .map(rkyv::string::ArchivedString::as_str)
            .map(str::to_string),
        expression: metric
            .expression
            .as_ref()
            .map(rkyv::string::ArchivedString::as_str)
            .map(str::to_string),
        field: None,
        parent_unique_id: context.unique_id.to_string(),
        parent_name: context
            .entity
            .name_str()
            .unwrap_or(context.unique_id)
            .to_string(),
        parent_resource_type: context
            .entity
            .resource_type_str()
            .unwrap_or("unknown")
            .to_string(),
        relation_name: context.entity.relation_name_str().map(str::to_string),
        domains: context
            .nova
            .domains
            .iter()
            .map(|value| value.as_str().to_string())
            .collect(),
        grain: grain_summary(preferred_grain),
        support_signals: context.support_signals.clone(),
        explain: Some(explain),
    }
}

fn build_metric_inventory_row(
    unique_id: &str,
    entity: &ArchivedEntity,
    nova: &ArchivedNovaMeta,
    metric: &ArchivedNovaMetric,
    canonical: bool,
) -> IndicatorInventoryRow {
    IndicatorInventoryRow {
        indicator_name: metric.name.as_str().to_string(),
        indicator_type: "metric".to_string(),
        canonical,
        synonyms: metric
            .synonyms
            .iter()
            .map(|value| value.as_str().to_string())
            .collect(),
        description: metric
            .description
            .as_ref()
            .map(rkyv::string::ArchivedString::as_str)
            .map(str::to_string),
        expression: metric
            .expression
            .as_ref()
            .map(rkyv::string::ArchivedString::as_str)
            .map(str::to_string),
        field: None,
        measure_type: None,
        template: Some(metric.template),
        parent_unique_id: unique_id.to_string(),
        parent_name: entity.name_str().unwrap_or(unique_id).to_string(),
        parent_resource_type: entity.resource_type_str().unwrap_or("unknown").to_string(),
        relation_name: entity.relation_name_str().map(str::to_string),
        domains: nova
            .domains
            .iter()
            .map(|value| value.as_str().to_string())
            .collect(),
        grain: grain_summary(metric.grain.as_ref().or(nova.grain.as_ref())),
    }
}

fn inventory_indicator_is_canonical(entity_canonical: bool, indicator_canonical: bool) -> bool {
    entity_canonical || indicator_canonical
}

fn build_column_inventory_row(
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
fn build_column_search_row(
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

fn grain_summary(grain: Option<&ArchivedNovaGrain>) -> IndicatorGrainSummary {
    let Some(grain) = grain else {
        return IndicatorGrainSummary {
            primary_key: Vec::new(),
            time_field: None,
            dimensions: Vec::new(),
        };
    };
    IndicatorGrainSummary {
        primary_key: grain
            .primary_key
            .iter()
            .map(|value| value.as_str().to_string())
            .collect(),
        time_field: grain
            .time_field
            .as_ref()
            .map(rkyv::string::ArchivedString::as_str)
            .map(str::to_string),
        dimensions: grain
            .dimensions
            .iter()
            .map(|value| value.as_str().to_string())
            .collect(),
    }
}

fn indicator_match_explain(
    context: &IndicatorSearchContext<'_>,
    indicator_name: &str,
    matched: &crate::manifest::search::SemanticPreviewItem,
) -> IndicatorScoreExplain {
    indicator_match_explain_with_grain(
        context,
        indicator_name,
        matched,
        preferred_grain_for_scoring(context.nova),
    )
}

fn indicator_match_explain_with_grain(
    context: &IndicatorSearchContext<'_>,
    indicator_name: &str,
    matched: &crate::manifest::search::SemanticPreviewItem,
    preferred_grain: Option<&ArchivedNovaGrain>,
) -> IndicatorScoreExplain {
    let coverage = if context.query_token_count == 0 {
        0.0
    } else {
        usize_to_f32(matched.matched_token_count) / usize_to_f32(context.query_token_count)
    };
    let match_base = match matched.match_type {
        SemanticMatchType::Name => 5.0,
        SemanticMatchType::Synonym => 4.0,
        SemanticMatchType::Field => 3.0,
        SemanticMatchType::Description => 2.0,
        SemanticMatchType::Expression => 1.0,
    };
    let mut score = match_base + (coverage * 4.0);
    let generic_label_bonus = generic_indicator_label_match_bonus(
        indicator_name,
        context.token_set,
        context.min_word_len,
        context.indicator_config,
    );
    let parent_synonym_bonus = parent_synonym_match_bonus(
        context.nova,
        context.token_set,
        context.min_word_len,
        context.indicator_config,
    );
    let metadata_support_bonus =
        metadata_support_bonus(context.support_signals.as_ref(), context.metadata_config);
    let time_field_bonus = preferred_grain
        .and_then(|grain| {
            grain
                .time_field
                .as_ref()
                .map(rkyv::string::ArchivedString::as_str)
                .map(str::trim)
                .is_some_and(|value| !value.is_empty())
                .then_some(0.5)
        })
        .unwrap_or_default();
    let dimension_bonus = if preferred_grain.is_some_and(|grain| !grain.dimensions.is_empty()) {
        0.25
    } else {
        0.0
    };
    if matched.canonical {
        score += 1.5;
    }
    if context.nova.canonical {
        score += 0.75;
    }
    score += generic_label_bonus;
    score += parent_synonym_bonus;
    score += metadata_support_bonus;
    score += time_field_bonus;
    score += dimension_bonus;
    if context
        .entity
        .description_str()
        .is_some_and(|value| !value.trim().is_empty())
    {
        score += 0.1;
    }
    IndicatorScoreExplain {
        match_base,
        query_coverage: coverage,
        base_match_score: score,
        generic_label_bonus: non_zero_option(generic_label_bonus),
        parent_synonym_bonus: non_zero_option(parent_synonym_bonus),
        metadata_support_bonus: non_zero_option(metadata_support_bonus),
        time_field_bonus: non_zero_option(time_field_bonus),
        dimension_bonus: non_zero_option(dimension_bonus),
        parent_coherence_bonus: None,
        rrf_bonus: None,
        reranker_bonus: None,
        retrieval: None,
        final_score: score,
    }
}

fn generic_indicator_label_match_bonus(
    indicator_name: &str,
    query_token_set: &HashSet<&str>,
    min_word_len: usize,
    config: &IndicatorRankingConfig,
) -> f32 {
    match fully_covered_token_count(indicator_name, query_token_set, min_word_len) {
        Some(1) => config.generic_label_bonus_one_token,
        Some(2) => config.generic_label_bonus_two_tokens,
        Some(_) => config.generic_label_bonus_three_plus_tokens,
        None => 0.0,
    }
}

fn parent_synonym_match_bonus(
    nova: &ArchivedNovaMeta,
    query_token_set: &HashSet<&str>,
    min_word_len: usize,
    config: &IndicatorRankingConfig,
) -> f32 {
    nova.synonyms
        .iter()
        .filter_map(|value| {
            fully_covered_token_count(value.as_str(), query_token_set, min_word_len)
        })
        .max()
        .map_or(0.0, |token_count| match token_count {
            1 => config.parent_synonym_bonus_one_token,
            2 => config.parent_synonym_bonus_two_tokens,
            _ => config.parent_synonym_bonus_three_plus_tokens,
        })
}

fn fully_covered_token_count(
    value: &str,
    query_token_set: &HashSet<&str>,
    min_word_len: usize,
) -> Option<usize> {
    let tokens = tokenize_alnum_lowercase(value, min_word_len);
    (!tokens.is_empty()
        && tokens
            .iter()
            .all(|token| query_token_set.contains(token.as_str())))
    .then_some(tokens.len())
}

fn metadata_support_bonus(
    signals: Option<&MetadataSupportSignals>,
    config: &MetadataSupportConfig,
) -> f32 {
    signals.map_or(0.0, |signals| {
        let parent_synonym_bonus =
            usize_to_f32(signals.parent_synonyms.len()) * config.parent_synonym_weight;
        let domain_bonus = usize_to_f32(signals.domains.len()) * config.domain_weight;
        let use_case_bonus = usize_to_f32(signals.use_cases.len()) * config.use_case_weight;
        let dimension_bonus = usize_to_f32(signals.dimensions.len()) * config.dimension_weight;
        let column_name_bonus =
            usize_to_f32(signals.column_names.len()) * config.column_name_weight;
        let column_role_bonus =
            usize_to_f32(signals.column_roles.len()) * config.column_role_weight;
        let semantic_type_bonus =
            usize_to_f32(signals.column_semantic_types.len()) * config.semantic_type_weight;
        let example_value_bonus =
            usize_to_f32(signals.example_values.len()) * config.example_value_weight;
        (parent_synonym_bonus
            + domain_bonus
            + use_case_bonus
            + dimension_bonus
            + column_name_bonus
            + column_role_bonus
            + semantic_type_bonus
            + example_value_bonus)
            .min(config.max_bonus)
    })
}

fn semantic_label_precision_bonus(
    semantic_matches: &NovaSemanticMatches,
    query_token_set: &HashSet<&str>,
    min_word_len: usize,
    config: &IndicatorRankingConfig,
) -> f32 {
    semantic_matches
        .measures
        .iter()
        .chain(semantic_matches.metrics.iter())
        .map(|item| {
            let canonical_bonus = if item.canonical {
                config.semantic_label_precision_canonical_bonus
            } else {
                0.0
            };
            (generic_indicator_label_match_bonus(
                item.name.as_str(),
                query_token_set,
                min_word_len,
                config,
            ) * config.semantic_label_precision_scale)
                + canonical_bonus
        })
        .fold(0.0, f32::max)
}

fn apply_parent_coherence_bonus(
    mut rows: Vec<IndicatorSearchRow>,
    config: &IndicatorRankingConfig,
) -> Vec<IndicatorSearchRow> {
    let mut coherence_by_parent: HashMap<String, ParentIndicatorCoherence> = HashMap::new();
    for row in &rows {
        coherence_by_parent
            .entry(row.parent_unique_id.clone())
            .or_default()
            .record(row);
    }

    for row in &mut rows {
        if let Some(coherence) = coherence_by_parent.get(&row.parent_unique_id) {
            let bonus = coherence.bonus(config);
            row.score += bonus;
            if let Some(explain) = &mut row.explain {
                explain.parent_coherence_bonus = non_zero_option(bonus);
                explain.final_score = row.score;
            }
        }
    }

    rows.sort_by(compare_indicator_rows);
    rows
}

fn token_overlap_count(
    value: &str,
    query_token_set: &HashSet<&str>,
    min_word_len: usize,
) -> Option<usize> {
    let tokens = tokenize_alnum_lowercase(value, min_word_len);
    let count = tokens
        .iter()
        .filter(|token| query_token_set.contains(token.as_str()))
        .count();
    (count > 0).then_some(count)
}

fn collect_metadata_support_signals(
    entity: &ArchivedEntity,
    nova: &ArchivedNovaMeta,
    query_token_set: &HashSet<&str>,
    min_word_len: usize,
    config: &MetadataSupportConfig,
) -> Option<MetadataSupportSignals> {
    let matched_parent_synonyms = collect_matching_values(
        nova.synonyms.iter().map(ArchivedString::as_str),
        query_token_set,
        min_word_len,
        config.max_values_per_field,
    );
    let matched_domains = collect_matching_values(
        nova.domains.iter().map(ArchivedString::as_str),
        query_token_set,
        min_word_len,
        config.max_values_per_field,
    );
    let matched_use_cases = collect_matching_values(
        nova.use_cases.iter().map(ArchivedString::as_str),
        query_token_set,
        min_word_len,
        config.max_values_per_field,
    );
    let matched_dimensions = preferred_grain_for_scoring(nova).map_or_else(Vec::new, |grain| {
        collect_matching_values(
            grain.dimensions.iter().map(ArchivedString::as_str),
            query_token_set,
            min_word_len,
            config.max_values_per_field,
        )
    });

    let mut signals = MetadataSupportSignals {
        parent_synonyms: matched_parent_synonyms,
        domains: matched_domains,
        use_cases: matched_use_cases,
        dimensions: matched_dimensions,
        ..MetadataSupportSignals::default()
    };
    for column in entity.column_meta() {
        collect_column_metadata_support_signals(
            column,
            query_token_set,
            min_word_len,
            config.max_values_per_field,
            &mut signals,
        );
    }

    (!signals.parent_synonyms.is_empty()
        || !signals.domains.is_empty()
        || !signals.use_cases.is_empty()
        || !signals.dimensions.is_empty()
        || !signals.column_names.is_empty()
        || !signals.column_roles.is_empty()
        || !signals.column_semantic_types.is_empty()
        || !signals.example_values.is_empty())
    .then_some(signals)
}

fn collect_matching_values<'a>(
    values: impl Iterator<Item = &'a str>,
    query_token_set: &HashSet<&str>,
    min_word_len: usize,
    max_values_per_field: usize,
) -> Vec<String> {
    let mut matches = Vec::new();
    for value in values {
        if token_overlap_count(value, query_token_set, min_word_len).is_some() {
            push_unique_string(&mut matches, value.to_string(), max_values_per_field);
        }
    }
    matches
}

fn collect_column_metadata_support_signals(
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

fn push_unique_string(values: &mut Vec<String>, value: String, max_values_per_field: usize) {
    if values.len() < max_values_per_field && !values.iter().any(|existing| existing == &value) {
        values.push(value);
    }
}

fn merge_signal_values(target: &mut Vec<String>, source: &[String], max_values_per_field: usize) {
    for value in source {
        push_unique_string(target, value.clone(), max_values_per_field);
    }
}

fn build_search_explain_payload(
    tokens: &[String],
    query_has_syntax: bool,
    persona: SearchPersona,
    config: &SearchConfig,
    retrievers_used: Vec<String>,
    reranker_applied: bool,
) -> SearchExplainPayload {
    SearchExplainPayload {
        query_tokens: tokens.to_vec(),
        query_has_syntax,
        rrf_enabled: config.enable_rrf,
        reranker_enabled: config.enable_reranker,
        reranker_applied,
        retrievers_used,
        config: SearchExplainConfigSnapshot {
            rrf_k: config.rrf_k,
            rerank_top_n: config.rerank_top_n,
            persona_weights: persona_weights(persona, config),
            indicator_ranking: config.indicator_ranking,
            metadata_support: config.metadata_support,
        },
    }
}

fn indicator_retrievers_used(rows: &[IndicatorSearchRow], rrf_enabled: bool) -> Vec<String> {
    if rows.is_empty() {
        return Vec::new();
    }
    if !rrf_enabled {
        return vec!["indicator_local".to_string()];
    }
    rows.iter()
        .find_map(|row| {
            row.explain
                .as_ref()
                .and_then(|explain| explain.retrieval.as_ref())
        })
        .map_or_else(
            || vec!["indicator_local".to_string()],
            |retrieval| retrieval.retrievers.keys().cloned().collect(),
        )
}

fn strip_sql_fields(value: &mut JsonValue) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };
    obj.remove("raw_code");
    obj.remove("compiled_code");
}

struct OwnedSearchRequest {
    query_text: String,
    resource_types: Vec<String>,
    limit: usize,
    min_score: Option<f32>,
    fuzzy: bool,
    include_highlights: bool,
    include_ngram_override: Option<bool>,
    scope: SearchScope,
    persona: SearchPersona,
}

async fn run_tantivy_search(
    tantivy: TantivySearcher,
    config: crate::config::SearchConfig,
    request: OwnedSearchRequest,
) -> Result<Vec<SearchHit>> {
    tokio::task::spawn_blocking(move || {
        let search_request = SearchRequest {
            query_text: &request.query_text,
            resource_types: &request.resource_types,
            limit: request.limit,
            min_score: request.min_score,
            fuzzy: request.fuzzy,
            include_highlights: request.include_highlights,
            include_ngram_override: request.include_ngram_override,
            scope: request.scope,
            persona: request.persona,
        };
        tantivy.search(&search_request, &config)
    })
    .await
    .map_err(|e| DbtNovaError::ServerError(e.to_string()))?
}

fn tokens_match(value: &str, token_set: &HashSet<&str>, min_word_len: usize) -> bool {
    if value.is_empty() || token_set.is_empty() {
        return false;
    }
    tokenize_alnum_lowercase(value, min_word_len)
        .iter()
        .any(|t| token_set.contains(t.as_str()))
}

fn analyst_semantic_multiplier(
    nova: &ArchivedNovaMeta,
    token_set: &HashSet<&str>,
    min_word_len: usize,
    config: &AnalystSemanticConfig,
) -> f32 {
    let preferred_grain = preferred_grain_for_scoring(nova);
    let has_metric_definition = nova.metric.iter().chain(nova.metrics.iter()).any(|metric| {
        metric
            .description
            .as_ref()
            .map(rkyv::string::ArchivedString::as_str)
            .map(str::trim)
            .is_some_and(|s| !s.is_empty())
            || metric
                .expression
                .as_ref()
                .map(rkyv::string::ArchivedString::as_str)
                .map(str::trim)
                .is_some_and(|s| !s.is_empty())
    });
    let has_measure_definition = nova.measures.iter().any(|measure| {
        measure
            .description
            .as_ref()
            .map(rkyv::string::ArchivedString::as_str)
            .map(str::trim)
            .is_some_and(|s| !s.is_empty())
            || measure
                .expression
                .as_ref()
                .map(rkyv::string::ArchivedString::as_str)
                .map(str::trim)
                .is_some_and(|s| !s.is_empty())
            || measure
                .field
                .as_ref()
                .map(rkyv::string::ArchivedString::as_str)
                .map(str::trim)
                .is_some_and(|s| !s.is_empty())
    });
    let has_grain = preferred_grain.is_some_and(|grain| {
        !grain.primary_key.is_empty()
            || !grain.dimensions.is_empty()
            || grain
                .time_field
                .as_ref()
                .map(rkyv::string::ArchivedString::as_str)
                .map(str::trim)
                .is_some_and(|s| !s.is_empty())
    });
    let has_time_field = preferred_grain
        .and_then(|grain| {
            grain
                .time_field
                .as_ref()
                .map(rkyv::string::ArchivedString::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
        })
        .is_some();
    let dimension_overlap = if token_set.is_empty() {
        0usize
    } else {
        preferred_grain.map_or(0usize, |grain| {
            grain
                .dimensions
                .iter()
                .filter(|dimension| tokens_match(dimension.as_str(), token_set, min_word_len))
                .count()
        })
    };

    let mut multiplier = 1.0f32;
    if has_metric_definition {
        multiplier *= config.metric_definition_multiplier;
    }
    if has_measure_definition {
        multiplier *= config.measure_definition_multiplier;
    }
    if has_grain {
        multiplier *= config.grain_multiplier;
    }
    if has_time_field {
        multiplier *= config.time_field_multiplier;
    }
    if dimension_overlap > 0 {
        let overlap_bonus = match dimension_overlap.min(3) {
            0 => 1.0,
            1 => config.dimension_overlap_one_multiplier,
            2 => config.dimension_overlap_two_multiplier,
            _ => config.dimension_overlap_three_plus_multiplier,
        };
        multiplier *= overlap_bonus;
    }
    if !has_metric_definition && !has_measure_definition {
        multiplier *= config.missing_metric_or_measure_multiplier;
    }
    if !has_grain {
        multiplier *= config.missing_grain_multiplier;
    }

    multiplier.clamp(config.min_multiplier, config.max_multiplier)
}

fn analyst_query_coverage_multiplier(best_query_coverage: f32, query_token_count: usize) -> f32 {
    if query_token_count <= 1 {
        return 1.0;
    }
    if best_query_coverage >= 0.999 {
        1.35
    } else if best_query_coverage >= 0.75 {
        1.12
    } else if best_query_coverage >= 0.5 {
        0.92
    } else if best_query_coverage > 0.0 {
        0.85
    } else {
        1.0
    }
}

fn preferred_grain_for_scoring(nova: &ArchivedNovaMeta) -> Option<&ArchivedNovaGrain> {
    if let Some(grain) = nova.grain.as_ref().filter(|grain| {
        !grain.primary_key.is_empty()
            || !grain.dimensions.is_empty()
            || grain
                .time_field
                .as_ref()
                .map(rkyv::string::ArchivedString::as_str)
                .map(str::trim)
                .is_some_and(|s| !s.is_empty())
    }) {
        return Some(grain);
    }
    if let Some(grain) = nova
        .metric
        .as_ref()
        .and_then(|metric| metric.grain.as_ref())
        .filter(|grain| {
            !grain.primary_key.is_empty()
                || !grain.dimensions.is_empty()
                || grain
                    .time_field
                    .as_ref()
                    .map(rkyv::string::ArchivedString::as_str)
                    .map(str::trim)
                    .is_some_and(|s| !s.is_empty())
        })
    {
        return Some(grain);
    }
    nova.metrics
        .iter()
        .filter_map(|metric| metric.grain.as_ref())
        .find(|grain| {
            !grain.primary_key.is_empty()
                || !grain.dimensions.is_empty()
                || grain
                    .time_field
                    .as_ref()
                    .map(rkyv::string::ArchivedString::as_str)
                    .map(str::trim)
                    .is_some_and(|s| !s.is_empty())
        })
}

const ANALYST_NEAR_TIE_GAP_THRESHOLD_PCT: f32 = 8.0;

fn analyst_near_tie_hint(candidates: &[SearchCandidate<'_>]) -> Option<String> {
    if candidates.len() < 2 {
        return None;
    }

    let top_score = candidates[0].score;
    let second_score = candidates[1].score;
    let denominator = top_score.abs().max(second_score.abs()).max(f32::EPSILON);
    let gap_pct = (((top_score - second_score).abs() / denominator) * 100.0).max(0.0);
    if gap_pct > ANALYST_NEAR_TIE_GAP_THRESHOLD_PCT {
        return None;
    }

    let first = candidate_display_name(&candidates[0]);
    let second = candidate_display_name(&candidates[1]);
    Some(format!(
        "Top candidates `{first}` and `{second}` are close ({gap_pct:.1}% score gap). Use `get_entity` or `get_context` to verify metric definition, grain, and date/country dimensions before final SQL."
    ))
}

fn candidate_display_name(candidate: &SearchCandidate<'_>) -> String {
    if let Some(entity) = candidate.entity
        && let Some(name) = entity.name_str()
    {
        return name.to_string();
    }
    candidate.unique_id.clone()
}

fn bool_to_f32(value: bool) -> f32 {
    if value { 1.0 } else { 0.0 }
}

fn is_neutral_multiplier(value: f32) -> bool {
    (value - 1.0).abs() < f32::EPSILON
}

fn usize_to_f32(value: usize) -> f32 {
    f32::from(u16::try_from(value).unwrap_or(u16::MAX))
}

fn non_zero_option(value: f32) -> Option<f32> {
    (value.abs() > f32::EPSILON).then_some(value)
}

fn non_neutral_option(value: f32) -> Option<f32> {
    ((value - 1.0).abs() > f32::EPSILON).then_some(value)
}

fn compare_scores_desc(left: f32, right: f32) -> Ordering {
    right.partial_cmp(&left).unwrap_or(Ordering::Equal)
}

fn compare_indicator_rows(left: &IndicatorSearchRow, right: &IndicatorSearchRow) -> Ordering {
    compare_scores_desc(left.score, right.score)
        .then_with(|| left.parent_unique_id.cmp(&right.parent_unique_id))
        .then_with(|| left.indicator_type.cmp(&right.indicator_type))
        .then_with(|| left.indicator_name.cmp(&right.indicator_name))
}

fn compare_indicator_inventory_rows(
    left: &IndicatorInventoryRow,
    right: &IndicatorInventoryRow,
) -> Ordering {
    right
        .canonical
        .cmp(&left.canonical)
        .then_with(|| left.parent_unique_id.cmp(&right.parent_unique_id))
        .then_with(|| left.indicator_type.cmp(&right.indicator_type))
        .then_with(|| left.indicator_name.cmp(&right.indicator_name))
}

fn compare_column_inventory_rows(
    left: &ColumnInventoryRow,
    right: &ColumnInventoryRow,
) -> Ordering {
    left.parent_unique_id
        .cmp(&right.parent_unique_id)
        .then_with(|| left.column_name.cmp(&right.column_name))
}

fn compare_column_search_rows(left: &ColumnSearchRow, right: &ColumnSearchRow) -> Ordering {
    compare_scores_desc(left.score, right.score)
        .then_with(|| left.parent_unique_id.cmp(&right.parent_unique_id))
        .then_with(|| left.column_name.cmp(&right.column_name))
        .then_with(|| left.match_type.cmp(&right.match_type))
}

fn compare_search_candidates(left: &SearchCandidate<'_>, right: &SearchCandidate<'_>) -> Ordering {
    compare_scores_desc(left.score, right.score)
        .then_with(|| {
            compare_scores_desc(
                left.indicator_parent_score.unwrap_or_default(),
                right.indicator_parent_score.unwrap_or_default(),
            )
        })
        .then_with(|| left.unique_id.cmp(&right.unique_id))
}

fn entity_column_meta_lookup(entity: &ArchivedEntity) -> HashMap<&str, &ArchivedColumnMetaSummary> {
    entity
        .column_meta()
        .iter()
        .map(|summary| (summary.name.as_str(), summary))
        .collect()
}

fn column_is_annotated(summary: Option<&ArchivedColumnMetaSummary>) -> bool {
    summary.is_some_and(|summary| {
        summary.role.is_some()
            || summary.semantic_type.is_some()
            || !summary.synonyms.is_empty()
            || !summary.example_values.is_empty()
    })
}

fn column_matches_filters(
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

fn best_column_search_match<'a>(
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

fn column_parent_context_bonus(
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

fn normalized_resource_type_filter(resource_types: &[String]) -> Option<HashSet<String>> {
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

fn normalized_value_filter(values: &[String]) -> Option<HashSet<String>> {
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

fn normalized_indicator_type_filter(indicator_types: &[String]) -> Result<Option<HashSet<String>>> {
    if indicator_types.is_empty() {
        return Ok(None);
    }

    let mut normalized = HashSet::new();
    for value in indicator_types {
        let candidate = value.trim().to_ascii_lowercase();
        if candidate.is_empty() {
            continue;
        }
        if !matches!(candidate.as_str(), "metric" | "measure") {
            return Err(DbtNovaError::InvalidParams(format!(
                "Unsupported indicator type '{value}'. Expected one of: metric, measure"
            )));
        }
        normalized.insert(candidate);
    }

    if normalized.is_empty() {
        Ok(None)
    } else {
        Ok(Some(normalized))
    }
}

fn indicator_type_selected(
    indicator_filter: Option<&HashSet<String>>,
    indicator_type: &str,
) -> bool {
    indicator_filter.is_none_or(|values| values.contains(indicator_type))
}

fn dedupe_indicator_parent_ids(rows: &[IndicatorSearchRow], limit: usize) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut ids = Vec::new();
    for row in rows {
        if seen.insert(row.parent_unique_id.clone()) {
            ids.push(row.parent_unique_id.clone());
            if ids.len() >= limit {
                break;
            }
        }
    }
    ids
}

fn normalized_indicator_parent_scores(
    rows: &[IndicatorSearchRow],
    config: &IndicatorRankingConfig,
) -> HashMap<String, f32> {
    let top_k = config.search_parent_indicator_top_k.max(1);
    let mut by_parent: HashMap<String, (f32, usize)> = HashMap::new();
    for row in rows {
        let entry = by_parent
            .entry(row.parent_unique_id.clone())
            .or_insert((0.0, 0));
        if entry.1 < top_k {
            entry.0 += row.score;
            entry.1 += 1;
        }
    }
    let max_score = by_parent
        .values()
        .map(|(score, _)| *score)
        .fold(0.0f32, f32::max);
    if max_score <= 0.0 {
        return HashMap::new();
    }
    by_parent
        .into_iter()
        .map(|(parent_unique_id, (score, _))| {
            (parent_unique_id, (score / max_score).clamp(0.0, 1.0))
        })
        .collect()
}

fn resource_type_allowed_for_search(
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

fn query_exact_match(query: &str, unique_id: &str, entity: &ArchivedEntity) -> bool {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return false;
    }
    let name = entity.name_str().unwrap_or("");
    let alias = entity.alias_str().unwrap_or("");
    let mut candidates = Vec::new();
    candidates.push(name.to_lowercase());
    candidates.push(alias.to_lowercase());
    candidates.push(unique_id.to_lowercase());
    if let Some(last) = unique_id.split('.').next_back() {
        candidates.push(last.to_lowercase());
    }
    if let Some(path) = entity.original_file_path_str() {
        candidates.push(path.to_lowercase());
        if let Some(file) = path.split('/').next_back() {
            candidates.push(file.to_lowercase());
        }
    }

    candidates.iter().any(|c| c == &q)
}

fn candidate_flag_for_persona(nova: &ArchivedNovaMeta, persona: SearchPersona) -> Option<bool> {
    let candidates = nova.search.as_ref()?.candidates.as_ref()?;
    Some(match persona {
        SearchPersona::Analyst => candidates.analyst,
        SearchPersona::Engineer => candidates.engineer,
        SearchPersona::Governance => candidates.governance,
        SearchPersona::Default => true,
    })
}

fn candidate_false_multiplier(
    persona: SearchPersona,
    persona_candidate: Option<bool>,
    exact_match: bool,
    config: &SearchConfig,
) -> f32 {
    if exact_match || persona_candidate.unwrap_or(true) {
        return 1.0;
    }

    match persona {
        SearchPersona::Analyst => config.analyst_candidate_false_deboost_factor,
        SearchPersona::Engineer => config.engineer_candidate_false_deboost_factor,
        SearchPersona::Governance => config.governance_candidate_false_deboost_factor,
        SearchPersona::Default => 1.0,
    }
}

fn suggestion_allowed(persona: SearchPersona, resource_type: Option<&str>) -> bool {
    let rt = resource_type.unwrap_or("").to_lowercase();
    match persona {
        SearchPersona::Analyst => !matches!(rt.as_str(), "test" | "macro"),
        SearchPersona::Governance => !matches!(rt.as_str(), "macro"),
        SearchPersona::Engineer | SearchPersona::Default => true,
    }
}

fn persona_resource_type_multiplier(persona: SearchPersona, resource_type: &str) -> f32 {
    let rt = resource_type.trim().to_lowercase();
    match persona {
        SearchPersona::Analyst => match rt.as_str() {
            "semantic_model" | "saved_query" | "model" => 1.25,
            "metric" | "exposure" => 1.1,
            "analysis" => 1.15,
            "source" => 1.05,
            "test" => 0.65,
            "macro" => 0.45,
            _ => 1.0,
        },
        SearchPersona::Engineer => match rt.as_str() {
            "model" => 1.3,
            "source" => 1.2,
            "test" => 1.15,
            "macro" => 1.1,
            "seed" | "snapshot" => 1.05,
            "analysis" => 0.85,
            "metric" | "semantic_model" | "saved_query" => 0.9,
            _ => 1.0,
        },
        SearchPersona::Governance => match rt.as_str() {
            "test" => 1.35,
            "model" | "source" => 1.2,
            "exposure" => 1.15,
            "metric" | "semantic_model" | "saved_query" => 1.05,
            "macro" => 0.55,
            _ => 1.0,
        },
        SearchPersona::Default => 1.0,
    }
}

fn persona_semantic_match_multiplier(persona: SearchPersona, config: &SearchConfig) -> f32 {
    match persona {
        SearchPersona::Analyst => config.analyst_nova_semantic_match_multiplier,
        SearchPersona::Engineer | SearchPersona::Governance | SearchPersona::Default => {
            config.non_analyst_nova_semantic_match_multiplier
        }
    }
}

#[cfg(test)]
mod candidate_tests {
    use super::*;

    #[test]
    fn candidate_false_multiplier_deboosts_analyst_only_by_default() {
        let config = SearchConfig::default();

        assert!(
            (candidate_false_multiplier(SearchPersona::Analyst, Some(false), false, &config)
                - config.analyst_candidate_false_deboost_factor)
                .abs()
                < f32::EPSILON
        );
        assert!(
            (candidate_false_multiplier(SearchPersona::Engineer, Some(false), false, &config)
                - 1.0)
                .abs()
                < f32::EPSILON
        );
        assert!(
            (candidate_false_multiplier(SearchPersona::Governance, Some(false), false, &config)
                - 1.0)
                .abs()
                < f32::EPSILON
        );
    }

    #[test]
    fn candidate_false_multiplier_skips_deboost_for_exact_matches() {
        let config = SearchConfig::default();

        assert!(
            (candidate_false_multiplier(SearchPersona::Analyst, Some(false), true, &config) - 1.0)
                .abs()
                < f32::EPSILON
        );
    }

    #[test]
    fn candidate_flag_for_persona_reads_archived_candidates() {
        let nova = crate::manifest::entity::NovaMeta {
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
            metrics: Vec::new(),
            governance: None,
            search: Some(crate::manifest::entity::NovaSearchMeta {
                candidates: Some(crate::manifest::entity::NovaSearchCandidates {
                    analyst: false,
                    engineer: true,
                    governance: true,
                }),
            }),
        };
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&nova).expect("archive nova meta");
        let archived = rkyv::access::<ArchivedNovaMeta, rkyv::rancor::Error>(&bytes)
            .expect("access archived nova meta");

        assert_eq!(
            candidate_flag_for_persona(archived, SearchPersona::Analyst),
            Some(false)
        );
        assert_eq!(
            candidate_flag_for_persona(archived, SearchPersona::Engineer),
            Some(true)
        );
    }

    #[test]
    fn indicator_embedding_text_includes_indicator_and_parent_context() {
        let row = IndicatorSearchRow {
            indicator_name: "average_order_value".to_string(),
            indicator_type: "metric".to_string(),
            canonical: true,
            match_type: "name".to_string(),
            score: 1.0,
            description: Some("Average order value across completed orders".to_string()),
            expression: Some("sum(gmv_amount) / nullif(count(distinct order_id), 0)".to_string()),
            field: None,
            parent_unique_id: "model.pkg.orders_semantic_templates".to_string(),
            parent_name: "orders_semantic_templates".to_string(),
            parent_resource_type: "model".to_string(),
            relation_name: None,
            domains: vec!["commerce".to_string()],
            grain: IndicatorGrainSummary {
                primary_key: Vec::new(),
                time_field: Some("order_date".to_string()),
                dimensions: vec!["country_code".to_string(), "sales_channel".to_string()],
            },
            support_signals: None,
            explain: None,
        };

        let text = indicator_embedding_text(&row);
        assert!(text.contains("indicator_name: average_order_value"));
        assert!(text.contains("parent_name: orders_semantic_templates"));
        assert!(text.contains("time_field: order_date"));
        assert!(text.contains("domains: commerce"));
    }

    #[test]
    fn reorder_indicator_rows_with_reranker_reorders_top_n_and_preserves_tail() {
        let rows = vec![
            IndicatorSearchRow {
                indicator_name: "first".to_string(),
                indicator_type: "metric".to_string(),
                canonical: true,
                match_type: "name".to_string(),
                score: 10.0,
                description: None,
                expression: None,
                field: None,
                parent_unique_id: "model.pkg.one".to_string(),
                parent_name: "one".to_string(),
                parent_resource_type: "model".to_string(),
                relation_name: None,
                domains: Vec::new(),
                grain: IndicatorGrainSummary {
                    primary_key: Vec::new(),
                    time_field: None,
                    dimensions: Vec::new(),
                },
                support_signals: None,
                explain: None,
            },
            IndicatorSearchRow {
                indicator_name: "second".to_string(),
                indicator_type: "metric".to_string(),
                canonical: true,
                match_type: "name".to_string(),
                score: 9.0,
                description: None,
                expression: None,
                field: None,
                parent_unique_id: "model.pkg.two".to_string(),
                parent_name: "two".to_string(),
                parent_resource_type: "model".to_string(),
                relation_name: None,
                domains: Vec::new(),
                grain: IndicatorGrainSummary {
                    primary_key: Vec::new(),
                    time_field: None,
                    dimensions: Vec::new(),
                },
                support_signals: None,
                explain: None,
            },
            IndicatorSearchRow {
                indicator_name: "third".to_string(),
                indicator_type: "metric".to_string(),
                canonical: true,
                match_type: "name".to_string(),
                score: 8.0,
                description: None,
                expression: None,
                field: None,
                parent_unique_id: "model.pkg.three".to_string(),
                parent_name: "three".to_string(),
                parent_resource_type: "model".to_string(),
                relation_name: None,
                domains: Vec::new(),
                grain: IndicatorGrainSummary {
                    primary_key: Vec::new(),
                    time_field: None,
                    dimensions: Vec::new(),
                },
                support_signals: None,
                explain: None,
            },
        ];

        let reordered = reorder_indicator_rows_with_reranker(
            rows,
            2,
            &[(1, 42.0), (0, 41.0)],
            &SearchConfig::default().indicator_ranking,
        );
        assert_eq!(reordered[0].indicator_name, "second");
        assert!((reordered[0].score - 51.0).abs() < f32::EPSILON);
        assert_eq!(reordered[1].indicator_name, "first");
        assert!((reordered[1].score - 51.0).abs() < f32::EPSILON);
        assert_eq!(reordered[2].indicator_name, "third");
        assert!((reordered[2].score - 8.0).abs() < f32::EPSILON);
    }

    #[test]
    fn reorder_indicator_rows_with_reranker_preserves_strong_prior_winner_when_rerank_gap_is_small()
    {
        let rows = vec![
            IndicatorSearchRow {
                indicator_name: "gmv".to_string(),
                indicator_type: "measure".to_string(),
                canonical: true,
                match_type: "name".to_string(),
                score: 10.0,
                description: None,
                expression: None,
                field: None,
                parent_unique_id: "model.pkg.sales".to_string(),
                parent_name: "sales".to_string(),
                parent_resource_type: "model".to_string(),
                relation_name: None,
                domains: Vec::new(),
                grain: IndicatorGrainSummary {
                    primary_key: Vec::new(),
                    time_field: None,
                    dimensions: Vec::new(),
                },
                support_signals: None,
                explain: None,
            },
            IndicatorSearchRow {
                indicator_name: "gmv_cancelled".to_string(),
                indicator_type: "measure".to_string(),
                canonical: true,
                match_type: "name".to_string(),
                score: 8.5,
                description: None,
                expression: None,
                field: None,
                parent_unique_id: "model.pkg.sales".to_string(),
                parent_name: "sales".to_string(),
                parent_resource_type: "model".to_string(),
                relation_name: None,
                domains: Vec::new(),
                grain: IndicatorGrainSummary {
                    primary_key: Vec::new(),
                    time_field: None,
                    dimensions: Vec::new(),
                },
                support_signals: None,
                explain: None,
            },
        ];

        let reordered = reorder_indicator_rows_with_reranker(
            rows,
            2,
            &[(1, 1.0), (0, 0.8)],
            &SearchConfig::default().indicator_ranking,
        );
        assert_eq!(reordered[0].indicator_name, "gmv");
        assert!((reordered[0].score - 10.8).abs() < f32::EPSILON);
        assert_eq!(reordered[1].indicator_name, "gmv_cancelled");
        assert!((reordered[1].score - 9.5).abs() < f32::EPSILON);
    }

    #[test]
    fn apply_parent_coherence_bonus_prefers_parent_covering_more_of_question() {
        let rows = vec![
            IndicatorSearchRow {
                indicator_name: "gmv".to_string(),
                indicator_type: "measure".to_string(),
                canonical: true,
                match_type: "name".to_string(),
                score: 8.7,
                description: None,
                expression: None,
                field: None,
                parent_unique_id: "model.pkg.fact_orders".to_string(),
                parent_name: "fact_orders".to_string(),
                parent_resource_type: "model".to_string(),
                relation_name: None,
                domains: vec!["commerce".to_string()],
                grain: IndicatorGrainSummary {
                    primary_key: vec!["order_id".to_string()],
                    time_field: Some("order_date".to_string()),
                    dimensions: vec!["country_code".to_string()],
                },
                support_signals: Some(MetadataSupportSignals {
                    parent_synonyms: vec!["gmv".to_string()],
                    domains: vec![],
                    use_cases: vec![],
                    dimensions: vec!["country_code".to_string()],
                    column_names: vec![],
                    column_roles: vec![],
                    column_semantic_types: vec![],
                    example_values: vec!["alpha".to_string()],
                }),
                explain: None,
            },
            IndicatorSearchRow {
                indicator_name: "net_sales".to_string(),
                indicator_type: "measure".to_string(),
                canonical: true,
                match_type: "name".to_string(),
                score: 8.1,
                description: None,
                expression: None,
                field: None,
                parent_unique_id: "model.pkg.fact_orders".to_string(),
                parent_name: "fact_orders".to_string(),
                parent_resource_type: "model".to_string(),
                relation_name: None,
                domains: vec!["commerce".to_string()],
                grain: IndicatorGrainSummary {
                    primary_key: vec!["order_id".to_string()],
                    time_field: Some("order_date".to_string()),
                    dimensions: vec!["country_code".to_string()],
                },
                support_signals: Some(MetadataSupportSignals {
                    parent_synonyms: vec![],
                    domains: vec!["commerce".to_string()],
                    use_cases: vec!["revenue_reporting".to_string()],
                    dimensions: vec!["country_code".to_string()],
                    column_names: vec![],
                    column_roles: vec![],
                    column_semantic_types: vec![],
                    example_values: vec!["alpha".to_string()],
                }),
                explain: None,
            },
            IndicatorSearchRow {
                indicator_name: "promoted_gmv".to_string(),
                indicator_type: "measure".to_string(),
                canonical: true,
                match_type: "name".to_string(),
                score: 9.0,
                description: None,
                expression: None,
                field: None,
                parent_unique_id: "model.pkg.promotions".to_string(),
                parent_name: "promotions".to_string(),
                parent_resource_type: "model".to_string(),
                relation_name: None,
                domains: vec!["promotions".to_string()],
                grain: IndicatorGrainSummary {
                    primary_key: Vec::new(),
                    time_field: None,
                    dimensions: Vec::new(),
                },
                support_signals: None,
                explain: None,
            },
        ];

        let adjusted =
            apply_parent_coherence_bonus(rows, &SearchConfig::default().indicator_ranking);
        assert_eq!(adjusted[0].parent_unique_id, "model.pkg.fact_orders");
        assert_eq!(adjusted[0].indicator_name, "gmv");
    }

    #[test]
    fn compare_search_candidates_orders_by_final_score_before_parent_signal() {
        let left = SearchCandidate {
            unique_id: "model.pkg.high_score".to_string(),
            entity: None,
            score: 10.0,
            support_signals: None,
            indicator_parent_score: Some(0.2),
            explain: None,
        };
        let right = SearchCandidate {
            unique_id: "model.pkg.low_score".to_string(),
            entity: None,
            score: 8.0,
            support_signals: None,
            indicator_parent_score: Some(0.9),
            explain: None,
        };

        assert_eq!(compare_search_candidates(&left, &right), Ordering::Less);
    }

    #[test]
    fn build_indicator_parent_groups_merges_and_caps_support_signals() {
        let rows = vec![
            IndicatorSearchRow {
                indicator_name: "gmv".to_string(),
                indicator_type: "measure".to_string(),
                canonical: true,
                match_type: "name".to_string(),
                score: 10.0,
                description: None,
                expression: None,
                field: None,
                parent_unique_id: "model.pkg.fact_orders".to_string(),
                parent_name: "fact_orders".to_string(),
                parent_resource_type: "model".to_string(),
                relation_name: None,
                domains: vec!["commerce".to_string()],
                grain: IndicatorGrainSummary {
                    primary_key: Vec::new(),
                    time_field: Some("order_date".to_string()),
                    dimensions: vec!["country_code".to_string()],
                },
                support_signals: Some(MetadataSupportSignals {
                    parent_synonyms: vec!["gmv".to_string()],
                    domains: vec!["commerce".to_string()],
                    use_cases: vec![],
                    dimensions: vec!["country_code".to_string()],
                    column_names: vec![],
                    column_roles: vec![],
                    column_semantic_types: vec![],
                    example_values: vec!["alpha".to_string(), "beta".to_string()],
                }),
                explain: None,
            },
            IndicatorSearchRow {
                indicator_name: "net_sales".to_string(),
                indicator_type: "measure".to_string(),
                canonical: true,
                match_type: "synonym".to_string(),
                score: 9.5,
                description: None,
                expression: None,
                field: None,
                parent_unique_id: "model.pkg.fact_orders".to_string(),
                parent_name: "fact_orders".to_string(),
                parent_resource_type: "model".to_string(),
                relation_name: None,
                domains: vec!["commerce".to_string()],
                grain: IndicatorGrainSummary {
                    primary_key: Vec::new(),
                    time_field: Some("order_date".to_string()),
                    dimensions: vec!["country_code".to_string()],
                },
                support_signals: Some(MetadataSupportSignals {
                    parent_synonyms: vec![],
                    domains: vec![],
                    use_cases: vec!["revenue_reporting".to_string()],
                    dimensions: vec![],
                    column_names: vec![],
                    column_roles: vec![],
                    column_semantic_types: vec![],
                    example_values: vec![
                        "gamma".to_string(),
                        "delta".to_string(),
                        "epsilon".to_string(),
                    ],
                }),
                explain: None,
            },
        ];

        let config = SearchConfig::default();
        let groups = build_indicator_parent_groups(
            &rows,
            &config.indicator_ranking,
            &config.metadata_support,
        );
        assert_eq!(groups.len(), 1);
        let support_signals = groups[0]
            .support_signals
            .as_ref()
            .expect("expected merged support signals");
        assert_eq!(support_signals.domains, vec!["commerce".to_string()]);
        assert_eq!(
            support_signals.use_cases,
            vec!["revenue_reporting".to_string()]
        );
        assert_eq!(support_signals.example_values.len(), 4);
        assert_eq!(support_signals.example_values[0], "alpha");
        assert_eq!(support_signals.example_values[3], "delta");
    }
}

fn persona_weights(persona: SearchPersona, config: &crate::config::SearchConfig) -> PersonaWeights {
    match persona {
        SearchPersona::Analyst => config.persona_weights.analyst,
        SearchPersona::Engineer => config.persona_weights.engineer,
        SearchPersona::Governance => config.persona_weights.governance,
        SearchPersona::Default => config.persona_weights.default,
    }
}

fn hits_to_ids(hits: &[crate::manifest::tantivy_search::SearchHit]) -> Vec<String> {
    hits.iter().map(|h| h.unique_id.clone()).collect()
}

fn scores_to_ids(hits: &[(String, f32)]) -> Vec<String> {
    hits.iter().map(|(id, _)| id.clone()).collect()
}

fn indicator_embedding_text(row: &IndicatorSearchRow) -> String {
    let mut parts = vec![
        format!("indicator_type: {}", row.indicator_type),
        format!("indicator_name: {}", row.indicator_name),
        format!("parent_name: {}", row.parent_name),
        format!("parent_resource_type: {}", row.parent_resource_type),
    ];
    if row.canonical {
        parts.push("canonical".to_string());
    }
    if let Some(description) = &row.description {
        parts.push(format!("description: {description}"));
    }
    if let Some(expression) = &row.expression {
        parts.push(format!("expression: {expression}"));
    }
    if let Some(field) = &row.field {
        parts.push(format!("field: {field}"));
    }
    if let Some(time_field) = &row.grain.time_field {
        parts.push(format!("time_field: {time_field}"));
    }
    if !row.grain.dimensions.is_empty() {
        parts.push(format!("dimensions: {}", row.grain.dimensions.join(", ")));
    }
    if !row.domains.is_empty() {
        parts.push(format!("domains: {}", row.domains.join(", ")));
    }
    parts.join("\n")
}

fn reorder_indicator_rows_with_reranker(
    mut rows: Vec<IndicatorSearchRow>,
    top_n: usize,
    reranked_hits: &[(usize, f32)],
    config: &IndicatorRankingConfig,
) -> Vec<IndicatorSearchRow> {
    let split = top_n.min(rows.len());
    let tail = rows.split_off(split);
    let head = rows;
    let mut rerank_scores: HashMap<usize, f32> = HashMap::new();
    for (idx, score) in reranked_hits {
        rerank_scores.entry(*idx).or_insert(*score);
    }

    let mut head_rows: Vec<(usize, f32, IndicatorSearchRow)> = head
        .into_iter()
        .enumerate()
        .map(|(idx, mut row)| {
            let rerank_score = rerank_scores.get(&idx).copied().unwrap_or_default();
            row.score += rerank_score * config.indicator_reranker_score_weight;
            if let Some(explain) = &mut row.explain {
                explain.reranker_bonus =
                    non_zero_option(rerank_score * config.indicator_reranker_score_weight);
                explain.final_score = row.score;
            }
            (idx, rerank_score, row)
        })
        .collect();
    head_rows.sort_by(|left, right| {
        right
            .2
            .score
            .partial_cmp(&left.2.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| right.1.partial_cmp(&left.1).unwrap_or(Ordering::Equal))
            .then_with(|| left.0.cmp(&right.0))
    });

    let mut reordered = Vec::with_capacity(head_rows.len() + tail.len());
    reordered.extend(head_rows.into_iter().map(|(_, _, row)| row));
    reordered.extend(tail);
    reordered
}

fn build_indicator_parent_groups(
    rows: &[IndicatorSearchRow],
    ranking_config: &IndicatorRankingConfig,
    metadata_config: &MetadataSupportConfig,
) -> Vec<IndicatorParentGroup> {
    let mut groups: Vec<IndicatorParentGroup> = Vec::new();
    let mut group_index_by_parent: HashMap<&str, usize> = HashMap::new();

    for row in rows {
        let group_idx = if let Some(idx) = group_index_by_parent.get(row.parent_unique_id.as_str())
        {
            *idx
        } else {
            if groups.len() >= ranking_config.parent_group_max_groups {
                continue;
            }
            let idx = groups.len();
            groups.push(IndicatorParentGroup {
                parent_unique_id: row.parent_unique_id.clone(),
                parent_name: row.parent_name.clone(),
                parent_resource_type: row.parent_resource_type.clone(),
                relation_name: row.relation_name.clone(),
                domains: row.domains.clone(),
                best_score: row.score,
                indicator_count: 0,
                grain: row.grain.clone(),
                indicators: Vec::new(),
                support_signals: row.support_signals.clone(),
            });
            group_index_by_parent.insert(row.parent_unique_id.as_str(), idx);
            idx
        };

        let group = &mut groups[group_idx];
        group.best_score = group.best_score.max(row.score);
        group.indicator_count += 1;
        if let Some(signals) = &row.support_signals {
            if let Some(existing) = &mut group.support_signals {
                existing.merge_from(signals, metadata_config.max_values_per_field);
            } else {
                group.support_signals = Some(signals.clone());
            }
        }
        if group.indicators.len() < ranking_config.parent_group_max_indicators {
            group.indicators.push(IndicatorParentGroupItem {
                indicator_name: row.indicator_name.clone(),
                indicator_type: row.indicator_type.clone(),
                canonical: row.canonical,
                match_type: row.match_type.clone(),
                score: row.score,
            });
        }
    }

    groups
}

fn indicator_row_key(row: &IndicatorSearchRow) -> String {
    format!(
        "{}::{}::{}",
        row.parent_unique_id, row.indicator_type, row.indicator_name
    )
}

fn expand_indicator_parent_ranking(
    rows: &[IndicatorSearchRow],
    parent_hits: &[(String, f32)],
) -> Vec<String> {
    let mut row_keys_by_parent: HashMap<&str, Vec<String>> = HashMap::new();
    for row in rows {
        row_keys_by_parent
            .entry(row.parent_unique_id.as_str())
            .or_default()
            .push(indicator_row_key(row));
    }

    let mut ranking = Vec::new();
    for (parent_unique_id, _) in parent_hits {
        if let Some(row_keys) = row_keys_by_parent.get(parent_unique_id.as_str()) {
            ranking.extend(row_keys.iter().cloned());
        }
    }
    ranking
}

fn retriever_weight(weights: &PersonaWeights, name: &str) -> f32 {
    match name {
        "bm25" => weights.bm25,
        "ngram" => weights.ngram,
        "fuzzy" => weights.fuzzy,
        "vector" => weights.vector,
        "sparse" => weights.sparse,
        "indicator" => weights.indicator,
        _ => 1.0,
    }
}

#[allow(clippy::cast_precision_loss)]
fn weighted_rrf_with_explain(
    rankings: &[(&str, Vec<String>)],
    weights: &PersonaWeights,
    k: f32,
) -> Vec<(String, RetrievalExplain)> {
    let mut scores: HashMap<String, RetrievalExplain> = HashMap::new();
    for (name, ranking) in rankings {
        let weight = retriever_weight(weights, name);
        for (rank, id) in ranking.iter().enumerate() {
            let rrf_score = weight / (k + usize_to_f32(rank) + 1.0);
            let entry = scores.entry(id.clone()).or_default();
            entry.total_score += rrf_score;
            entry.retrievers.insert(
                (*name).to_string(),
                RetrieverContribution {
                    rank: rank + 1,
                    score: rrf_score,
                },
            );
        }
    }
    let mut results: Vec<(String, RetrievalExplain)> = scores.into_iter().collect();
    results.sort_by(|left, right| {
        compare_scores_desc(left.1.total_score, right.1.total_score)
            .then_with(|| left.0.cmp(&right.0))
    });
    results
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{SearchCandidate, analyst_near_tie_hint, resource_type_allowed_for_search};

    #[test]
    fn analyst_near_tie_hint_present_for_close_scores() {
        let candidates = vec![
            SearchCandidate {
                unique_id: "model.analytics.orders".to_string(),
                entity: None,
                score: 10.0,
                support_signals: None,
                indicator_parent_score: None,
                explain: None,
            },
            SearchCandidate {
                unique_id: "model.analytics.sessions".to_string(),
                entity: None,
                score: 9.4,
                support_signals: None,
                indicator_parent_score: None,
                explain: None,
            },
        ];
        let hint = analyst_near_tie_hint(&candidates).expect("near tie hint");
        assert!(hint.contains("Top candidates"));
        assert!(hint.contains("get_entity"));
    }

    #[test]
    fn analyst_near_tie_hint_absent_for_clear_winner() {
        let candidates = vec![
            SearchCandidate {
                unique_id: "model.analytics.orders".to_string(),
                entity: None,
                score: 10.0,
                support_signals: None,
                indicator_parent_score: None,
                explain: None,
            },
            SearchCandidate {
                unique_id: "model.analytics.sessions".to_string(),
                entity: None,
                score: 6.5,
                support_signals: None,
                indicator_parent_score: None,
                explain: None,
            },
        ];
        assert!(analyst_near_tie_hint(&candidates).is_none());
    }

    #[test]
    fn resource_type_filter_enforced() {
        let mut allowed: HashSet<String> = HashSet::new();
        allowed.insert("model".to_string());
        assert!(resource_type_allowed_for_search(
            Some("model"),
            Some(&allowed)
        ));
        assert!(!resource_type_allowed_for_search(
            Some("doc"),
            Some(&allowed)
        ));
        assert!(!resource_type_allowed_for_search(None, Some(&allowed)));
    }

    #[test]
    fn resource_type_filter_skipped_when_not_provided() {
        assert!(resource_type_allowed_for_search(Some("model"), None));
        assert!(resource_type_allowed_for_search(Some("doc"), None));
        assert!(resource_type_allowed_for_search(None, None));
    }
}
