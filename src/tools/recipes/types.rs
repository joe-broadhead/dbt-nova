use std::collections::HashMap;
use std::path::PathBuf;

use serde_json::Value as JsonValue;

#[derive(Debug, Clone)]
pub(super) enum RecipeQuerySource {
    ManifestAnalysis { analysis_id: String },
}

impl RecipeQuerySource {
    pub(super) fn label(&self) -> &'static str {
        match self {
            Self::ManifestAnalysis { .. } => "manifest_analysis",
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct RecipeQuery {
    pub(super) name: String,
    pub(super) path: PathBuf,
    pub(super) order: usize,
    pub(super) source: RecipeQuerySource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RecipeSqlSource {
    CompiledCode,
    RawCode,
}

impl RecipeSqlSource {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::CompiledCode => "compiled_code",
            Self::RawCode => "raw_code",
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct RecipeRecord {
    pub(super) id: String,
    pub(super) path: PathBuf,
    pub(super) queries: Vec<RecipeQuery>,
}

#[derive(Debug, Clone)]
pub(super) struct RecipeParameterSpec {
    pub(super) key: String,
    pub(super) name: String,
    pub(super) required: bool,
    pub(super) placeholder_type: Option<String>,
    pub(super) default_value: Option<JsonValue>,
    pub(super) description: Option<String>,
    pub(super) source: &'static str,
}

impl RecipeParameterSpec {
    pub(super) fn effective_required(&self) -> bool {
        self.required && self.default_value.is_none()
    }

    pub(super) fn to_json_value(&self) -> JsonValue {
        let mut obj = serde_json::Map::new();
        obj.insert("name".to_string(), JsonValue::String(self.name.clone()));
        obj.insert(
            "required".to_string(),
            JsonValue::Bool(self.effective_required()),
        );
        obj.insert(
            "source".to_string(),
            JsonValue::String(self.source.to_string()),
        );
        if let Some(placeholder_type) = &self.placeholder_type {
            obj.insert(
                "placeholder_type".to_string(),
                JsonValue::String(placeholder_type.clone()),
            );
        }
        if let Some(description) = &self.description {
            obj.insert(
                "description".to_string(),
                JsonValue::String(description.clone()),
            );
        }
        if let Some(default_value) = &self.default_value {
            obj.insert("default".to_string(), default_value.clone());
        }
        JsonValue::Object(obj)
    }
}

#[derive(Debug, Clone)]
pub(super) struct PreparedRecipeQuery {
    pub(super) query: RecipeQuery,
    pub(super) base_sql: String,
    pub(super) parameter_specs: Vec<RecipeParameterSpec>,
    pub(super) analysis_unique_id: String,
    pub(super) sql_source: RecipeSqlSource,
}

#[derive(Debug)]
pub(super) struct ResolvedRecipeSql {
    pub(super) base_sql: String,
    pub(super) payload: JsonValue,
    pub(super) analysis_unique_id: String,
    pub(super) source: RecipeSqlSource,
}

#[derive(Debug, Clone)]
pub(super) struct RecipeParameterSchema {
    pub(super) aggregated_specs: Vec<RecipeParameterSpec>,
    pub(super) query_specs: Vec<(String, Vec<RecipeParameterSpec>)>,
    pub(super) missing_parameters: Vec<String>,
    pub(super) unused_parameters: Vec<String>,
    pub(super) type_mismatches: Vec<JsonValue>,
    pub(super) effective_parameters: HashMap<String, JsonValue>,
}
