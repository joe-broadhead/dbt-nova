use super::{
    AgentModellingContext, AgentModellingFinding, AgentModellingSummaryInput, ArchivedEntity,
    ArchivedNovaGrain, ArchivedNovaMeta, ArchivedNovaMetric, ArchivedString, BTreeMap, BTreeSet,
    CandidatePairs, DuplicateIndicatorParent, DuplicateIndicatorRow, EntityOverlapEvidence,
    EntityOverlapProfile, EntityOverlapRow, GrainComparison, GrainVariant, HashMap,
    IndicatorOverlapIndicatorProfile, JsonValue, MAX_OVERLAP_BUCKET_SIZE,
    MAX_OVERLAP_CANDIDATE_PAIRS, ManifestSearch, ModellingConsistencyReportParams,
    ModellingReportPage, MultiGrainEntityRow, OverlapRowsResult, Result, agent_modelling_summary,
    best_grain_pair, collect_column_semantic_ambiguity_findings,
    collect_duplicate_indicator_findings, collect_entity_agent_modelling_findings,
    collect_multi_grain_entity_findings, collect_semantic_label_collision_findings,
    compare_duplicate_indicator_rows, compare_overlap_rows, duplicate_indicator_summary_row,
    grain_signature_key, is_distinctive_column_name, json, modelling_drill_down_hints,
    modelling_has_next_page, normalize_value, overlap_evidence_categories_for_row,
    overlap_evidence_category_counts, score_from_overlap, semantic_metric_names,
    semantic_model_measure_names, sorted_difference, sorted_intersection, tokenize_alnum_lowercase,
};

pub(super) fn build_entity_profile(
    unique_id: &str,
    entity: &ArchivedEntity,
    min_word_len: usize,
) -> EntityOverlapProfile {
    let nova = entity.nova_meta();
    let mut name_tokens = BTreeSet::new();
    if let Some(name) = entity.name_str() {
        name_tokens.extend(tokenize_alnum_lowercase(name, min_word_len));
    }
    if let Some(alias) = entity.alias_str() {
        name_tokens.extend(tokenize_alnum_lowercase(alias, min_word_len));
    }

    let parent_synonyms = nova.map_or_else(BTreeSet::new, |nova| {
        nova.synonyms
            .iter()
            .map(ArchivedString::as_str)
            .map(normalize_value)
            .filter(|value| !value.is_empty())
            .collect()
    });
    let column_names = entity
        .column_names_iter()
        .map(normalize_value)
        .filter(|value| is_distinctive_column_name(value, min_word_len))
        .collect();
    let domains = nova.map_or_else(BTreeSet::new, |nova| {
        nova.domains
            .iter()
            .map(ArchivedString::as_str)
            .map(normalize_value)
            .filter(|value| !value.is_empty())
            .collect()
    });

    let mut indicator_names = BTreeSet::new();
    let mut typed_indicators = BTreeSet::new();
    let entity_canonical = nova.is_some_and(|nova| nova.canonical);
    let entity_grain_variants = build_entity_grain_variants(nova);
    let indicator_profiles = build_indicator_profiles(
        nova,
        entity_canonical,
        &entity_grain_variants,
        &mut indicator_names,
        &mut typed_indicators,
    );

    let column_semantic_types = entity
        .column_meta()
        .iter()
        .filter_map(|column| column.semantic_type.as_ref().map(ArchivedString::as_str))
        .map(normalize_value)
        .filter(|value| !value.is_empty())
        .collect();

    EntityOverlapProfile {
        unique_id: unique_id.to_string(),
        name: entity.name_str().unwrap_or(unique_id).to_string(),
        resource_type: entity.resource_type_str().unwrap_or("unknown").to_string(),
        relation_name: entity.relation_name_str().map(str::to_string),
        canonical: nova.is_some_and(|nova| nova.canonical),
        name_tokens,
        column_names,
        parent_synonyms,
        domains,
        indicator_names,
        typed_indicators,
        indicator_profiles,
        column_semantic_types,
        grain_variants: build_grain_variants(nova),
    }
}

pub(super) fn build_indicator_profiles(
    nova: Option<&ArchivedNovaMeta>,
    entity_canonical: bool,
    entity_grain_variants: &[GrainVariant],
    indicator_names: &mut BTreeSet<String>,
    typed_indicators: &mut BTreeSet<(String, String)>,
) -> BTreeMap<(String, String), IndicatorOverlapIndicatorProfile> {
    let mut indicator_profiles = BTreeMap::new();
    let Some(nova) = nova else {
        return indicator_profiles;
    };

    for measure in nova.measures.iter() {
        let name = normalize_value(measure.name.as_str());
        if name.is_empty() {
            continue;
        }
        indicator_names.insert(name.clone());
        let key = ("measure".to_string(), name);
        typed_indicators.insert(key.clone());
        indicator_profiles.insert(
            key,
            IndicatorOverlapIndicatorProfile {
                canonical: entity_canonical || measure.canonical,
                grain_variants: entity_grain_variants.to_vec(),
            },
        );
    }

    if let Some(metric) = nova.metric.as_ref() {
        insert_metric_indicator_profile(
            metric,
            entity_canonical,
            entity_grain_variants,
            indicator_names,
            typed_indicators,
            &mut indicator_profiles,
        );
    }
    for metric in nova.metrics.iter() {
        insert_metric_indicator_profile(
            metric,
            entity_canonical,
            entity_grain_variants,
            indicator_names,
            typed_indicators,
            &mut indicator_profiles,
        );
    }

    indicator_profiles
}

pub(super) fn insert_metric_indicator_profile(
    metric: &ArchivedNovaMetric,
    entity_canonical: bool,
    entity_grain_variants: &[GrainVariant],
    indicator_names: &mut BTreeSet<String>,
    typed_indicators: &mut BTreeSet<(String, String)>,
    indicator_profiles: &mut BTreeMap<(String, String), IndicatorOverlapIndicatorProfile>,
) {
    let name = normalize_value(metric.name.as_str());
    if name.is_empty() {
        return;
    }
    indicator_names.insert(name.clone());
    let key = ("metric".to_string(), name);
    typed_indicators.insert(key.clone());
    indicator_profiles.insert(
        key,
        IndicatorOverlapIndicatorProfile {
            canonical: entity_canonical || metric.canonical,
            grain_variants: metric_grain_variants(metric, entity_grain_variants),
        },
    );
}

pub(super) fn build_entity_grain_variants(nova: Option<&ArchivedNovaMeta>) -> Vec<GrainVariant> {
    nova.and_then(|nova| nova.grain.as_ref())
        .map(|grain| vec![grain_variant_from_archived("entity".to_string(), grain)])
        .unwrap_or_default()
}

pub(super) fn metric_grain_variants(
    metric: &ArchivedNovaMetric,
    entity_grain_variants: &[GrainVariant],
) -> Vec<GrainVariant> {
    metric.grain.as_ref().map_or_else(
        || entity_grain_variants.to_vec(),
        |grain| {
            vec![grain_variant_from_archived(
                metric.name.as_str().to_string(),
                grain,
            )]
        },
    )
}

pub(super) fn grain_variant_from_archived(
    source: String,
    grain: &ArchivedNovaGrain,
) -> GrainVariant {
    GrainVariant {
        sources: vec![source],
        primary_key: grain
            .primary_key
            .iter()
            .map(ArchivedString::as_str)
            .map(str::to_string)
            .collect(),
        time_field: grain
            .time_field
            .as_ref()
            .map(ArchivedString::as_str)
            .map(str::to_string),
        dimensions: grain
            .dimensions
            .iter()
            .map(ArchivedString::as_str)
            .map(str::to_string)
            .collect(),
    }
}

pub(super) fn build_grain_variants(nova: Option<&ArchivedNovaMeta>) -> Vec<GrainVariant> {
    let Some(nova) = nova else {
        return Vec::new();
    };

    let mut variants: Vec<GrainVariant> = Vec::new();
    let mut positions: HashMap<String, usize> = HashMap::new();

    let mut push_variant = |source: String, grain: &ArchivedNovaGrain| {
        let candidate = grain_variant_from_archived(source, grain);
        let key = grain_signature_key(&candidate);
        if let Some(index) = positions.get(&key).copied() {
            if !variants[index]
                .sources
                .iter()
                .any(|existing| existing == &candidate.sources[0])
            {
                variants[index].sources.push(candidate.sources[0].clone());
            }
        } else {
            positions.insert(key, variants.len());
            variants.push(candidate);
        }
    };

    if let Some(grain) = nova.grain.as_ref() {
        push_variant("entity".to_string(), grain);
    }
    if let Some(metric) = nova.metric.as_ref()
        && let Some(grain) = metric.grain.as_ref()
    {
        push_variant(format!("metric:{}", metric.name.as_str()), grain);
    }
    for metric in nova.metrics.iter() {
        if let Some(grain) = metric.grain.as_ref() {
            push_variant(format!("metric:{}", metric.name.as_str()), grain);
        }
    }

    variants
}

pub(super) fn compare_entity_grains(
    left: &EntityOverlapProfile,
    right: &EntityOverlapProfile,
) -> GrainComparison {
    let best_pair = best_grain_pair(left, right);
    let left_grain = best_pair.map(|(left_grain, _)| left_grain);
    let right_grain = best_pair.map(|(_, right_grain)| right_grain);

    let left_pk = left_grain.map_or_else(BTreeSet::new, |grain| {
        grain.primary_key.iter().cloned().collect()
    });
    let right_pk = right_grain.map_or_else(BTreeSet::new, |grain| {
        grain.primary_key.iter().cloned().collect()
    });
    let left_dims = left_grain.map_or_else(BTreeSet::new, |grain| {
        grain.dimensions.iter().cloned().collect()
    });
    let right_dims = right_grain.map_or_else(BTreeSet::new, |grain| {
        grain.dimensions.iter().cloned().collect()
    });
    let shared_primary_key = sorted_intersection(&left_pk, &right_pk);
    let left_only_primary_key = sorted_difference(&left_pk, &right_pk);
    let right_only_primary_key = sorted_difference(&right_pk, &left_pk);
    let shared_dimensions = sorted_intersection(&left_dims, &right_dims);
    let left_only_dimensions = sorted_difference(&left_dims, &right_dims);
    let right_only_dimensions = sorted_difference(&right_dims, &left_dims);
    let same_time_field = left_grain
        .and_then(|grain| grain.time_field.as_deref())
        .zip(right_grain.and_then(|grain| grain.time_field.as_deref()))
        .is_some_and(|(left_time, right_time)| left_time == right_time);
    let exact_match = left_grain
        .zip(right_grain)
        .is_some_and(|(left_grain, right_grain)| {
            grain_signature_key(left_grain) == grain_signature_key(right_grain)
        });

    GrainComparison {
        entity1: left.entity_ref(),
        entity2: right.entity_ref(),
        entity1_grain_variants: left.grain_variants.clone(),
        entity2_grain_variants: right.grain_variants.clone(),
        exact_match,
        same_time_field,
        shared_primary_key,
        entity1_only_primary_key: left_only_primary_key,
        entity2_only_primary_key: right_only_primary_key,
        shared_dimensions,
        entity1_only_dimensions: left_only_dimensions,
        entity2_only_dimensions: right_only_dimensions,
    }
}

pub(super) fn overlap_rows(
    profiles: &[EntityOverlapProfile],
    focus_unique_id: Option<&str>,
) -> OverlapRowsResult {
    let focus_index = focus_unique_id
        .and_then(|unique_id| profiles.iter().position(|p| p.unique_id == unique_id));
    let candidate_pairs = overlap_candidate_pairs(profiles, focus_index);
    let mut rows = Vec::new();

    for (left_index, right_index) in candidate_pairs.pairs {
        let left = &profiles[left_index];
        let right = &profiles[right_index];
        let evidence = overlap_evidence(left, right);
        let surface_overlap_count = evidence.surface_overlap_count();
        if surface_overlap_count == 0 {
            continue;
        }
        let shared_value_count = evidence.shared_value_count();
        let score = score_from_overlap(surface_overlap_count, shared_value_count);
        rows.push(EntityOverlapRow {
            entity1: left.entity_ref(),
            entity2: right.entity_ref(),
            score,
            surface_overlap_count,
            shared_value_count,
            evidence,
        });
    }

    rows.sort_by(compare_overlap_rows);
    OverlapRowsResult {
        rows,
        candidate_pairs_truncated: candidate_pairs.truncated,
    }
}

pub(super) fn overlap_candidate_pairs(
    profiles: &[EntityOverlapProfile],
    focus_index: Option<usize>,
) -> CandidatePairs {
    let mut buckets: BTreeMap<String, Vec<usize>> = BTreeMap::new();

    for (index, profile) in profiles.iter().enumerate() {
        for key in overlap_bucket_keys(profile) {
            buckets.entry(key).or_default().push(index);
        }
    }

    let mut pairs = BTreeSet::new();
    let mut truncated = false;
    for mut indices in buckets.into_values() {
        indices.sort_unstable();

        if let Some(focus_index) = focus_index {
            if !indices.contains(&focus_index) {
                continue;
            }
            if indices.len() > MAX_OVERLAP_BUCKET_SIZE {
                truncated = true;
            }
            for other_index in indices
                .into_iter()
                .filter(|index| *index != focus_index)
                .take(MAX_OVERLAP_BUCKET_SIZE.saturating_sub(1))
            {
                pairs.insert(ordered_pair(focus_index, other_index));
                if pairs.len() >= MAX_OVERLAP_CANDIDATE_PAIRS {
                    return CandidatePairs {
                        pairs,
                        truncated: true,
                    };
                }
            }
            continue;
        }

        if indices.len() > MAX_OVERLAP_BUCKET_SIZE {
            indices.truncate(MAX_OVERLAP_BUCKET_SIZE);
            truncated = true;
        }

        for (offset, left_index) in indices.iter().enumerate() {
            for right_index in indices.iter().skip(offset + 1) {
                pairs.insert((*left_index, *right_index));
                if pairs.len() >= MAX_OVERLAP_CANDIDATE_PAIRS {
                    return CandidatePairs {
                        pairs,
                        truncated: true,
                    };
                }
            }
        }
    }
    CandidatePairs { pairs, truncated }
}

pub(super) fn ordered_pair(left: usize, right: usize) -> (usize, usize) {
    if left <= right {
        (left, right)
    } else {
        (right, left)
    }
}

pub(super) fn overlap_bucket_keys(profile: &EntityOverlapProfile) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    for token in &profile.name_tokens {
        keys.insert(format!("tok:{token}"));
    }
    for synonym in &profile.parent_synonyms {
        keys.insert(format!("syn:{synonym}"));
    }
    for domain in &profile.domains {
        keys.insert(format!("dom:{domain}"));
    }
    for indicator in &profile.indicator_names {
        keys.insert(format!("ind:{indicator}"));
    }
    for column_name in &profile.column_names {
        keys.insert(format!("col:{column_name}"));
    }
    for semantic_type in &profile.column_semantic_types {
        keys.insert(format!("stype:{semantic_type}"));
    }
    for grain in &profile.grain_variants {
        if let Some(time_field) = &grain.time_field {
            keys.insert(format!("time:{}", normalize_value(time_field)));
        }
        for dimension in &grain.dimensions {
            keys.insert(format!("dim:{}", normalize_value(dimension)));
        }
    }
    keys
}

pub(super) fn overlap_evidence(
    left: &EntityOverlapProfile,
    right: &EntityOverlapProfile,
) -> EntityOverlapEvidence {
    let best_pair = best_grain_pair(left, right);
    let shared_time_field = best_pair
        .and_then(|(left_grain, right_grain)| {
            left_grain
                .time_field
                .as_deref()
                .zip(right_grain.time_field.as_deref())
        })
        .filter(|(left_time, right_time)| left_time == right_time)
        .map(|(time_field, _)| time_field.to_string());
    let shared_dimensions = best_pair.map_or_else(Vec::new, |(left_grain, right_grain)| {
        sorted_intersection(
            &left_grain.dimensions.iter().cloned().collect(),
            &right_grain.dimensions.iter().cloned().collect(),
        )
    });

    EntityOverlapEvidence {
        shared_name_tokens: sorted_intersection(&left.name_tokens, &right.name_tokens),
        shared_column_names: sorted_intersection(&left.column_names, &right.column_names),
        shared_parent_synonyms: sorted_intersection(&left.parent_synonyms, &right.parent_synonyms),
        shared_domains: sorted_intersection(&left.domains, &right.domains),
        shared_indicators: sorted_intersection(&left.indicator_names, &right.indicator_names),
        shared_column_semantic_types: sorted_intersection(
            &left.column_semantic_types,
            &right.column_semantic_types,
        ),
        shared_dimensions,
        shared_time_field,
    }
}

impl EntityOverlapEvidence {
    pub(super) fn surface_overlap_count(&self) -> usize {
        usize::from(!self.shared_name_tokens.is_empty())
            + usize::from(!self.shared_column_names.is_empty())
            + usize::from(!self.shared_parent_synonyms.is_empty())
            + usize::from(!self.shared_domains.is_empty())
            + usize::from(!self.shared_indicators.is_empty())
            + usize::from(!self.shared_column_semantic_types.is_empty())
            + usize::from(!self.shared_dimensions.is_empty())
            + usize::from(self.shared_time_field.is_some())
    }

    pub(super) fn shared_value_count(&self) -> usize {
        self.shared_name_tokens.len()
            + self.shared_column_names.len()
            + self.shared_parent_synonyms.len()
            + self.shared_domains.len()
            + self.shared_indicators.len()
            + self.shared_column_semantic_types.len()
            + self.shared_dimensions.len()
            + usize::from(self.shared_time_field.is_some())
    }
}

pub(super) fn duplicate_indicator_rows(
    profiles: &[EntityOverlapProfile],
    limit: usize,
) -> Vec<DuplicateIndicatorRow> {
    let mut by_indicator: BTreeMap<(String, String), Vec<&EntityOverlapProfile>> = BTreeMap::new();
    for profile in profiles {
        for (indicator_type, indicator_name) in &profile.typed_indicators {
            by_indicator
                .entry((indicator_type.clone(), indicator_name.clone()))
                .or_default()
                .push(profile);
        }
    }

    let mut rows = Vec::new();
    for ((indicator_type, indicator_name), parents) in by_indicator {
        if parents.len() < 2 {
            continue;
        }
        let indicator_key = (indicator_type.clone(), indicator_name.clone());
        let canonical_parent_count = parents
            .iter()
            .filter(|profile| {
                profile
                    .indicator_profiles
                    .get(&indicator_key)
                    .is_some_and(|details| details.canonical)
            })
            .count();
        let parents_without_grain = parents
            .iter()
            .filter(|profile| {
                profile
                    .indicator_profiles
                    .get(&indicator_key)
                    .is_none_or(|details| details.grain_variants.is_empty())
            })
            .count();
        let mut grain_signatures: BTreeMap<String, GrainVariant> = BTreeMap::new();
        for profile in &parents {
            let Some(details) = profile.indicator_profiles.get(&indicator_key) else {
                continue;
            };
            for grain in &details.grain_variants {
                grain_signatures
                    .entry(grain_signature_key(grain))
                    .or_insert_with(|| grain.clone());
            }
        }
        let inconsistent_grains = grain_signatures.len() > 1
            || (parents_without_grain > 0 && parents_without_grain < parents.len());
        rows.push(DuplicateIndicatorRow {
            indicator_name,
            indicator_type,
            parent_count: parents.len(),
            canonical_parent_count,
            parents_without_grain,
            inconsistent_grains,
            parents: parents
                .iter()
                .map(|profile| DuplicateIndicatorParent {
                    unique_id: profile.unique_id.clone(),
                    name: profile.name.clone(),
                    resource_type: profile.resource_type.clone(),
                    relation_name: profile.relation_name.clone(),
                    canonical: profile
                        .indicator_profiles
                        .get(&indicator_key)
                        .is_some_and(|details| details.canonical),
                })
                .collect(),
            grain_signatures: grain_signatures.into_values().collect(),
        });
    }

    rows.sort_by(compare_duplicate_indicator_rows);
    rows.truncate(limit);
    rows
}

pub(super) fn multi_grain_entity_rows(
    profiles: &[EntityOverlapProfile],
) -> Vec<MultiGrainEntityRow> {
    let mut rows: Vec<MultiGrainEntityRow> = profiles
        .iter()
        .filter(|profile| profile.grain_variants.len() > 1)
        .map(|profile| MultiGrainEntityRow {
            entity: profile.entity_ref(),
            grain_variant_count: profile.grain_variants.len(),
            grain_variants: profile.grain_variants.clone(),
        })
        .collect();
    rows.sort_by(|left, right| {
        right
            .grain_variant_count
            .cmp(&left.grain_variant_count)
            .then_with(|| left.entity.unique_id.cmp(&right.entity.unique_id))
    });
    rows
}

pub(super) fn build_modelling_consistency_summary(
    params: &ModellingConsistencyReportParams,
    page: ModellingReportPage,
    overlap_rows: &[EntityOverlapRow],
    duplicate_indicator_rows: &[DuplicateIndicatorRow],
    canonical_conflict_rows: &[DuplicateIndicatorRow],
    multi_grain_entity_rows: &[MultiGrainEntityRow],
    agent_modelling: AgentModellingSummaryInput<'_>,
) -> JsonValue {
    let overlap_evidence_categories = overlap_evidence_category_counts(overlap_rows);
    let overlap_examples = overlap_rows
        .iter()
        .take(5)
        .map(|row| {
            json!({
                "entity1": &row.entity1.unique_id,
                "entity2": &row.entity2.unique_id,
                "score": row.score,
                "surface_overlap_count": row.surface_overlap_count,
                "evidence_categories": overlap_evidence_categories_for_row(row),
                "shared_column_examples": row.evidence.shared_column_names.iter().take(5).collect::<Vec<_>>(),
                "shared_name_token_examples": row.evidence.shared_name_tokens.iter().take(5).collect::<Vec<_>>(),
                "shared_indicator_examples": row.evidence.shared_indicators.iter().take(5).collect::<Vec<_>>(),
                "shared_domain_examples": row.evidence.shared_domains.iter().take(5).collect::<Vec<_>>(),
                "shared_time_field": row.evidence.shared_time_field
            })
        })
        .collect::<Vec<_>>();
    let top_duplicate_indicator_groups = duplicate_indicator_rows
        .iter()
        .take(5)
        .map(duplicate_indicator_summary_row)
        .collect::<Vec<_>>();
    let top_canonical_conflicts = canonical_conflict_rows
        .iter()
        .take(5)
        .map(duplicate_indicator_summary_row)
        .collect::<Vec<_>>();
    let top_multi_grain_entities = multi_grain_entity_rows
        .iter()
        .take(5)
        .map(|row| {
            json!({
                "unique_id": &row.entity.unique_id,
                "name": &row.entity.name,
                "resource_type": &row.entity.resource_type,
                "grain_variant_count": row.grain_variant_count,
                "variant_sources": row.grain_variants.iter().flat_map(|variant| variant.sources.iter()).take(8).collect::<Vec<_>>()
            })
        })
        .collect::<Vec<_>>();
    let next_offset = modelling_has_next_page(
        page.limit,
        page.offset,
        overlap_rows,
        duplicate_indicator_rows,
        canonical_conflict_rows,
        multi_grain_entity_rows,
    )
    .then_some(page.offset.saturating_add(page.limit));

    json!({
        "section_counts": {
            "overlap_candidates": overlap_rows.len(),
            "duplicate_indicators": duplicate_indicator_rows.len(),
            "canonical_indicator_conflicts": canonical_conflict_rows.len(),
            "entities_with_multiple_grain_variants": multi_grain_entity_rows.len(),
            "agent_modelling_findings": agent_modelling.findings.len()
        },
        "agent_modelling": agent_modelling_summary(
            agent_modelling.findings,
            agent_modelling.truncated
        ),
        "page": {
            "limit": page.limit,
            "offset": page.offset,
            "next_offset": next_offset,
            "overlap_candidate_generation_truncated": page.overlap_candidate_generation_truncated
        },
        "overlap_evidence_categories": overlap_evidence_categories,
        "overlap_examples": overlap_examples,
        "top_duplicate_indicator_groups": top_duplicate_indicator_groups,
        "top_canonical_conflicts": top_canonical_conflicts,
        "top_multi_grain_entities": top_multi_grain_entities,
        "drill_down_hints": modelling_drill_down_hints(params, page.limit, page.offset, overlap_rows, duplicate_indicator_rows, canonical_conflict_rows, multi_grain_entity_rows)
    })
}

pub(super) fn build_agent_modelling_findings(
    search: &ManifestSearch,
    profiles: &[EntityOverlapProfile],
    duplicate_indicator_rows: &[DuplicateIndicatorRow],
    multi_grain_entity_rows: &[MultiGrainEntityRow],
) -> Result<Vec<AgentModellingFinding>> {
    let mut findings = Vec::new();
    let context = AgentModellingContext {
        search,
        semantic_model_measure_names: semantic_model_measure_names(&search.entities)?,
        semantic_metric_names: semantic_metric_names(&search.entities)?,
    };
    collect_duplicate_indicator_findings(duplicate_indicator_rows, &mut findings);
    collect_semantic_label_collision_findings(&context, profiles, &mut findings)?;
    collect_column_semantic_ambiguity_findings(&context, profiles, &mut findings)?;
    collect_multi_grain_entity_findings(&search.entities, multi_grain_entity_rows, &mut findings)?;
    collect_entity_agent_modelling_findings(&context, profiles, &mut findings)?;
    Ok(findings)
}
