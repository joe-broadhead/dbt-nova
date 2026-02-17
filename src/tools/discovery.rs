use serde_json::Value as JsonValue;

use crate::error::Result;
use crate::manifest::search::ManifestSearch;
use crate::responses::SuccessResponse;
use crate::tools::helpers::sort_by_json_field;
use tracing::instrument;

impl ManifestSearch {
    /// Return project metadata overview including entity counts by type.
    ///
    /// # Errors
    /// Returns an error if serialization fails or the manifest is unavailable.
    #[instrument(skip(self), fields(tool = "show_metadata"))]
    pub async fn show_metadata(&self) -> Result<JsonValue> {
        let total_entities = self.entity_count();
        let response = serde_json::json!({
            "metadata": self.manifest_metadata,
            "total_entities": total_entities,
            "entity_counts": self.entity_counts,
        });

        Ok(serde_json::to_value(SuccessResponse::new(response, 1))?)
    }

    /// List all unique tags with usage counts.
    ///
    /// # Errors
    /// Returns an error if serialization fails or the manifest is unavailable.
    #[instrument(skip(self), fields(tool = "list_tags"))]
    pub async fn list_tags(&self) -> Result<JsonValue> {
        let mut tags: Vec<JsonValue> = Vec::new();
        for (tag, entities) in &self.by_tag {
            tags.push(serde_json::json!({
                "tag": tag,
                "count": entities.len(),
            }));
        }

        sort_by_json_field(&mut tags, "tag");

        let count = tags.len();
        Ok(serde_json::to_value(SuccessResponse::new(tags, count))?)
    }

    /// List all packages with entity counts.
    ///
    /// # Errors
    /// Returns an error if serialization fails or the manifest is unavailable.
    #[instrument(skip(self), fields(tool = "list_packages"))]
    pub async fn list_packages(&self) -> Result<JsonValue> {
        let mut packages: Vec<JsonValue> = Vec::new();
        for (pkg, entities) in &self.by_package {
            packages.push(serde_json::json!({
                "package": pkg,
                "count": entities.len(),
            }));
        }

        sort_by_json_field(&mut packages, "package");

        let count = packages.len();
        Ok(serde_json::to_value(SuccessResponse::new(packages, count))?)
    }

    /// List all database.schema combinations with entity counts.
    ///
    /// # Errors
    /// Returns an error if serialization fails or the manifest is unavailable.
    #[instrument(skip(self), fields(tool = "list_databases"))]
    pub async fn list_databases(&self) -> Result<JsonValue> {
        let mut dbs: Vec<JsonValue> = Vec::new();
        for (db_schema, entities) in &self.by_database_schema {
            dbs.push(serde_json::json!({
                "database_schema": db_schema,
                "count": entities.len(),
            }));
        }

        sort_by_json_field(&mut dbs, "database_schema");

        let count = dbs.len();
        Ok(serde_json::to_value(SuccessResponse::new(dbs, count))?)
    }
}
