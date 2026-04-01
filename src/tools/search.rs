use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde_json::Value as JsonValue;

use crate::config::PersonaWeights;
use crate::config::search::{AnalystSemanticConfig, SearchConfig};
use crate::error::{DbtNovaError, Result};
use crate::manifest::entity::{ArchivedEntity, ArchivedNovaGrain, ArchivedNovaMeta};
use crate::manifest::search::ManifestSearch;
use crate::manifest::tantivy_search::{SearchHit, SearchRequest, SearchScope, TantivySearcher};
use crate::manifest::vector_search::embedding_text_from_archived;
use crate::params::{DetailLevel, ListEntitiesParams, SearchParams};
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

        let mut fused_hits = self
            .build_fused_hits(
                params,
                persona,
                persona_weights,
                query_has_syntax,
                fetch_limit,
                &primary_results,
            )
            .await?;

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
                                        reordered.push((id, score));
                                    }
                                }
                                for (id, score) in fused_hits.into_iter().skip(top_n) {
                                    if seen.insert(id.clone()) {
                                        reordered.push((id, score));
                                    }
                                }
                                fused_hits = reordered;
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
            return Ok(serde_json::to_value(response)?);
        }

        let total_hits = fused_hits.len();
        let mut candidates: Vec<(String, Option<&ArchivedEntity>, f32)> =
            Vec::with_capacity(total_hits);
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
            let adjusted = self.adjust_score_with_meta(
                &id,
                base_score,
                entity,
                &tokens,
                min_word_len,
                persona,
                &params.query,
            );
            candidates.push((id, entity, adjusted));
        }
        candidates.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
        let analysis_hints: Vec<String> = if persona == SearchPersona::Analyst {
            analyst_near_tie_hint(&candidates).into_iter().collect()
        } else {
            Vec::new()
        };

        let start = params.pagination.offset.min(candidates.len());
        let end = (start + limit).min(candidates.len());

        let mut results: Vec<JsonValue> = Vec::new();
        for (id, entity, score) in candidates.into_iter().skip(start).take(end - start) {
            let highlight_value = highlight_map.get(&id).cloned().unwrap_or(None);

            if detail == DetailLevel::Full {
                if let Some(entity) = entity {
                    let mut entity_json =
                        serde_json::from_str(entity.payload_json()).unwrap_or(JsonValue::Null);
                    if !include_sql {
                        strip_sql_fields(&mut entity_json);
                    }
                    ManifestSearch::insert_unique_id(&mut entity_json, &id);
                    if let Some(obj) = entity_json.as_object_mut() {
                        obj.insert("score".to_string(), JsonValue::from(score));
                        if let Some(highlights) = highlight_value {
                            obj.insert("highlights".to_string(), highlights);
                        }
                    }
                    results.push(entity_json);
                }
            } else {
                let mut summary = self.summary_from_archived(&id, entity, persona, Some(&tokens));
                if let Some(obj) = summary.as_object_mut() {
                    obj.insert("score".to_string(), JsonValue::from(score));
                    if let Some(highlights) = highlight_value {
                        obj.insert("highlights".to_string(), highlights);
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
        Ok(serde_json::to_value(response)?)
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
    ) -> Result<Vec<(String, f32)>> {
        if !self.config.search.enable_rrf {
            return Ok(primary_results
                .iter()
                .map(|hit| (hit.unique_id.clone(), hit.score))
                .collect());
        }

        let mut rankings: Vec<(&str, Vec<String>)> = Vec::new();

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

        let fusion_limit = fetch_limit.max(1);
        for (_, ranking) in &mut rankings {
            if ranking.len() > fusion_limit {
                ranking.truncate(fusion_limit);
            }
        }

        Ok(weighted_rrf(
            &rankings,
            &persona_weights,
            self.config.search.rrf_k,
        ))
    }

    #[allow(clippy::float_cmp, clippy::too_many_lines, clippy::too_many_arguments)]
    fn adjust_score_with_meta(
        &self,
        unique_id: &str,
        score: f32,
        entity: Option<&ArchivedEntity>,
        tokens: &[String],
        min_word_len: usize,
        persona: SearchPersona,
        query_text: &str,
    ) -> f32 {
        if score <= 0.0 {
            return score;
        }

        let mut adjusted = score;
        let weights = persona_weights(persona, &self.config.search);
        let Some(entity) = entity else {
            return adjusted;
        };
        let exact_match = query_exact_match(query_text, unique_id, entity);

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
        }

        if persona == SearchPersona::Engineer && exact_match {
            adjusted *= self.config.search.engineer_exact_match_multiplier;
        }

        let resource_type = entity.resource_type_str().unwrap_or("");
        adjusted *= persona_resource_type_multiplier(persona, resource_type);

        if tokens.is_empty() {
            return adjusted;
        }

        let Some(nova) = entity.nova_meta() else {
            if persona == SearchPersona::Analyst {
                adjusted *= 0.93;
            }
            return adjusted;
        };

        adjusted *= candidate_false_multiplier(
            persona,
            candidate_flag_for_persona(nova, persona),
            exact_match,
            &self.config.search,
        );

        let token_set: HashSet<&str> = tokens.iter().map(String::as_str).collect();

        let mut measure_match = false;
        for measure in nova.measures.iter() {
            if tokens_match(measure.name.as_str(), &token_set, min_word_len) {
                measure_match = true;
                break;
            }
            for syn in measure.synonyms.iter() {
                if tokens_match(syn.as_str(), &token_set, min_word_len) {
                    measure_match = true;
                    break;
                }
            }
            if measure_match {
                break;
            }
        }

        let mut metric_match = false;
        for metric in nova.metric.iter().chain(nova.metrics.iter()) {
            if tokens_match(metric.name.as_str(), &token_set, min_word_len) {
                metric_match = true;
                break;
            }
            for syn in metric.synonyms.iter() {
                if tokens_match(syn.as_str(), &token_set, min_word_len) {
                    metric_match = true;
                    break;
                }
            }
            if metric_match {
                break;
            }
        }

        let mut synonym_match = false;
        for syn in nova.synonyms.iter() {
            if tokens_match(syn.as_str(), &token_set, min_word_len) {
                synonym_match = true;
                break;
            }
        }

        if measure_match {
            adjusted *= self.config.search.nova_measure_match_multiplier * weights.measures;
        }
        if metric_match {
            adjusted *= self.config.search.nova_metric_match_multiplier * weights.metrics;
        }
        if synonym_match {
            adjusted *= self.config.search.nova_synonym_match_multiplier * weights.synonyms;
        }
        let canonical = nova.canonical;
        if canonical {
            adjusted *= self.config.search.nova_canonical_multiplier;
        }
        if canonical && (measure_match || metric_match || synonym_match) {
            adjusted *= self.config.search.nova_canonical_match_multiplier;
            adjusted += self.config.search.nova_canonical_match_bonus;
        }

        if persona == SearchPersona::Analyst {
            adjusted *= analyst_semantic_multiplier(
                nova,
                &token_set,
                min_word_len,
                &self.config.search.analyst_semantic,
            );
        }

        if weights.docs != 1.0 {
            let has_desc = entity
                .description_str()
                .is_some_and(|d| !d.trim().is_empty());
            let has_docs = entity.doc_blocks_present();
            if has_desc || has_docs {
                adjusted *= weights.docs;
            }
        }

        if weights.tests != 1.0 && self.tests_by_entity.contains_key(unique_id) {
            adjusted *= weights.tests;
        }

        if weights.tags != 1.0 {
            let has_tags = entity.tags_iter().next().is_some();
            if has_tags {
                adjusted *= weights.tags;
            }
        }

        if weights.path != 1.0 {
            let path = entity.original_file_path_str().unwrap_or("");
            if tokens_match(path, &token_set, min_word_len) {
                adjusted *= weights.path;
            }
        }

        adjusted
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

fn analyst_near_tie_hint(candidates: &[(String, Option<&ArchivedEntity>, f32)]) -> Option<String> {
    if candidates.len() < 2 {
        return None;
    }

    let top_score = candidates[0].2;
    let second_score = candidates[1].2;
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

fn candidate_display_name(candidate: &(String, Option<&ArchivedEntity>, f32)) -> String {
    if let Some(entity) = candidate.1
        && let Some(name) = entity.name_str()
    {
        return name.to_string();
    }
    candidate.0.clone()
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
            "metric" | "semantic_model" => 1.35,
            "saved_query" => 1.25,
            "model" => 1.2,
            "analysis" => 1.15,
            "exposure" => 1.1,
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

fn retriever_weight(weights: &PersonaWeights, name: &str) -> f32 {
    match name {
        "bm25" => weights.bm25,
        "ngram" => weights.ngram,
        "fuzzy" => weights.fuzzy,
        "vector" => weights.vector,
        "sparse" => weights.sparse,
        _ => 1.0,
    }
}

#[allow(clippy::cast_precision_loss)]
fn weighted_rrf(
    rankings: &[(&str, Vec<String>)],
    weights: &PersonaWeights,
    k: f32,
) -> Vec<(String, f32)> {
    let mut scores: HashMap<String, f32> = HashMap::new();
    for (name, ranking) in rankings {
        let weight = retriever_weight(weights, name);
        for (rank, id) in ranking.iter().enumerate() {
            let rrf_score = weight / (k + (rank as f32) + 1.0);
            *scores.entry(id.clone()).or_default() += rrf_score;
        }
    }
    let mut results: Vec<(String, f32)> = scores.into_iter().collect();
    results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    results
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{ArchivedEntity, analyst_near_tie_hint, resource_type_allowed_for_search};

    #[test]
    fn analyst_near_tie_hint_present_for_close_scores() {
        let candidates: Vec<(String, Option<&ArchivedEntity>, f32)> = vec![
            ("model.analytics.orders".to_string(), None, 10.0),
            ("model.analytics.sessions".to_string(), None, 9.4),
        ];
        let hint = analyst_near_tie_hint(&candidates).expect("near tie hint");
        assert!(hint.contains("Top candidates"));
        assert!(hint.contains("get_entity"));
    }

    #[test]
    fn analyst_near_tie_hint_absent_for_clear_winner() {
        let candidates: Vec<(String, Option<&ArchivedEntity>, f32)> = vec![
            ("model.analytics.orders".to_string(), None, 10.0),
            ("model.analytics.sessions".to_string(), None, 6.5),
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
