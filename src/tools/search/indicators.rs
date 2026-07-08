use super::{
    Arc, ArchivedEntity, ArchivedNovaGrain, ArchivedNovaMeasure, ArchivedNovaMeta,
    ArchivedNovaMetric, ArchivedString, DbtNovaError, DetailLevel, EntityStore, FusedHitContext,
    HashMap, HashSet, IndicatorExecutionMetadata, IndicatorGrainSummary, IndicatorInventoryParams,
    IndicatorInventoryRow, IndicatorParentGroup, IndicatorParentGroupItem, IndicatorRankingConfig,
    IndicatorScoreExplain, IndicatorSearchContext, IndicatorSearchRow, JsonValue, ManifestSearch,
    MetadataSupportConfig, MetadataSupportSignals, NovaSemanticMatches, Ordering,
    OwnedSearchRequest, ParentGroupMode, ParentIndicatorCoherence, PreparedIndicatorSearch, Result,
    RetrievalExplain, SearchConfig, SearchDeadline, SearchExplainConfigSnapshot,
    SearchExplainPayload, SearchIndicatorParams, SearchParams, SearchPersona, SearchResponse,
    SearchScope, SemanticMatchType, SemanticWorkResult, SuccessResponse, check_scan_deadline,
    collect_column_metadata_support_signals, compare_indicator_inventory_rows,
    compare_indicator_rows, debug, has_query_syntax, match_nova_semantics, non_zero_option,
    normalized_resource_type_filter, persona_weights, preferred_grain_for_scoring,
    push_unique_string, resource_type_allowed_for_search, run_semantic_blocking,
    run_tantivy_search, tokenize_alnum_lowercase, usize_to_f32, warn, weighted_rrf_with_explain,
};

impl ManifestSearch {
    /// Resolve Nova measures and metrics that match the query.
    ///
    /// # Errors
    /// Returns an error if the query is invalid or indicator filtering is invalid.
    #[tracing::instrument(skip(self, params), fields(tool = "search_indicator", query_len = params.query.len(), limit = ?params.pagination.limit, offset = params.pagination.offset))]
    pub async fn search_indicator(&self, params: &SearchIndicatorParams) -> Result<JsonValue> {
        let deadline = SearchDeadline::from_config(&self.config.search);
        let (tokens, query_has_syntax, resource_filter, indicator_filter, persona) =
            self.prepare_indicator_search(params)?;
        let mut rows = self
            .search_indicator_rows_blocking(
                tokens.clone(),
                resource_filter.clone(),
                indicator_filter.clone(),
                params.explain,
                deadline,
            )
            .await?;
        rows = self
            .rank_indicator_rows(params, query_has_syntax, persona, rows, deadline)
            .await?;
        self.build_indicator_search_response(params, persona, query_has_syntax, &tokens, rows)
    }

    /// List Nova measures and metrics deterministically.
    ///
    /// # Errors
    /// Returns an error if indicator filtering is invalid or pagination exceeds
    /// configured limits.
    #[tracing::instrument(skip(self, params), fields(tool = "indicator_inventory", limit = ?params.pagination.limit, offset = params.pagination.offset))]
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

        let deadline = SearchDeadline::from_config(&self.config.search);
        let resource_filter = normalized_resource_type_filter(&params.resource_types);
        let indicator_filter = normalized_indicator_type_filter(&params.indicator_types)?;
        let rows = self
            .indicator_inventory_rows_blocking(
                resource_filter.clone(),
                indicator_filter.clone(),
                params.canonical_only,
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

    pub(super) fn prepare_indicator_search(
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

    pub(super) async fn rank_indicator_rows(
        &self,
        params: &SearchIndicatorParams,
        query_has_syntax: bool,
        persona: SearchPersona,
        mut rows: Vec<IndicatorSearchRow>,
        deadline: SearchDeadline,
    ) -> Result<Vec<IndicatorSearchRow>> {
        if self.config.search.indicator_ranking.enable_parent_coherence && rows.len() > 1 {
            rows = apply_parent_coherence_bonus(rows, &self.config.search.indicator_ranking);
        }
        if let Some(min_score) = params.min_score {
            rows.retain(|row| row.score >= min_score);
        }
        if self.config.search.enable_rrf && rows.len() > 1 {
            rows = self
                .fuse_indicator_rows(params, persona, query_has_syntax, rows, deadline)
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
                    match run_semantic_blocking(
                        deadline,
                        "reranker",
                        "indicator scoring",
                        move || reranker.rerank(&query, &docs, top_n),
                    )
                    .await?
                    {
                        SemanticWorkResult::Completed(Ok(reranked_hits)) => {
                            self.reranker_breaker.on_success().await;
                            rows = reorder_indicator_rows_with_reranker(
                                rows,
                                top_n,
                                &reranked_hits,
                                &self.config.search.indicator_ranking,
                            );
                        }
                        SemanticWorkResult::Completed(Err(err)) => {
                            self.reranker_breaker.on_failure().await;
                            warn!(
                                error = %err,
                                "indicator reranker failed; using existing indicator ranking"
                            );
                        }
                        SemanticWorkResult::SkippedDeadline => {
                            debug!(
                                "indicator reranker skipped because request deadline was exhausted"
                            );
                        }
                        SemanticWorkResult::TimedOut => {
                            self.reranker_breaker.on_failure().await;
                            warn!("indicator reranker timed out; using existing indicator ranking");
                        }
                    }
                }
            } else {
                debug!("reranker circuit open; skipping indicator rerank");
            }
        }
        Ok(rows)
    }

    pub(super) fn build_indicator_search_response(
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
        let mut parent_groups = build_indicator_parent_groups(
            &rows,
            &self.config.search.indicator_ranking,
            &self.config.search.metadata_support,
        );
        parent_groups = filter_indicator_parent_groups(parent_groups, params);
        let total_available = rows.len();
        let limit = self.page_limit(params.pagination.limit);
        let start = params.pagination.offset.min(rows.len());
        let end = (start + limit).min(rows.len());
        let detail = self.detail_level(params.detail);
        let results: Vec<JsonValue> = rows
            .into_iter()
            .skip(start)
            .take(end.saturating_sub(start))
            .map(|row| indicator_row_value(row, detail, params.include_support_signals))
            .collect();

        let count = results.len();
        let mut response = SearchResponse::new(results, count, persona.as_str().to_string())
            .with_total(total_available);
        if total_available > end {
            response = response.with_truncated(true);
        }
        let mut response_json = serde_json::to_value(response)?;
        if let Some(obj) = response_json.as_object_mut() {
            if params.group_mode.unwrap_or_default() != ParentGroupMode::None {
                let mut value = serde_json::to_value(parent_groups)?;
                if !params.include_support_signals {
                    strip_support_signals(&mut value);
                }
                obj.insert("parent_groups".to_string(), value);
            }
            if let Some(explain) = explain_payload {
                obj.insert("explain".to_string(), serde_json::to_value(explain)?);
            }
        }
        Ok(response_json)
    }

    pub(super) async fn fuse_indicator_rows(
        &self,
        params: &SearchIndicatorParams,
        persona: SearchPersona,
        query_has_syntax: bool,
        mut rows: Vec<IndicatorSearchRow>,
        deadline: SearchDeadline,
    ) -> Result<Vec<IndicatorSearchRow>> {
        let fetch_limit = rows.len().max(1);
        let search_params = SearchParams {
            query: params.query.clone(),
            resource_types: params.resource_types.clone(),
            persona: Some(persona.as_str().to_string()),
            detail: Some(DetailLevel::Standard),
            pagination: crate::params::PaginationParams {
                limit: Some(fetch_limit),
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

    pub(super) async fn search_indicator_rows_blocking(
        &self,
        tokens: Vec<String>,
        resource_filter: Option<HashSet<String>>,
        indicator_filter: Option<HashSet<String>>,
        include_explain: bool,
        deadline: SearchDeadline,
    ) -> Result<Vec<IndicatorSearchRow>> {
        let entities = Arc::clone(&self.entities);
        let search_config = self.config.search.clone();
        tokio::task::spawn_blocking(move || {
            Self::search_indicator_rows_from_store(
                &entities,
                &search_config,
                &tokens,
                resource_filter.as_ref(),
                indicator_filter.as_ref(),
                include_explain,
                deadline,
            )
        })
        .await
        .map_err(|err| {
            DbtNovaError::ServerError(format!("join failure while scanning indicators: {err}"))
        })?
    }

    pub(super) fn search_indicator_rows_from_store(
        entities: &EntityStore,
        search_config: &SearchConfig,
        tokens: &[String],
        resource_filter: Option<&HashSet<String>>,
        indicator_filter: Option<&HashSet<String>>,
        include_explain: bool,
        deadline: SearchDeadline,
    ) -> Result<Vec<IndicatorSearchRow>> {
        let min_word_len = search_config.min_word_length.max(1);
        let token_set: HashSet<&str> = tokens.iter().map(String::as_str).collect();
        let query_token_count = token_set.len();
        let mut rows: Vec<IndicatorSearchRow> = Vec::new();
        let mut scanned = 0usize;

        for unique_id in entities.ids() {
            check_scan_deadline(deadline, &mut scanned)?;
            let Some(entity) = entities.get_archived(unique_id)? else {
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
                &search_config.metadata_support,
            );
            let context = IndicatorSearchContext {
                unique_id,
                entity,
                nova,
                include_explain,
                token_set: &token_set,
                query_token_count,
                min_word_len,
                support_signals,
                indicator_config: &search_config.indicator_ranking,
                metadata_config: &search_config.metadata_support,
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

    pub(super) async fn indicator_inventory_rows_blocking(
        &self,
        resource_filter: Option<HashSet<String>>,
        indicator_filter: Option<HashSet<String>>,
        canonical_only: bool,
        deadline: SearchDeadline,
    ) -> Result<Vec<IndicatorInventoryRow>> {
        let entities = Arc::clone(&self.entities);
        tokio::task::spawn_blocking(move || {
            Self::indicator_inventory_rows_from_store(
                &entities,
                resource_filter.as_ref(),
                indicator_filter.as_ref(),
                canonical_only,
                deadline,
            )
        })
        .await
        .map_err(|err| {
            DbtNovaError::ServerError(format!(
                "join failure while scanning indicator inventory: {err}"
            ))
        })?
    }

    pub(super) fn indicator_inventory_rows_from_store(
        entities: &EntityStore,
        resource_filter: Option<&HashSet<String>>,
        indicator_filter: Option<&HashSet<String>>,
        canonical_only: bool,
        deadline: SearchDeadline,
    ) -> Result<Vec<IndicatorInventoryRow>> {
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
}
pub(super) fn build_measure_indicator_row(
    context: &IndicatorSearchContext<'_>,
    measure: &ArchivedNovaMeasure,
    matched: &crate::manifest::search::SemanticPreviewItem,
) -> IndicatorSearchRow {
    let explain_payload = indicator_match_explain(context, measure.name.as_str(), matched);
    let execution = indicator_execution_metadata(context.entity);
    IndicatorSearchRow {
        indicator_name: measure.name.as_str().to_string(),
        indicator_type: "measure".to_string(),
        canonical: matched.canonical,
        match_type: matched.match_type.as_str().to_string(),
        score: explain_payload.final_score,
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
        execution,
        domains: context
            .nova
            .domains
            .iter()
            .map(|value| value.as_str().to_string())
            .collect(),
        grain: grain_summary(context.nova.grain.as_ref()),
        support_signals: context.support_signals.clone(),
        explain: context.include_explain.then_some(explain_payload),
    }
}

pub(super) fn build_measure_inventory_row(
    unique_id: &str,
    entity: &ArchivedEntity,
    nova: &ArchivedNovaMeta,
    measure: &ArchivedNovaMeasure,
    canonical: bool,
) -> IndicatorInventoryRow {
    let execution = indicator_execution_metadata(entity);
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
        execution,
        domains: nova
            .domains
            .iter()
            .map(|value| value.as_str().to_string())
            .collect(),
        grain: grain_summary(nova.grain.as_ref()),
    }
}

pub(super) fn build_metric_indicator_row(
    context: &IndicatorSearchContext<'_>,
    metric: &ArchivedNovaMetric,
    matched: &crate::manifest::search::SemanticPreviewItem,
) -> IndicatorSearchRow {
    let preferred_grain = metric.grain.as_ref().or(context.nova.grain.as_ref());
    let explain_payload =
        indicator_match_explain_with_grain(context, metric.name.as_str(), matched, preferred_grain);
    let execution = indicator_execution_metadata(context.entity);
    IndicatorSearchRow {
        indicator_name: metric.name.as_str().to_string(),
        indicator_type: "metric".to_string(),
        canonical: matched.canonical,
        match_type: matched.match_type.as_str().to_string(),
        score: explain_payload.final_score,
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
        execution,
        domains: context
            .nova
            .domains
            .iter()
            .map(|value| value.as_str().to_string())
            .collect(),
        grain: grain_summary(preferred_grain),
        support_signals: context.support_signals.clone(),
        explain: context.include_explain.then_some(explain_payload),
    }
}

pub(super) fn build_metric_inventory_row(
    unique_id: &str,
    entity: &ArchivedEntity,
    nova: &ArchivedNovaMeta,
    metric: &ArchivedNovaMetric,
    canonical: bool,
) -> IndicatorInventoryRow {
    let execution = indicator_execution_metadata(entity);
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
        execution,
        domains: nova
            .domains
            .iter()
            .map(|value| value.as_str().to_string())
            .collect(),
        grain: grain_summary(metric.grain.as_ref().or(nova.grain.as_ref())),
    }
}

pub(super) fn indicator_execution_metadata(entity: &ArchivedEntity) -> IndicatorExecutionMetadata {
    match entity.resource_type_str() {
        Some("metric") => semantic_layer_indicator_execution("dbt_metric"),
        Some("semantic_model") => semantic_layer_indicator_execution("dbt_semantic_model"),
        _ if entity.relation_name_str().is_some() => IndicatorExecutionMetadata {
            indicator_source: "nova_meta",
            execution_surface: "relation",
            queryable: true,
            direct_sql_queryable: true,
            queryable_via: "relation_name",
            execution_note: None,
        },
        _ => IndicatorExecutionMetadata {
            indicator_source: "nova_meta",
            execution_surface: "metadata_only",
            queryable: false,
            direct_sql_queryable: false,
            queryable_via: "none",
            execution_note: Some(
                "No deterministic relation or Semantic Layer execution surface is available.",
            ),
        },
    }
}

pub(super) fn semantic_layer_indicator_execution(
    source: &'static str,
) -> IndicatorExecutionMetadata {
    IndicatorExecutionMetadata {
        indicator_source: source,
        execution_surface: "semantic_layer",
        queryable: true,
        direct_sql_queryable: false,
        queryable_via: "metricflow",
        execution_note: Some("Use MetricFlow or the dbt Semantic Layer for execution."),
    }
}

pub(super) fn inventory_indicator_is_canonical(
    entity_canonical: bool,
    indicator_canonical: bool,
) -> bool {
    entity_canonical || indicator_canonical
}

pub(super) fn grain_summary(grain: Option<&ArchivedNovaGrain>) -> IndicatorGrainSummary {
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

pub(super) fn indicator_match_explain(
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

pub(super) fn indicator_match_explain_with_grain(
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

pub(super) fn generic_indicator_label_match_bonus(
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

pub(super) fn parent_synonym_match_bonus(
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

pub(super) fn fully_covered_token_count(
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

pub(super) fn metadata_support_bonus(
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

pub(super) fn semantic_label_precision_bonus(
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

pub(super) fn apply_parent_coherence_bonus(
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

pub(super) fn token_overlap_count(
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

pub(super) fn collect_metadata_support_signals(
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

pub(super) fn collect_matching_values<'a>(
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

pub(super) fn build_search_explain_payload(
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

pub(super) fn indicator_retrievers_used(
    rows: &[IndicatorSearchRow],
    rrf_enabled: bool,
) -> Vec<String> {
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

pub(super) fn strip_sql_fields(value: &mut JsonValue) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };
    obj.remove("raw_code");
    obj.remove("compiled_code");
}

pub(super) fn normalized_indicator_type_filter(
    indicator_types: &[String],
) -> Result<Option<HashSet<String>>> {
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

pub(super) fn indicator_type_selected(
    indicator_filter: Option<&HashSet<String>>,
    indicator_type: &str,
) -> bool {
    indicator_filter.is_none_or(|values| values.contains(indicator_type))
}

pub(super) fn dedupe_indicator_parent_ids(
    rows: &[IndicatorSearchRow],
    limit: usize,
) -> Vec<String> {
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

pub(super) fn normalized_indicator_parent_scores(
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

pub(super) fn indicator_embedding_text(row: &IndicatorSearchRow) -> String {
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

pub(super) fn reorder_indicator_rows_with_reranker(
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

pub(super) fn build_indicator_parent_groups(
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

pub(super) fn filter_indicator_parent_groups(
    mut groups: Vec<IndicatorParentGroup>,
    params: &SearchIndicatorParams,
) -> Vec<IndicatorParentGroup> {
    let max_groups = match params.group_mode.unwrap_or_default() {
        ParentGroupMode::None => 0,
        ParentGroupMode::Top => params.max_parent_groups.unwrap_or(1).min(1),
        ParentGroupMode::All => params.max_parent_groups.unwrap_or(groups.len()),
    };
    groups.truncate(max_groups);
    groups
}

pub(super) fn indicator_row_value(
    row: IndicatorSearchRow,
    detail: DetailLevel,
    include_support_signals: bool,
) -> JsonValue {
    let mut value = serde_json::to_value(row).unwrap_or(JsonValue::Null);
    if let Some(obj) = value.as_object_mut() {
        if detail == DetailLevel::Compact {
            obj.remove("description");
            obj.remove("explain");
        }
        if !include_support_signals {
            obj.remove("support_signals");
        }
    }
    value
}

pub(super) fn strip_support_signals(value: &mut JsonValue) {
    match value {
        JsonValue::Object(obj) => {
            obj.remove("support_signals");
            for child in obj.values_mut() {
                strip_support_signals(child);
            }
        }
        JsonValue::Array(items) => {
            for item in items {
                strip_support_signals(item);
            }
        }
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::String(_) => {}
    }
}

pub(super) fn indicator_row_key(row: &IndicatorSearchRow) -> String {
    format!(
        "{}::{}::{}",
        row.parent_unique_id, row.indicator_type, row.indicator_name
    )
}

pub(super) fn expand_indicator_parent_ranking(
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
