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

impl ManifestSearch {
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

        let mut allowed: Option<HashSet<String>> = None;

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

        let detail = self.detail_level(params.detail);
        let limit = self.page_limit(params.pagination.limit);
        let mut total = 0usize;
        let mut skipped = 0usize;
        let mut results: Vec<JsonValue> = Vec::with_capacity(limit);

        for id in candidates {
            let allowed_match = allowed
                .as_ref()
                .is_none_or(|allowed_set| allowed_set.contains(id));
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

            if let Some(entity) = self.get_entity_archived(id)? {
                match detail {
                    DetailLevel::Full => {
                        results.push(ManifestSearch::with_unique_id(entity.to_json_value(), id));
                    }
                    DetailLevel::Compact => {
                        results.push(self.summary_for_compact(id, entity));
                    }
                    DetailLevel::Standard => {
                        results.push(self.summary_for_standard(id, entity));
                    }
                }
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
