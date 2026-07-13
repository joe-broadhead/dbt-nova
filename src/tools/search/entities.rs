use crate::manifest::entity::{ArchivedEntity, ArchivedNovaMeta};

use super::{
    DbtNovaError, DetailLevel, HashSet, JsonValue, ListEntitiesParams, ManifestSearch, Result,
    SuccessResponse,
};

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

fn normalized_matches(value: &str, expected: &str) -> bool {
    value.trim().eq_ignore_ascii_case(expected.trim())
}

fn matches_any_filter(value: Option<&str>, expected: &[String]) -> bool {
    expected.is_empty()
        || value.is_some_and(|actual| {
            expected
                .iter()
                .any(|candidate| normalized_matches(actual, candidate))
        })
}

fn matches_all_compliance(nova: Option<&ArchivedNovaMeta>, expected: &[String]) -> bool {
    expected.is_empty()
        || nova
            .and_then(|nova| nova.governance.as_ref())
            .is_some_and(|governance| {
                expected.iter().all(|candidate| {
                    governance
                        .compliance
                        .iter()
                        .any(|actual| normalized_matches(actual.as_str(), candidate))
                })
            })
}

fn matches_governance_filter(entity: &ArchivedEntity, params: &ListEntitiesParams) -> bool {
    let Some(filter) = params.governance.as_ref() else {
        return true;
    };
    let nova = entity.nova_meta();
    let governance = nova.and_then(|nova| nova.governance.as_ref());

    if let Some(declared) = filter.declared
        && governance.is_some() != declared
    {
        return false;
    }

    matches_any_filter(
        governance
            .and_then(|governance| governance.pii.as_ref())
            .map(rkyv::string::ArchivedString::as_str),
        &filter.pii,
    ) && matches_any_filter(
        governance
            .and_then(|governance| governance.sensitivity.as_ref())
            .map(rkyv::string::ArchivedString::as_str),
        &filter.sensitivity,
    ) && matches_all_compliance(nova, &filter.compliance_includes)
}

fn matches_tier_filter(entity: &ArchivedEntity, tiers: &[String]) -> bool {
    matches_any_filter(
        entity
            .nova_meta()
            .and_then(|nova| nova.tier.as_ref())
            .map(rkyv::string::ArchivedString::as_str),
        tiers,
    )
}

fn matches_canonical_filter(entity: &ArchivedEntity, canonical: Option<bool>) -> bool {
    canonical
        .is_none_or(|expected| entity.nova_meta().is_some_and(|nova| nova.canonical) == expected)
}

fn matches_nova_filters(entity: &ArchivedEntity, params: &ListEntitiesParams) -> bool {
    matches_governance_filter(entity, params)
        && matches_tier_filter(entity, &params.tier)
        && matches_canonical_filter(entity, params.canonical)
}

fn has_nova_filters(params: &ListEntitiesParams) -> bool {
    params.governance.is_some() || !params.tier.is_empty() || params.canonical.is_some()
}

fn should_collect_row(
    total: &mut usize,
    skipped: &mut usize,
    offset: usize,
    result_count: usize,
    limit: usize,
) -> bool {
    *total += 1;
    if *skipped < offset {
        *skipped += 1;
        return false;
    }
    result_count < limit
}

fn empty_list_entities_response() -> Result<JsonValue> {
    Ok(serde_json::to_value(SuccessResponse::new(
        Vec::<JsonValue>::new(),
        0,
    ))?)
}

impl ManifestSearch {
    fn list_entity_row(&self, id: &str, entity: &ArchivedEntity, detail: DetailLevel) -> JsonValue {
        match detail {
            DetailLevel::Full => ManifestSearch::with_unique_id(entity.to_json_value(), id),
            DetailLevel::Compact => self.summary_for_compact(id, entity),
            DetailLevel::Standard => self.summary_for_standard(id, entity),
        }
    }

    fn list_entities_allowed_ids(&self, params: &ListEntitiesParams) -> Option<HashSet<String>> {
        let mut allowed: Option<HashSet<String>> = None;

        if let Some(ref pkg) = params.package {
            match self.by_package.get(pkg) {
                Some(ids) => apply_vec_filter(&mut allowed, ids),
                None => return Some(HashSet::new()),
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
                    return Some(HashSet::new());
                }
                if let Some(ref mut current) = allowed {
                    current.retain(|id| matches.contains(id));
                } else {
                    allowed = Some(matches);
                }
            }
        }

        for tag in &params.tags {
            match self.by_tag.get(tag) {
                Some(ids) => apply_set_filter(&mut allowed, ids),
                None => return Some(HashSet::new()),
            }
        }

        allowed
    }

    /// List entities by type with optional filtering.
    ///
    /// # Errors
    /// Returns an error if manifest access fails.
    #[allow(clippy::unused_async)]
    #[tracing::instrument(skip(self, params), fields(tool = "list_entities", resource_type = %params.resource_type, limit = ?params.pagination.limit, offset = params.pagination.offset))]
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

        let allowed = self.list_entities_allowed_ids(params);
        if allowed.as_ref().is_some_and(HashSet::is_empty) {
            return empty_list_entities_response();
        }

        let detail = self.detail_level(params.detail);
        let limit = self.page_limit(params.pagination.limit);
        let mut total = 0usize;
        let mut skipped = 0usize;
        let mut results: Vec<JsonValue> = Vec::with_capacity(limit);
        let apply_nova_filters = has_nova_filters(params);

        for id in candidates {
            let allowed_match = allowed
                .as_ref()
                .is_none_or(|allowed_set| allowed_set.contains(id));
            if !allowed_match {
                continue;
            }

            if !apply_nova_filters {
                if !should_collect_row(
                    &mut total,
                    &mut skipped,
                    params.pagination.offset,
                    results.len(),
                    limit,
                ) {
                    continue;
                }

                if let Some(entity) = self.get_entity_archived(id)? {
                    results.push(self.list_entity_row(id, entity, detail));
                }
                continue;
            }

            if let Some(entity) = self.get_entity_archived(id)? {
                if !matches_nova_filters(entity, params) {
                    continue;
                }

                if !should_collect_row(
                    &mut total,
                    &mut skipped,
                    params.pagination.offset,
                    results.len(),
                    limit,
                ) {
                    continue;
                }

                results.push(self.list_entity_row(id, entity, detail));
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
