use super::{
    AnalystSemanticConfig, Arc, ArchivedEntity, ArchivedNovaGrain, ArchivedNovaMeta, BTreeMap,
    ColumnInventoryRow, ColumnSearchRow, DbtNovaError, DetailLevel, Duration, FusedHitBundle,
    FusedHitContext, HashMap, HashSet, IndicatorInventoryRow, IndicatorSearchRow, Instant,
    JsonValue, ManifestSearch, MetadataSupportSignals, Ordering, PersonaWeights, Result,
    RetrievalExplain, RetrieverContribution, SearchCandidate, SearchConfig, SearchHit,
    SearchParams, SearchPersona, SearchRequest, SearchResponse, SearchScope, SearchScoreContext,
    SearchScoreExplain, SearchScoreOutcome, TantivySearcher, apply_parent_coherence_bonus,
    build_search_explain_payload, collect_metadata_support_signals, debug,
    dedupe_indicator_parent_ids, embedding_text_from_archived, has_query_syntax,
    match_nova_semantics, metadata_support_bonus, normalized_indicator_parent_scores,
    normalized_resource_type_filter, resource_type_allowed_for_search,
    semantic_label_precision_bonus, strip_sql_fields, tokenize_alnum_lowercase, warn,
};

const MAX_FETCH_LIMIT: usize = 2000;

impl ManifestSearch {
    /// Full-text search across all entities using Tantivy.
    ///
    /// Searches names, aliases, descriptions, SQL code, file paths, column
    /// names, and tags with field boosting and relevance scoring.
    ///
    /// # Errors
    /// Returns an error if the query is invalid or search execution fails.
    #[allow(clippy::too_many_lines)]
    #[tracing::instrument(skip(self, params), fields(tool = "search", query_len = params.query.len(), limit = ?params.pagination.limit, offset = params.pagination.offset, fuzzy = params.fuzzy))]
    pub async fn search(&self, params: &SearchParams) -> Result<JsonValue> {
        debug!(
            query = %params.query,
            resource_types = ?params.resource_types,
            limit = ?params.pagination.limit,
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
        let deadline = SearchDeadline::from_config(&self.config.search);

        let detail = self.detail_level(params.detail);
        let persona = params
            .persona
            .as_deref()
            .or(self.config.search.default_persona.as_deref())
            .map_or(SearchPersona::Default, SearchPersona::parse);
        let token_set: HashSet<&str> = tokens.iter().map(String::as_str).collect();
        let persona_weights = persona_weights(persona, &self.config.search);

        let base_limit = limit.saturating_add(params.pagination.offset);
        let overfetch = self.config.search.rrf_overfetch.max(1);
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
                FusedHitContext {
                    persona,
                    persona_weights,
                    query_has_syntax,
                    fetch_limit,
                    primary_results: &primary_results,
                    deadline,
                },
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
                        match run_semantic_blocking(
                            deadline,
                            "reranker",
                            "document scoring",
                            move || reranker.rerank(&query, &docs, top_n),
                        )
                        .await?
                        {
                            SemanticWorkResult::Completed(Ok(reranked_hits)) => {
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
                            SemanticWorkResult::Completed(Err(err)) => {
                                self.reranker_breaker.on_failure().await;
                                warn!(error = %err, "reranker failed; using fused ranking");
                            }
                            SemanticWorkResult::SkippedDeadline => {
                                debug!("reranker skipped because request deadline was exhausted");
                            }
                            SemanticWorkResult::TimedOut => {
                                self.reranker_breaker.on_failure().await;
                                warn!("reranker timed out; using fused ranking");
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
        let mut score_cutoff_active = false;
        let mut total_available = 0usize;
        for (id, base_score) in fused_hits {
            if !score_cutoff_active {
                if let Some(previous) = prev_score
                    && base_score < previous * 0.01
                    && candidates.len() >= base_limit
                {
                    score_cutoff_active = true;
                } else {
                    prev_score = Some(base_score);
                }
            }
            let entity = self.get_entity_archived(&id)?;
            if !resource_type_allowed_for_search(
                entity.and_then(ArchivedEntity::resource_type_str),
                allowed_resource_types.as_ref(),
            ) {
                continue;
            }
            total_available += 1;
            if score_cutoff_active {
                continue;
            }
            let support_signals = match entity {
                Some(entity_ref) => entity_ref.nova_meta().and_then(|nova| {
                    collect_metadata_support_signals(
                        entity_ref,
                        nova,
                        params.query.as_str(),
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

        let start = params.pagination.offset.min(total_available);
        let end = (start + limit).min(total_available);

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
                    let provenance = self.provenance_for_archived_json(
                        &candidate.unique_id,
                        entity,
                        &entity_json,
                    );
                    let extended_meta_summary = self.extended_meta_summary(&entity_json);
                    if let Some(obj) = entity_json.as_object_mut() {
                        obj.insert("score".to_string(), JsonValue::from(candidate.score));
                        obj.insert("provenance".to_string(), provenance);
                        if let Some(summary) = extended_meta_summary {
                            obj.insert("extended_meta_summary".to_string(), summary);
                        }
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
            } else if detail == DetailLevel::Compact {
                if let Some(entity) = candidate.entity {
                    let mut summary = self.summary_for_compact(&candidate.unique_id, entity);
                    let entity_json = entity.to_json_value();
                    let provenance = self.provenance_for_archived_json(
                        &candidate.unique_id,
                        entity,
                        &entity_json,
                    );
                    if let Some(obj) = summary.as_object_mut() {
                        obj.insert("provenance".to_string(), provenance);
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
            } else {
                let mut summary = self.summary_from_archived(
                    &candidate.unique_id,
                    candidate.entity,
                    persona,
                    Some(&tokens),
                );
                let provenance = candidate.entity.map(|entity| {
                    let entity_json = entity.to_json_value();
                    self.provenance_for_archived_json(&candidate.unique_id, entity, &entity_json)
                });
                if let Some(obj) = summary.as_object_mut() {
                    if let Some(provenance) = provenance {
                        obj.insert("provenance".to_string(), provenance);
                    }
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

    pub(super) async fn build_fused_hits(
        &self,
        params: &SearchParams,
        context: FusedHitContext<'_>,
    ) -> Result<FusedHitBundle> {
        let FusedHitContext {
            persona,
            persona_weights,
            query_has_syntax,
            fetch_limit,
            primary_results,
            deadline,
        } = context;
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
        rankings.push(("bm25", hits_to_ids(primary_results)));

        let (ngram_ranking, fuzzy_ranking, vector_ranking, sparse_ranking, indicator_ranking) = tokio::try_join!(
            self.fused_ngram_ranking(params, persona, query_has_syntax, fetch_limit),
            self.fused_fuzzy_ranking(params, persona, fetch_limit),
            self.fused_vector_ranking(params, fetch_limit, deadline),
            self.fused_sparse_ranking(params, fetch_limit, deadline),
            self.fused_indicator_ranking(params, persona, fetch_limit, deadline),
        )?;

        if let Some(ranking) = ngram_ranking {
            rankings.push(("ngram", ranking));
        }
        if let Some(ranking) = fuzzy_ranking {
            rankings.push(("fuzzy", ranking));
        }
        if let Some(ranking) = vector_ranking {
            rankings.push(("vector", ranking));
        }
        if let Some(ranking) = sparse_ranking {
            rankings.push(("sparse", ranking));
        }
        let (indicator_ranking, indicator_parent_scores) = indicator_ranking;
        if let Some(ranking) = indicator_ranking {
            rankings.push(("indicator", ranking));
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

    pub(super) async fn fused_ngram_ranking(
        &self,
        params: &SearchParams,
        persona: SearchPersona,
        query_has_syntax: bool,
        fetch_limit: usize,
    ) -> Result<Option<Vec<String>>> {
        if !self.config.search.enable_ngram || query_has_syntax {
            return Ok(None);
        }
        let hits = run_tantivy_search(
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
        Ok(Some(hits_to_ids(&hits)))
    }

    pub(super) async fn fused_fuzzy_ranking(
        &self,
        params: &SearchParams,
        persona: SearchPersona,
        fetch_limit: usize,
    ) -> Result<Option<Vec<String>>> {
        if !params.fuzzy {
            return Ok(None);
        }
        let hits = run_tantivy_search(
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
        Ok(Some(hits_to_ids(&hits)))
    }

    pub(super) async fn fused_vector_ranking(
        &self,
        params: &SearchParams,
        fetch_limit: usize,
        deadline: SearchDeadline,
    ) -> Result<Option<Vec<String>>> {
        if !self.config.search.enable_vector_search {
            return Ok(None);
        }
        let Some(vector_search) = &self.vector_search else {
            return Ok(None);
        };
        if !self.vector_breaker.allow().await {
            debug!("vector search circuit open; skipping vector retriever");
            return Ok(None);
        }
        let vector_limit = self.config.search.vector_top_k.max(fetch_limit);
        let query = params.query.clone();
        let vector_search = Arc::clone(vector_search);
        match run_semantic_blocking(deadline, "vector search", "query embedding", move || {
            vector_search.search(&query, vector_limit)
        })
        .await?
        {
            SemanticWorkResult::Completed(Ok(vector_hits)) => {
                self.vector_breaker.on_success().await;
                Ok(Some(scores_to_ids(&vector_hits)))
            }
            SemanticWorkResult::Completed(Err(err)) => {
                self.vector_breaker.on_failure().await;
                warn!(error = %err, "vector search failed; skipping vector results");
                Ok(None)
            }
            SemanticWorkResult::SkippedDeadline => {
                debug!("vector retriever skipped because request deadline was exhausted");
                Ok(None)
            }
            SemanticWorkResult::TimedOut => {
                self.vector_breaker.on_failure().await;
                warn!("vector search timed out; skipping vector results");
                Ok(None)
            }
        }
    }

    pub(super) async fn fused_sparse_ranking(
        &self,
        params: &SearchParams,
        fetch_limit: usize,
        deadline: SearchDeadline,
    ) -> Result<Option<Vec<String>>> {
        if !self.config.search.enable_sparse_search {
            return Ok(None);
        }
        let Some(sparse_search) = &self.sparse_search else {
            return Ok(None);
        };
        if !self.sparse_breaker.allow().await {
            debug!("sparse search circuit open; skipping sparse retriever");
            return Ok(None);
        }
        let sparse_limit = self.config.search.sparse_top_k.max(fetch_limit);
        let query = params.query.clone();
        let sparse_search = Arc::clone(sparse_search);
        match run_semantic_blocking(deadline, "sparse search", "query embedding", move || {
            sparse_search.search(&query, sparse_limit)
        })
        .await?
        {
            SemanticWorkResult::Completed(Ok(sparse_hits)) => {
                self.sparse_breaker.on_success().await;
                Ok(Some(scores_to_ids(&sparse_hits)))
            }
            SemanticWorkResult::Completed(Err(err)) => {
                self.sparse_breaker.on_failure().await;
                warn!(error = %err, "sparse search failed; skipping sparse results");
                Ok(None)
            }
            SemanticWorkResult::SkippedDeadline => {
                debug!("sparse retriever skipped because request deadline was exhausted");
                Ok(None)
            }
            SemanticWorkResult::TimedOut => {
                self.sparse_breaker.on_failure().await;
                warn!("sparse search timed out; skipping sparse results");
                Ok(None)
            }
        }
    }

    pub(super) async fn fused_indicator_ranking(
        &self,
        params: &SearchParams,
        persona: SearchPersona,
        fetch_limit: usize,
        deadline: SearchDeadline,
    ) -> Result<(Option<Vec<String>>, HashMap<String, f32>)> {
        if persona != SearchPersona::Analyst {
            return Ok((None, HashMap::new()));
        }
        let indicator_tokens =
            tokenize_alnum_lowercase(&params.query, self.config.search.min_word_length.max(1));
        let resource_filter = normalized_resource_type_filter(&params.resource_types);
        let mut indicator_rows = self
            .search_indicator_rows_blocking(
                indicator_tokens,
                resource_filter,
                None,
                false,
                deadline,
            )
            .await?;
        if self.config.search.indicator_ranking.enable_parent_coherence && indicator_rows.len() > 1
        {
            indicator_rows =
                apply_parent_coherence_bonus(indicator_rows, &self.config.search.indicator_ranking);
        }
        let ranking = dedupe_indicator_parent_ids(&indicator_rows, fetch_limit);
        if ranking.is_empty() {
            return Ok((None, HashMap::new()));
        }
        let scores = normalized_indicator_parent_scores(
            &indicator_rows,
            &self.config.search.indicator_ranking,
        );
        Ok((Some(ranking), scores))
    }

    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    pub(super) fn adjust_score_with_meta(
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
                    phrase_match_multiplier: None,
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
            phrase_match_multiplier: None,
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
        if let Some(phrase_match_multiplier) = phrase_match_multiplier(
            context.support_signals,
            context.token_set.len(),
            self.config.search.enable_phrase_boost,
        ) {
            adjusted *= phrase_match_multiplier;
            if let Some(debug) = &mut explain_payload {
                debug.phrase_match_multiplier = Some(phrase_match_multiplier);
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

    pub(super) async fn build_suggestions(
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
}
pub(super) const SEMANTIC_TIMEOUT_RESERVE_MS: u64 = 10;

pub(super) const SCAN_DEADLINE_CHECK_INTERVAL: usize = 64;

#[derive(Debug, Clone, Copy)]
pub(super) struct SearchDeadline {
    pub(super) started_at: Instant,
    pub(super) timeout_ms: u64,
}

impl SearchDeadline {
    pub(super) fn from_config(config: &SearchConfig) -> Self {
        Self {
            started_at: Instant::now(),
            timeout_ms: u64::try_from(config.search_timeout_ms).unwrap_or(u64::MAX),
        }
    }

    pub(super) fn semantic_remaining(self) -> Option<Duration> {
        if self.timeout_ms == 0 {
            return None;
        }
        let elapsed_ms = u64::try_from(self.started_at.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.timeout_ms
            .checked_sub(elapsed_ms)?
            .checked_sub(SEMANTIC_TIMEOUT_RESERVE_MS)
            .map(Duration::from_millis)
            .filter(|remaining| !remaining.is_zero())
    }

    pub(super) fn has_timeout(self) -> bool {
        self.timeout_ms > 0
    }

    pub(super) fn check(self) -> Result<()> {
        if !self.has_timeout() {
            return Ok(());
        }
        let elapsed_ms = u64::try_from(self.started_at.elapsed().as_millis()).unwrap_or(u64::MAX);
        if elapsed_ms >= self.timeout_ms {
            return Err(DbtNovaError::ServerError(format!(
                "Search timed out after {}ms",
                self.timeout_ms
            )));
        }
        Ok(())
    }
}

pub(super) enum SemanticWorkResult<T> {
    Completed(Result<T>),
    SkippedDeadline,
    TimedOut,
}

pub(super) fn check_scan_deadline(deadline: SearchDeadline, scanned: &mut usize) -> Result<()> {
    *scanned = scanned.wrapping_add(1);
    if *scanned == 1 || (*scanned).is_multiple_of(SCAN_DEADLINE_CHECK_INTERVAL) {
        deadline.check()?;
    }
    Ok(())
}

pub(super) async fn run_semantic_blocking<T, F>(
    deadline: SearchDeadline,
    component: &'static str,
    operation: &'static str,
    f: F,
) -> Result<SemanticWorkResult<T>>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    let remaining = deadline.semantic_remaining();
    if deadline.has_timeout() && remaining.is_none() {
        debug!(
            component,
            operation, "semantic operation skipped because request deadline is exhausted"
        );
        return Ok(SemanticWorkResult::SkippedDeadline);
    }

    let mut handle = tokio::task::spawn_blocking(f);
    if let Some(remaining) = remaining {
        if let Ok(joined) = tokio::time::timeout(remaining, &mut handle).await {
            joined
                .map(SemanticWorkResult::Completed)
                .map_err(|error| DbtNovaError::ServerError(error.to_string()))
        } else {
            handle.abort();
            warn!(
                component,
                operation,
                timeout_ms = remaining.as_millis(),
                "semantic operation timed out before request deadline"
            );
            Ok(SemanticWorkResult::TimedOut)
        }
    } else {
        handle
            .await
            .map(SemanticWorkResult::Completed)
            .map_err(|error| DbtNovaError::ServerError(error.to_string()))
    }
}

pub(super) struct OwnedSearchRequest {
    pub(super) query_text: String,
    pub(super) resource_types: Vec<String>,
    pub(super) limit: usize,
    pub(super) min_score: Option<f32>,
    pub(super) fuzzy: bool,
    pub(super) include_highlights: bool,
    pub(super) include_ngram_override: Option<bool>,
    pub(super) scope: SearchScope,
    pub(super) persona: SearchPersona,
}

pub(super) async fn run_tantivy_search(
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

pub(super) fn tokens_match(value: &str, token_set: &HashSet<&str>, min_word_len: usize) -> bool {
    if value.is_empty() || token_set.is_empty() {
        return false;
    }
    tokenize_alnum_lowercase(value, min_word_len)
        .iter()
        .any(|t| token_set.contains(t.as_str()))
}

pub(super) fn analyst_semantic_multiplier(
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

pub(super) fn analyst_query_coverage_multiplier(
    best_query_coverage: f32,
    query_token_count: usize,
) -> f32 {
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

pub(super) fn phrase_match_multiplier(
    signals: Option<&MetadataSupportSignals>,
    query_token_count: usize,
    enabled: bool,
) -> Option<f32> {
    if !enabled || query_token_count <= 1 {
        return None;
    }
    let signals = signals?;
    let coverage = signals
        .best_single_field_query_coverage
        .unwrap_or_default()
        .clamp(0.0, 1.0);
    let multiplier = if !signals.exact_phrases.is_empty() {
        3.0 + coverage
    } else if coverage >= 0.75 {
        1.0 + (coverage * 1.5)
    } else if coverage >= 0.5 {
        1.0 + (coverage * 0.5)
    } else {
        1.0
    };
    non_neutral_option(multiplier)
}

pub(super) fn preferred_grain_for_scoring(nova: &ArchivedNovaMeta) -> Option<&ArchivedNovaGrain> {
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

pub(super) const ANALYST_NEAR_TIE_GAP_THRESHOLD_PCT: f32 = 8.0;

pub(super) fn analyst_near_tie_hint(candidates: &[SearchCandidate<'_>]) -> Option<String> {
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

pub(super) fn candidate_display_name(candidate: &SearchCandidate<'_>) -> String {
    if let Some(entity) = candidate.entity
        && let Some(name) = entity.name_str()
    {
        return name.to_string();
    }
    candidate.unique_id.clone()
}

pub(super) fn bool_to_f32(value: bool) -> f32 {
    if value { 1.0 } else { 0.0 }
}

pub(super) fn is_neutral_multiplier(value: f32) -> bool {
    (value - 1.0).abs() < f32::EPSILON
}

pub(super) fn usize_to_f32(value: usize) -> f32 {
    f32::from(u16::try_from(value).unwrap_or(u16::MAX))
}

pub(super) fn non_zero_option(value: f32) -> Option<f32> {
    (value.abs() > f32::EPSILON).then_some(value)
}

pub(super) fn non_neutral_option(value: f32) -> Option<f32> {
    ((value - 1.0).abs() > f32::EPSILON).then_some(value)
}

pub(super) fn compare_scores_desc(left: f32, right: f32) -> Ordering {
    right.partial_cmp(&left).unwrap_or(Ordering::Equal)
}

pub(super) fn compare_indicator_rows(
    left: &IndicatorSearchRow,
    right: &IndicatorSearchRow,
) -> Ordering {
    compare_scores_desc(left.score, right.score)
        .then_with(|| left.parent_unique_id.cmp(&right.parent_unique_id))
        .then_with(|| left.indicator_type.cmp(&right.indicator_type))
        .then_with(|| left.indicator_name.cmp(&right.indicator_name))
}

pub(super) fn compare_indicator_inventory_rows(
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

pub(super) fn compare_column_inventory_rows(
    left: &ColumnInventoryRow,
    right: &ColumnInventoryRow,
) -> Ordering {
    left.parent_unique_id
        .cmp(&right.parent_unique_id)
        .then_with(|| left.column_name.cmp(&right.column_name))
}

pub(super) fn compare_column_search_rows(
    left: &ColumnSearchRow,
    right: &ColumnSearchRow,
) -> Ordering {
    compare_scores_desc(left.score, right.score)
        .then_with(|| left.parent_unique_id.cmp(&right.parent_unique_id))
        .then_with(|| left.column_name.cmp(&right.column_name))
        .then_with(|| left.match_type.cmp(&right.match_type))
}

pub(super) fn compare_search_candidates(
    left: &SearchCandidate<'_>,
    right: &SearchCandidate<'_>,
) -> Ordering {
    compare_scores_desc(left.score, right.score)
        .then_with(|| {
            compare_scores_desc(
                left.indicator_parent_score.unwrap_or_default(),
                right.indicator_parent_score.unwrap_or_default(),
            )
        })
        .then_with(|| left.unique_id.cmp(&right.unique_id))
}

pub(super) fn query_exact_match(query: &str, unique_id: &str, entity: &ArchivedEntity) -> bool {
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

pub(super) fn candidate_flag_for_persona(
    nova: &ArchivedNovaMeta,
    persona: SearchPersona,
) -> Option<bool> {
    let candidates = nova.search.as_ref()?.candidates.as_ref()?;
    Some(match persona {
        SearchPersona::Analyst => candidates.analyst,
        SearchPersona::Engineer => candidates.engineer,
        SearchPersona::Governance => candidates.governance,
        SearchPersona::Default => true,
    })
}

pub(super) fn candidate_false_multiplier(
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

pub(super) fn suggestion_allowed(persona: SearchPersona, resource_type: Option<&str>) -> bool {
    let rt = resource_type.unwrap_or("").to_lowercase();
    match persona {
        SearchPersona::Analyst => !matches!(rt.as_str(), "test" | "macro"),
        SearchPersona::Governance => !matches!(rt.as_str(), "macro"),
        SearchPersona::Engineer | SearchPersona::Default => true,
    }
}

pub(super) fn persona_resource_type_multiplier(persona: SearchPersona, resource_type: &str) -> f32 {
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

pub(super) fn persona_semantic_match_multiplier(
    persona: SearchPersona,
    config: &SearchConfig,
) -> f32 {
    match persona {
        SearchPersona::Analyst => config.analyst_nova_semantic_match_multiplier,
        SearchPersona::Engineer | SearchPersona::Governance | SearchPersona::Default => {
            config.non_analyst_nova_semantic_match_multiplier
        }
    }
}

pub(super) fn persona_weights(
    persona: SearchPersona,
    config: &crate::config::SearchConfig,
) -> PersonaWeights {
    match persona {
        SearchPersona::Analyst => config.persona_weights.analyst,
        SearchPersona::Engineer => config.persona_weights.engineer,
        SearchPersona::Governance => config.persona_weights.governance,
        SearchPersona::Default => config.persona_weights.default,
    }
}

pub(super) fn hits_to_ids(hits: &[crate::manifest::tantivy_search::SearchHit]) -> Vec<String> {
    hits.iter().map(|h| h.unique_id.clone()).collect()
}

pub(super) fn scores_to_ids(hits: &[(String, f32)]) -> Vec<String> {
    hits.iter().map(|(id, _)| id.clone()).collect()
}

pub(super) fn retriever_weight(weights: &PersonaWeights, name: &str) -> f32 {
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
pub(super) fn weighted_rrf_with_explain(
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
