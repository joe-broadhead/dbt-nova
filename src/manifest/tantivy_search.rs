use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::Instant;

use serde_json::Value as JsonValue;
use tantivy::collector::TopDocs;
use tantivy::query::{
    BooleanQuery, BoostQuery, FuzzyTermQuery, Occur, Query, QueryParser, TermQuery,
};
use tantivy::schema::{
    Field, IndexRecordOption, STORED, STRING, Schema, TextFieldIndexing, TextOptions, Value,
};
use tantivy::snippet::{Snippet, SnippetGenerator};
use tantivy::tokenizer::{
    Language, LowerCaser, NgramTokenizer, RegexTokenizer, SimpleTokenizer, Stemmer, TextAnalyzer,
};
use tantivy::{Index, IndexReader, IndexWriter, Term};

use crate::config::SearchConfig;
use crate::error::{DbtNovaError, Result};
use crate::manifest::store::EntityStore;
use crate::utils::{SearchPersona, has_query_syntax, tokenize_alnum_lowercase};
use tracing::{debug, info, instrument, warn};

const MAX_FETCH_LIMIT: usize = 2000;

/// Field handles for quick access during indexing and querying.
#[derive(Clone, Copy)]
pub struct FieldHandles {
    pub unique_id: Field,
    pub alias: Field,
    pub alias_code: Field,
    pub alias_ngram: Field,
    pub name: Field,
    pub name_code: Field,
    pub name_ngram: Field,
    pub description: Field,
    pub columns: Field,
    pub columns_code: Field,
    pub columns_ngram: Field,
    pub tags: Field,
    pub tags_code: Field,
    pub tags_ngram: Field,
    pub file_path: Field,
    pub file_path_code: Field,
    pub raw_code: Field,
    pub resource_type: Field,
    pub package_name: Field,
    pub nova_synonyms: Field,
    pub nova_domains: Field,
    pub nova_use_cases: Field,
    pub nova_measures: Field,
    pub nova_metric: Field,
    pub nova_sensitivity: Field,
    pub nova_pii: Field,
    pub nova_compliance: Field,
}

/// Search hit with score and optional highlights.
pub struct SearchHit {
    pub unique_id: String,
    pub score: f32,
    pub highlights: Option<HashMap<String, Vec<String>>>,
}

/// Search request parameters for the Tantivy index.
pub struct SearchRequest<'a> {
    pub query_text: &'a str,
    pub resource_types: &'a [String],
    pub limit: usize,
    pub min_score: Option<f32>,
    pub fuzzy: bool,
    pub include_highlights: bool,
    pub include_ngram_override: Option<bool>,
    pub scope: SearchScope,
    pub persona: SearchPersona,
}

/// Scope for search queries (full search or suggestion-only).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchScope {
    Full,
    Suggestion,
}

/// Tantivy-based full-text search engine for dbt entities.
#[derive(Clone)]
pub struct TantivySearcher {
    index: Index,
    reader: IndexReader,
    fields: FieldHandles,
}

impl TantivySearcher {
    /// Open an existing Tantivy index if present and compatible.
    ///
    /// # Errors
    /// Returns an error if the index cannot be opened or tokenizers fail to register.
    pub fn open(storage_dir: &Path, config: &SearchConfig) -> Result<Option<Self>> {
        let index_dir = storage_dir.join(&config.index_dir);
        if !index_dir.exists() {
            return Ok(None);
        }
        let (schema, fields) = Self::build_schema();
        let index = match Index::open_in_dir(&index_dir) {
            Ok(index) => index,
            Err(err) => {
                tracing::warn!(error = %err, "failed to open existing tantivy index");
                return Ok(None);
            }
        };
        if index.schema() != schema {
            tracing::warn!("existing tantivy schema mismatch; rebuilding index");
            return Ok(None);
        }
        Self::register_tokenizers(&index, config)?;
        let reader = index.reader()?;
        Ok(Some(Self {
            index,
            reader,
            fields,
        }))
    }
    /// Build a new Tantivy index from on-disk entity storage.
    ///
    /// # Errors
    /// Returns an error if the index cannot be created, written, or committed.
    #[instrument(
        level = "info",
        skip(resource_types, file_paths, tags_by_id, config),
        fields(
            storage_dir = %storage_dir.display(),
            index_dir = %config.index_dir,
            enable_ngram = config.enable_ngram,
            max_sql_chunk_bytes = config.max_sql_chunk_bytes
        )
    )]
    #[allow(clippy::too_many_lines)]
    pub fn build(
        storage_dir: &Path,
        resource_types: &HashMap<String, String>,
        file_paths: &HashMap<String, String>,
        tags_by_id: &HashMap<String, Vec<String>>,
        config: &SearchConfig,
    ) -> Result<Self> {
        let (schema, fields) = Self::build_schema();

        let index_dir = storage_dir.join(&config.index_dir);
        if index_dir.exists() {
            fs::remove_dir_all(&index_dir)?;
        }
        fs::create_dir_all(&index_dir)?;

        let index = Index::create_in_dir(&index_dir, schema.clone())?;
        Self::register_tokenizers(&index, config)?;

        let heap = config.index_writer_heap_bytes.max(1);
        let mut writer: IndexWriter = index.writer(heap)?;

        let store = EntityStore::open(storage_dir)?;
        let build_start = Instant::now();
        let mut doc_count = 0usize;

        for unique_id in store.ids() {
            let Some(entity) = store.get_archived(unique_id)? else {
                continue;
            };
            let entity_json = entity.to_json_value();

            let mut doc = tantivy::TantivyDocument::new();

            doc.add_text(fields.unique_id, unique_id);

            if let Some(alias) = entity.alias_str() {
                doc.add_text(fields.alias, alias);
                doc.add_text(fields.alias_code, alias);
                if config.enable_ngram {
                    doc.add_text(fields.alias_ngram, alias);
                }
            }

            if let Some(name) = entity.name_str() {
                doc.add_text(fields.name, name);
                doc.add_text(fields.name_code, name);
                if config.enable_ngram {
                    doc.add_text(fields.name_ngram, name);
                }
            }

            if let Some(desc) = entity.description_str().filter(|desc| !desc.is_empty()) {
                doc.add_text(fields.description, desc);
            }

            if let Some(cols) = entity_json.get("columns").and_then(|c| c.as_object()) {
                let mut tokens: Vec<String> = Vec::new();
                for (name, col) in cols {
                    tokens.push(name.clone());
                    if let Some(nova) = col
                        .get("meta")
                        .and_then(|m| m.get("nova"))
                        .and_then(|v| v.as_object())
                    {
                        if let Some(semantic) = nova.get("semantic_type").and_then(|v| v.as_str()) {
                            tokens.push(semantic.to_string());
                        }
                        if let Some(syns) = nova.get("synonyms").and_then(|v| v.as_array()) {
                            for syn in syns {
                                if let Some(s) = syn.as_str().filter(|s| !s.is_empty()) {
                                    tokens.push(s.to_string());
                                }
                            }
                        }
                        if let Some(values) = nova.get("example_values").and_then(|v| v.as_array())
                        {
                            for value in values {
                                if let Some(v) = value.as_str().filter(|v| !v.is_empty()) {
                                    tokens.push(v.to_string());
                                }
                            }
                        }
                    }
                }
                if !tokens.is_empty() {
                    let joined = tokens.join(" ");
                    doc.add_text(fields.columns, &joined);
                    doc.add_text(fields.columns_code, &joined);
                    if config.enable_ngram {
                        doc.add_text(fields.columns_ngram, &joined);
                    }
                }
            }

            if let Some(tag_strs) = tags_by_id.get(unique_id).filter(|tags| !tags.is_empty()) {
                let joined = tag_strs.join(" ");
                doc.add_text(fields.tags, &joined);
                doc.add_text(fields.tags_code, &joined);
                if config.enable_ngram {
                    doc.add_text(fields.tags_ngram, &joined);
                }
            }

            if let Some(path) = file_paths.get(unique_id) {
                doc.add_text(fields.file_path, path);
                doc.add_text(fields.file_path_code, path);
            }

            Self::add_capped_text(
                &mut doc,
                fields.raw_code,
                entity_json.get("raw_code").and_then(|v| v.as_str()),
                config.max_sql_chunk_bytes,
            );
            Self::add_capped_text(
                &mut doc,
                fields.raw_code,
                entity_json.get("macro_sql").and_then(|v| v.as_str()),
                config.max_sql_chunk_bytes,
            );
            Self::add_capped_text(
                &mut doc,
                fields.raw_code,
                entity_json.get("block_contents").and_then(|v| v.as_str()),
                config.max_sql_chunk_bytes,
            );

            if let Some(rt) = resource_types.get(unique_id) {
                doc.add_text(fields.resource_type, rt);
            } else if let Some(rt) = entity.resource_type_str() {
                doc.add_text(fields.resource_type, rt);
            }

            if let Some(pkg) = entity.package_name_str() {
                doc.add_text(fields.package_name, pkg);
            }

            if let Some(nova) = entity_json
                .get("meta")
                .and_then(|m| m.get("nova"))
                .and_then(|v| v.as_object())
            {
                if let Some(syns) = nova.get("synonyms").and_then(|v| v.as_array()) {
                    for syn in syns {
                        if let Some(s) = syn.as_str().filter(|s| !s.is_empty()) {
                            doc.add_text(fields.nova_synonyms, s);
                        }
                    }
                }
                if let Some(domains) = nova.get("domains").and_then(|v| v.as_array()) {
                    for domain in domains {
                        if let Some(d) = domain.as_str().filter(|d| !d.is_empty()) {
                            doc.add_text(fields.nova_domains, d);
                        }
                    }
                }
                if let Some(use_cases) = nova.get("use_cases").and_then(|v| v.as_array()) {
                    for use_case in use_cases {
                        if let Some(u) = use_case.as_str().filter(|u| !u.is_empty()) {
                            doc.add_text(fields.nova_use_cases, u);
                        }
                    }
                }
                if let Some(measures) = nova.get("measures").and_then(|v| v.as_array()) {
                    for measure in measures {
                        if let Some(name) = measure
                            .get("name")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                        {
                            doc.add_text(fields.nova_measures, name);
                        }
                        if let Some(syns) = measure.get("synonyms").and_then(|v| v.as_array()) {
                            for syn in syns {
                                if let Some(s) = syn.as_str().filter(|s| !s.is_empty()) {
                                    doc.add_text(fields.nova_measures, s);
                                }
                            }
                        }
                    }
                }
                if let Some(metric) = nova.get("metric").and_then(|v| v.as_object()) {
                    if let Some(name) = metric
                        .get("name")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                    {
                        doc.add_text(fields.nova_metric, name);
                    }
                    if let Some(description) = metric
                        .get("description")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                    {
                        doc.add_text(fields.nova_metric, description);
                    }
                    if let Some(syns) = metric.get("synonyms").and_then(|v| v.as_array()) {
                        for syn in syns {
                            if let Some(s) = syn.as_str().filter(|s| !s.is_empty()) {
                                doc.add_text(fields.nova_metric, s);
                            }
                        }
                    }
                }
                if let Some(metrics) = nova.get("metrics").and_then(|v| v.as_array()) {
                    for metric in metrics {
                        let Some(metric) = metric.as_object() else {
                            continue;
                        };
                        if let Some(name) = metric
                            .get("name")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                        {
                            doc.add_text(fields.nova_metric, name);
                        }
                        if let Some(description) = metric
                            .get("description")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                        {
                            doc.add_text(fields.nova_metric, description);
                        }
                        if let Some(syns) = metric.get("synonyms").and_then(|v| v.as_array()) {
                            for syn in syns {
                                if let Some(s) = syn.as_str().filter(|s| !s.is_empty()) {
                                    doc.add_text(fields.nova_metric, s);
                                }
                            }
                        }
                    }
                }
                if let Some(gov) = nova.get("governance").and_then(|v| v.as_object()) {
                    if let Some(sensitivity) = gov
                        .get("sensitivity")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                    {
                        doc.add_text(fields.nova_sensitivity, sensitivity);
                    }
                    if let Some(pii) = gov.get("pii") {
                        match pii {
                            JsonValue::Bool(true) => {
                                doc.add_text(fields.nova_pii, "pii");
                                doc.add_text(fields.nova_pii, "true");
                            }
                            JsonValue::String(s) => {
                                if !s.is_empty() {
                                    doc.add_text(fields.nova_pii, "pii");
                                    doc.add_text(fields.nova_pii, s);
                                }
                            }
                            JsonValue::Array(values) => {
                                doc.add_text(fields.nova_pii, "pii");
                                for value in values {
                                    if let Some(s) = value.as_str().filter(|s| !s.is_empty()) {
                                        doc.add_text(fields.nova_pii, s);
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    if let Some(compliance) = gov.get("compliance").and_then(|v| v.as_array()) {
                        for entry in compliance {
                            if let Some(s) = entry.as_str().filter(|s| !s.is_empty()) {
                                doc.add_text(fields.nova_compliance, s);
                            }
                        }
                    }
                }
            }

            writer.add_document(doc)?;
            doc_count += 1;
        }

        writer.commit()?;

        let reader = index.reader()?;
        info!(
            doc_count,
            elapsed_ms = build_start.elapsed().as_millis(),
            "tantivy index build complete"
        );

        Ok(Self {
            index,
            reader,
            fields,
        })
    }

    fn add_capped_text(
        doc: &mut tantivy::TantivyDocument,
        field: Field,
        text: Option<&str>,
        max_bytes: usize,
    ) {
        let Some(text) = text else {
            return;
        };
        if max_bytes == 0 {
            return;
        }

        let bytes = text.as_bytes();
        let mut end = bytes.len().min(max_bytes);
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        if end == 0 {
            return;
        }
        if let Some(slice) = text.get(0..end) {
            doc.add_text(field, slice);
        }
    }

    /// Register tokenizers used by the Tantivy index.
    ///
    /// # Errors
    /// Returns an error if tokenizer construction fails.
    fn register_tokenizers(index: &Index, config: &SearchConfig) -> Result<()> {
        let tokenizer = TextAnalyzer::builder(SimpleTokenizer::default())
            .filter(LowerCaser)
            .build();
        index.tokenizers().register("simple_lower", tokenizer);

        let stemmed = TextAnalyzer::builder(SimpleTokenizer::default())
            .filter(LowerCaser)
            .filter(Stemmer::new(Language::English))
            .build();
        index.tokenizers().register("text_en", stemmed);

        let code_tokenizer = RegexTokenizer::new(r"[A-Za-z0-9_]+").map_err(|e| {
            DbtNovaError::ServerError(format!("Failed to build code tokenizer: {e}"))
        })?;
        let code_analyzer = TextAnalyzer::builder(code_tokenizer)
            .filter(LowerCaser)
            .build();
        index.tokenizers().register("code_lower", code_analyzer);

        let ngram_tokenizer =
            NgramTokenizer::new(config.ngram_min.max(1), config.ngram_max.max(1), false).map_err(
                |e| DbtNovaError::ServerError(format!("Failed to build ngram tokenizer: {e}")),
            )?;
        let ngram_analyzer = TextAnalyzer::builder(ngram_tokenizer)
            .filter(LowerCaser)
            .build();
        index.tokenizers().register("ngram", ngram_analyzer);

        Ok(())
    }

    fn build_schema() -> (Schema, FieldHandles) {
        let mut schema_builder = Schema::builder();

        let simple_options = TextOptions::default()
            .set_indexing_options(
                TextFieldIndexing::default()
                    .set_tokenizer("simple_lower")
                    .set_index_option(IndexRecordOption::WithFreqsAndPositions),
            )
            .set_stored();

        let stemmed_options = TextOptions::default()
            .set_indexing_options(
                TextFieldIndexing::default()
                    .set_tokenizer("text_en")
                    .set_index_option(IndexRecordOption::WithFreqsAndPositions),
            )
            .set_stored();

        let code_options = TextOptions::default().set_indexing_options(
            TextFieldIndexing::default()
                .set_tokenizer("code_lower")
                .set_index_option(IndexRecordOption::WithFreqsAndPositions),
        );

        let ngram_options = TextOptions::default().set_indexing_options(
            TextFieldIndexing::default()
                .set_tokenizer("ngram")
                .set_index_option(IndexRecordOption::WithFreqsAndPositions),
        );

        let keyword_options = STRING | STORED;

        let unique_id = schema_builder.add_text_field("unique_id", keyword_options.clone());
        let alias = schema_builder.add_text_field("alias", simple_options.clone());
        let alias_code = schema_builder.add_text_field("alias_code", code_options.clone());
        let alias_ngram = schema_builder.add_text_field("alias_ngram", ngram_options.clone());
        let name = schema_builder.add_text_field("name", simple_options.clone());
        let name_code = schema_builder.add_text_field("name_code", code_options.clone());
        let name_ngram = schema_builder.add_text_field("name_ngram", ngram_options.clone());
        let description = schema_builder.add_text_field("description", stemmed_options);
        let columns = schema_builder.add_text_field("columns", simple_options.clone());
        let columns_code = schema_builder.add_text_field("columns_code", code_options.clone());
        let columns_ngram = schema_builder.add_text_field("columns_ngram", ngram_options.clone());
        let tags = schema_builder.add_text_field("tags", simple_options.clone());
        let tags_code = schema_builder.add_text_field("tags_code", code_options.clone());
        let tags_ngram = schema_builder.add_text_field("tags_ngram", ngram_options.clone());
        let file_path = schema_builder.add_text_field("file_path", simple_options.clone());
        let file_path_code = schema_builder.add_text_field("file_path_code", code_options.clone());
        let raw_code = schema_builder.add_text_field("raw_code", simple_options.clone());
        let resource_type = schema_builder.add_text_field("resource_type", keyword_options.clone());
        let package_name = schema_builder.add_text_field("package_name", keyword_options);
        let nova_synonyms = schema_builder.add_text_field("nova_synonyms", simple_options.clone());
        let nova_domains = schema_builder.add_text_field("nova_domains", simple_options.clone());
        let nova_use_cases =
            schema_builder.add_text_field("nova_use_cases", simple_options.clone());
        let nova_measures = schema_builder.add_text_field("nova_measures", code_options.clone());
        let nova_metric = schema_builder.add_text_field("nova_metric", code_options.clone());
        let nova_sensitivity =
            schema_builder.add_text_field("nova_sensitivity", simple_options.clone());
        let nova_pii = schema_builder.add_text_field("nova_pii", simple_options.clone());
        let nova_compliance = schema_builder.add_text_field("nova_compliance", simple_options);

        let fields = FieldHandles {
            unique_id,
            alias,
            alias_code,
            alias_ngram,
            name,
            name_code,
            name_ngram,
            description,
            columns,
            columns_code,
            columns_ngram,
            tags,
            tags_code,
            tags_ngram,
            file_path,
            file_path_code,
            raw_code,
            resource_type,
            package_name,
            nova_synonyms,
            nova_domains,
            nova_use_cases,
            nova_measures,
            nova_metric,
            nova_sensitivity,
            nova_pii,
            nova_compliance,
        };

        (schema_builder.build(), fields)
    }

    /// Search with boosted fields. Returns `unique_id`/`score` with optional highlights.
    ///
    /// # Errors
    /// Returns an error if query parsing or Tantivy search fails.
    #[instrument(
        level = "debug",
        skip(self, request, config),
        fields(
            query_len = request.query_text.chars().count(),
            resource_types = ?request.resource_types,
            limit = request.limit,
            fuzzy = request.fuzzy,
            include_highlights = request.include_highlights,
            scope = ?request.scope
        )
    )]
    #[allow(clippy::too_many_lines)]
    pub fn search(
        &self,
        request: &SearchRequest<'_>,
        config: &SearchConfig,
    ) -> Result<Vec<SearchHit>> {
        let searcher = self.reader.searcher();
        let query_has_syntax = has_query_syntax(request.query_text);
        let tokens = tokenize_alnum_lowercase(request.query_text, config.min_word_length.max(1));
        let include_ngram = match request.include_ngram_override {
            Some(value) => value && config.enable_ngram,
            None => config.enable_ngram && !query_has_syntax && request.scope == SearchScope::Full,
        };

        let mut final_query: Box<dyn Query> =
            if request.fuzzy && !query_has_syntax && request.scope == SearchScope::Full {
                if tokens.is_empty() {
                    return Ok(vec![]);
                }

                let mut term_queries: Vec<(Occur, Box<dyn Query>)> = Vec::new();

                for term in &tokens {
                    let mut field_queries: Vec<(Occur, Box<dyn Query>)> = Vec::new();
                    let distance_loose = fuzzy_distance_loose(term, config);
                    let distance_strict = fuzzy_distance_strict(term, config);

                    let boosted_fields = [
                        (
                            self.fields.alias,
                            config.field_boosts.alias,
                            distance_strict,
                        ),
                        (
                            self.fields.alias_code,
                            config.field_boosts.alias,
                            distance_strict,
                        ),
                        (self.fields.name, config.field_boosts.name, distance_strict),
                        (
                            self.fields.name_code,
                            config.field_boosts.name,
                            distance_strict,
                        ),
                        (
                            self.fields.file_path,
                            config.field_boosts.path,
                            distance_strict,
                        ),
                        (
                            self.fields.file_path_code,
                            config.field_boosts.path,
                            distance_strict,
                        ),
                        (
                            self.fields.description,
                            config.field_boosts.description,
                            distance_loose,
                        ),
                        (
                            self.fields.columns,
                            config.field_boosts.column,
                            distance_loose,
                        ),
                        (
                            self.fields.columns_code,
                            config.field_boosts.column,
                            distance_loose,
                        ),
                        (self.fields.tags, config.field_boosts.tag, distance_loose),
                        (
                            self.fields.tags_code,
                            config.field_boosts.tag,
                            distance_loose,
                        ),
                        (
                            self.fields.raw_code,
                            config.field_boosts.code,
                            distance_loose,
                        ),
                        (
                            self.fields.nova_synonyms,
                            config.field_boosts.nova_synonyms,
                            distance_loose,
                        ),
                        (
                            self.fields.nova_domains,
                            config.field_boosts.nova_domains,
                            distance_loose,
                        ),
                        (
                            self.fields.nova_use_cases,
                            config.field_boosts.nova_use_cases,
                            distance_loose,
                        ),
                        (
                            self.fields.nova_sensitivity,
                            config.field_boosts.nova_sensitivity,
                            distance_loose,
                        ),
                        (
                            self.fields.nova_pii,
                            config.field_boosts.nova_pii,
                            distance_loose,
                        ),
                        (
                            self.fields.nova_compliance,
                            config.field_boosts.nova_compliance,
                            distance_loose,
                        ),
                        (
                            self.fields.nova_measures,
                            config.field_boosts.nova_measures,
                            distance_loose,
                        ),
                        (
                            self.fields.nova_metric,
                            config.field_boosts.nova_metric,
                            distance_loose,
                        ),
                    ];

                    for (field, boost, distance) in boosted_fields {
                        let term_query: Box<dyn Query> = match distance {
                            Some(dist) => Box::new(FuzzyTermQuery::new(
                                Term::from_field_text(field, term),
                                dist,
                                true,
                            )),
                            None => Box::new(TermQuery::new(
                                Term::from_field_text(field, term),
                                IndexRecordOption::WithFreqs,
                            )),
                        };

                        let boosted = Box::new(BoostQuery::new(term_query, boost));
                        field_queries.push((Occur::Should, boosted));
                    }

                    let term_bool = BooleanQuery::new(field_queries);
                    term_queries.push((Occur::Should, Box::new(term_bool)));
                }

                Box::new(BooleanQuery::new(term_queries))
            } else {
                let query = self
                    .build_query_parser(config, request.scope, include_ngram, request.fuzzy)
                    .parse_query(request.query_text)
                    .map_err(|e| {
                        DbtNovaError::InvalidParams(format!("Invalid search query syntax: {e}"))
                    })?;
                Box::new(query)
            };

        if !request.resource_types.is_empty() {
            let rt_queries: Vec<(Occur, Box<dyn Query>)> = request
                .resource_types
                .iter()
                .map(|rt| {
                    let q: Box<dyn Query> = Box::new(TermQuery::new(
                        Term::from_field_text(self.fields.resource_type, rt),
                        IndexRecordOption::Basic,
                    ));
                    (Occur::Should, q)
                })
                .collect();

            let rt_filter = Box::new(BooleanQuery::new(rt_queries));

            final_query = Box::new(BooleanQuery::new(vec![
                (Occur::Must, final_query),
                (Occur::Must, rt_filter),
            ]));
        }

        let fetch_limit = std::cmp::min(
            request
                .limit
                .saturating_mul(config.dedup_fetch_multiplier.max(1)),
            MAX_FETCH_LIMIT,
        );
        let top_docs = searcher.search(&final_query, &TopDocs::with_limit(fetch_limit))?;

        let highlight_enabled = request.include_highlights
            && config.highlight_max_fields > 0
            && config.highlight_max_chars > 0;
        let snippet_generators = if highlight_enabled {
            Some(self.build_snippet_generators(
                &searcher,
                &*final_query,
                config,
                request.persona,
            )?)
        } else {
            None
        };

        let mut seen: HashMap<String, (f32, tantivy::DocAddress)> = HashMap::new();

        let mut prev_score = f32::MAX;
        for (score, doc_address) in top_docs {
            if seen.len() >= request.limit && score < prev_score * 0.01 {
                break;
            }
            prev_score = score;
            if request.min_score.is_some_and(|min| score < min) {
                continue;
            }

            let doc: tantivy::TantivyDocument = searcher.doc(doc_address)?;

            if let Some(id_str) = doc
                .get_first(self.fields.unique_id)
                .and_then(|value| Value::as_str(&value))
            {
                match seen.get_mut(id_str) {
                    Some((best_score, best_addr)) => {
                        if score > *best_score {
                            *best_score = score;
                            *best_addr = doc_address;
                        }
                    }
                    None => {
                        seen.insert(id_str.to_string(), (score, doc_address));
                    }
                }
            }
        }

        let mut hits: Vec<(String, f32, tantivy::DocAddress)> = seen
            .into_iter()
            .map(|(id, (score, addr))| (id, score, addr))
            .collect();

        hits.sort_by(Self::score_desc_cmp);
        hits.truncate(request.limit);

        let mut results: Vec<SearchHit> = Vec::with_capacity(hits.len());
        for (id, score, addr) in hits {
            let highlights = if let Some(ref generators) = snippet_generators {
                Self::build_highlights(&searcher, addr, generators, config)?
            } else {
                None
            };
            results.push(SearchHit {
                unique_id: id,
                score,
                highlights,
            });
        }

        debug!(hit_count = results.len(), "tantivy search completed");
        Ok(results)
    }

    #[inline]
    fn score_desc_cmp(
        left: &(String, f32, tantivy::DocAddress),
        right: &(String, f32, tantivy::DocAddress),
    ) -> std::cmp::Ordering {
        let left_score = if left.1.is_finite() {
            left.1
        } else {
            f32::NEG_INFINITY
        };
        let right_score = if right.1.is_finite() {
            right.1
        } else {
            f32::NEG_INFINITY
        };
        match right_score.total_cmp(&left_score) {
            std::cmp::Ordering::Equal => left.0.cmp(&right.0),
            other => other,
        }
    }

    #[allow(clippy::too_many_lines)]
    fn build_query_parser(
        &self,
        config: &SearchConfig,
        scope: SearchScope,
        include_ngram: bool,
        enable_fuzzy: bool,
    ) -> QueryParser {
        let mut default_fields = match scope {
            SearchScope::Full => vec![
                self.fields.alias,
                self.fields.alias_code,
                self.fields.name,
                self.fields.name_code,
                self.fields.description,
                self.fields.columns,
                self.fields.columns_code,
                self.fields.tags,
                self.fields.tags_code,
                self.fields.nova_synonyms,
                self.fields.nova_domains,
                self.fields.nova_use_cases,
                self.fields.nova_sensitivity,
                self.fields.nova_pii,
                self.fields.nova_compliance,
                self.fields.nova_measures,
                self.fields.nova_metric,
                self.fields.file_path,
                self.fields.file_path_code,
                self.fields.raw_code,
            ],
            SearchScope::Suggestion => vec![
                self.fields.alias,
                self.fields.alias_code,
                self.fields.name,
                self.fields.name_code,
            ],
        };

        if include_ngram && scope == SearchScope::Full {
            default_fields.extend([
                self.fields.alias_ngram,
                self.fields.name_ngram,
                self.fields.columns_ngram,
                self.fields.tags_ngram,
            ]);
        }

        let mut parser = QueryParser::for_index(&self.index, default_fields);
        parser.set_field_boost(self.fields.alias, config.field_boosts.alias);
        parser.set_field_boost(self.fields.alias_code, config.field_boosts.alias);
        parser.set_field_boost(self.fields.name, config.field_boosts.name);
        parser.set_field_boost(self.fields.name_code, config.field_boosts.name);
        if scope == SearchScope::Full {
            parser.set_field_boost(self.fields.description, config.field_boosts.description);
            parser.set_field_boost(self.fields.columns, config.field_boosts.column);
            parser.set_field_boost(self.fields.columns_code, config.field_boosts.column);
            parser.set_field_boost(self.fields.tags, config.field_boosts.tag);
            parser.set_field_boost(self.fields.tags_code, config.field_boosts.tag);
            parser.set_field_boost(self.fields.nova_synonyms, config.field_boosts.nova_synonyms);
            parser.set_field_boost(self.fields.nova_domains, config.field_boosts.nova_domains);
            parser.set_field_boost(
                self.fields.nova_use_cases,
                config.field_boosts.nova_use_cases,
            );
            parser.set_field_boost(
                self.fields.nova_sensitivity,
                config.field_boosts.nova_sensitivity,
            );
            parser.set_field_boost(self.fields.nova_pii, config.field_boosts.nova_pii);
            parser.set_field_boost(
                self.fields.nova_compliance,
                config.field_boosts.nova_compliance,
            );
            parser.set_field_boost(self.fields.nova_measures, config.field_boosts.nova_measures);
            parser.set_field_boost(self.fields.nova_metric, config.field_boosts.nova_metric);
            parser.set_field_boost(self.fields.file_path, config.field_boosts.path);
            parser.set_field_boost(self.fields.file_path_code, config.field_boosts.path);
            parser.set_field_boost(self.fields.raw_code, config.field_boosts.code);
        }

        if include_ngram && scope == SearchScope::Full {
            parser.set_field_boost(self.fields.alias_ngram, config.ngram_boost);
            parser.set_field_boost(self.fields.name_ngram, config.ngram_boost);
            parser.set_field_boost(self.fields.columns_ngram, config.ngram_boost);
            parser.set_field_boost(self.fields.tags_ngram, config.ngram_boost);
        }

        if enable_fuzzy {
            let dist = config.fuzzy_max_distance.max(1);
            let dist = clamp_usize_to_u8(dist);
            let strict_distance = 1u8.min(dist);
            let strict_fields = [
                self.fields.alias,
                self.fields.alias_code,
                self.fields.name,
                self.fields.name_code,
                self.fields.file_path,
                self.fields.file_path_code,
            ];
            for field in strict_fields {
                parser.set_field_fuzzy(field, false, strict_distance, true);
            }
            if scope == SearchScope::Full {
                let loose_fields = [
                    self.fields.description,
                    self.fields.columns,
                    self.fields.columns_code,
                    self.fields.tags,
                    self.fields.tags_code,
                    self.fields.raw_code,
                    self.fields.nova_synonyms,
                    self.fields.nova_domains,
                    self.fields.nova_use_cases,
                    self.fields.nova_sensitivity,
                    self.fields.nova_pii,
                    self.fields.nova_compliance,
                    self.fields.nova_measures,
                    self.fields.nova_metric,
                ];
                for field in loose_fields {
                    parser.set_field_fuzzy(field, false, dist, true);
                }
            }
        }

        parser
    }

    fn build_snippet_generators(
        &self,
        searcher: &tantivy::Searcher,
        query: &dyn Query,
        config: &SearchConfig,
        persona: SearchPersona,
    ) -> Result<Vec<(String, SnippetGenerator)>> {
        let mut generators = Vec::new();
        for (name, field) in self.highlight_fields_for_persona(persona) {
            let mut generator = SnippetGenerator::create(searcher, query, field)?;
            generator.set_max_num_chars(config.highlight_max_chars.max(1));
            generators.push((name.to_string(), generator));
        }
        Ok(generators)
    }

    fn build_highlights(
        searcher: &tantivy::Searcher,
        doc_address: tantivy::DocAddress,
        generators: &[(String, SnippetGenerator)],
        config: &SearchConfig,
    ) -> Result<Option<HashMap<String, Vec<String>>>> {
        if config.highlight_max_fields == 0 {
            return Ok(None);
        }

        let doc: tantivy::TantivyDocument = searcher.doc(doc_address)?;
        let mut highlights: HashMap<String, Vec<String>> = HashMap::new();

        for (name, generator) in generators {
            if highlights.len() >= config.highlight_max_fields {
                break;
            }
            let snippet = generator.snippet_from_doc(&doc);
            if snippet.highlighted().is_empty() {
                continue;
            }
            let formatted = format_highlight(&snippet, &config.highlight_format);
            highlights.insert(name.clone(), vec![formatted]);
        }

        if highlights.is_empty() {
            Ok(None)
        } else {
            Ok(Some(highlights))
        }
    }

    /// Return the number of documents in the index.
    ///
    /// # Errors
    /// Returns an error if the count cannot be represented on this platform.
    pub fn doc_count(&self) -> Result<usize> {
        let docs = self.reader.searcher().num_docs();
        usize::try_from(docs).map_err(|_| {
            DbtNovaError::ServerError("Document count does not fit in usize".to_string())
        })
    }

    fn highlight_fields_for_persona(&self, persona: SearchPersona) -> Vec<(&'static str, Field)> {
        match persona {
            SearchPersona::Analyst => vec![
                ("alias", self.fields.alias),
                ("name", self.fields.name),
                ("description", self.fields.description),
                ("nova_measures", self.fields.nova_measures),
                ("nova_metric", self.fields.nova_metric),
                ("nova_synonyms", self.fields.nova_synonyms),
                ("columns", self.fields.columns),
            ],
            SearchPersona::Engineer => vec![
                ("alias", self.fields.alias),
                ("name", self.fields.name),
                ("file_path", self.fields.file_path),
                ("raw_code", self.fields.raw_code),
                ("columns", self.fields.columns),
            ],
            SearchPersona::Governance => vec![
                ("alias", self.fields.alias),
                ("name", self.fields.name),
                ("description", self.fields.description),
                ("tags", self.fields.tags),
                ("nova_sensitivity", self.fields.nova_sensitivity),
                ("nova_pii", self.fields.nova_pii),
                ("nova_compliance", self.fields.nova_compliance),
            ],
            SearchPersona::Default => vec![
                ("alias", self.fields.alias),
                ("name", self.fields.name),
                ("description", self.fields.description),
                ("columns", self.fields.columns),
                ("tags", self.fields.tags),
                ("file_path", self.fields.file_path),
                ("raw_code", self.fields.raw_code),
            ],
        }
    }
}

fn format_highlight(snippet: &Snippet, format: &str) -> String {
    let html = snippet.to_html();
    match format {
        "text" | "plain" => strip_b_tags(&html),
        _ => html,
    }
}

fn strip_b_tags(input: &str) -> String {
    input.replace("<b>", "").replace("</b>", "")
}

fn fuzzy_distance_loose(term: &str, config: &SearchConfig) -> Option<u8> {
    let length = term.chars().count();
    if length < config.fuzzy_min_length {
        return None;
    }

    let distance = if length <= config.fuzzy_mid_length {
        1
    } else {
        config.fuzzy_max_distance.max(1)
    };

    Some(clamp_usize_to_u8(distance))
}

fn fuzzy_distance_strict(term: &str, config: &SearchConfig) -> Option<u8> {
    let length = term.chars().count();
    if length < config.fuzzy_min_length {
        return None;
    }
    Some(1u8)
}

fn clamp_usize_to_u8(value: usize) -> u8 {
    u8::try_from(value.min(u8::MAX as usize)).unwrap_or(u8::MAX)
}
