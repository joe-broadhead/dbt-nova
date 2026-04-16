use std::collections::HashSet;

use serde_json::Value as JsonValue;

use crate::config::SearchConfig;
use crate::manifest::entity::{ArchivedEntity, Entity, entity_nova_meta_json};

#[must_use]
pub fn embedding_text(entity: &Entity, config: &SearchConfig) -> String {
    embedding_text_from(entity, config)
}

#[must_use]
pub fn embedding_text_from_entity(entity: &Entity, config: &SearchConfig) -> String {
    embedding_text_from(entity, config)
}

#[must_use]
pub fn embedding_text_from_archived(entity: &ArchivedEntity, config: &SearchConfig) -> String {
    embedding_text_from(entity, config)
}

#[must_use]
pub fn embedding_text_from_payload(payload_json: &str, config: &SearchConfig) -> String {
    let entity_json: JsonValue = serde_json::from_str(payload_json).unwrap_or(JsonValue::Null);
    embedding_text_from(&entity_json, config)
}

#[must_use]
pub fn embedding_text_from_json(entity_json: &JsonValue, config: &SearchConfig) -> String {
    embedding_text_from(entity_json, config)
}

trait EmbeddingSource {
    fn name(&self) -> Option<&str>;
    fn alias(&self) -> Option<&str>;
    fn resource_type(&self) -> Option<&str>;
    fn description(&self) -> Option<&str>;
    fn visit_tags(&self, f: &mut dyn FnMut(&str));
    fn visit_columns(&self, f: &mut dyn FnMut(&str));
    fn visit_nova_meta(&self, f: &mut dyn FnMut(&str));
}

impl EmbeddingSource for Entity {
    fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    fn alias(&self) -> Option<&str> {
        self.alias.as_deref()
    }

    fn resource_type(&self) -> Option<&str> {
        self.resource_type.as_deref()
    }

    fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    fn visit_tags(&self, f: &mut dyn FnMut(&str)) {
        for tag in &self.tags {
            f(tag);
        }
    }

    fn visit_columns(&self, f: &mut dyn FnMut(&str)) {
        for column in &self.column_names {
            f(column);
        }
    }

    #[allow(clippy::too_many_lines)]
    #[allow(clippy::too_many_lines)]
    fn visit_nova_meta(&self, f: &mut dyn FnMut(&str)) {
        let Some(nova) = self.nova_meta.as_ref() else {
            return;
        };
        for syn in &nova.synonyms {
            f(syn);
        }
        for domain in &nova.domains {
            f(domain);
        }
        for use_case in &nova.use_cases {
            f(use_case);
        }
        for value in &nova.example_values {
            f(value);
        }
        for measure in &nova.measures {
            f(&measure.name);
            for syn in &measure.synonyms {
                f(syn);
            }
            if let Some(description) = measure.description.as_deref() {
                f(description);
            }
            if let Some(field) = measure.field.as_deref() {
                f(field);
            }
            if let Some(expr) = measure.expression.as_deref() {
                f(expr);
            }
        }
        for metric in nova.metric.iter().chain(nova.metrics.iter()) {
            f(&metric.name);
            if let Some(description) = metric.description.as_deref() {
                f(description);
            }
            if let Some(expression) = metric.expression.as_deref() {
                f(expression);
            }
            for syn in &metric.synonyms {
                f(syn);
            }
        }
        if nova.canonical {
            f("canonical");
        }
        if let Some(tier) = nova.tier.as_deref() {
            f(tier);
        }
        if let Some(grain) = nova.grain.as_ref() {
            for pk in &grain.primary_key {
                f(pk);
            }
            if let Some(time_field) = grain.time_field.as_deref() {
                f(time_field);
            }
            for dim in &grain.dimensions {
                f(dim);
            }
        }
        if let Some(gov) = nova.governance.as_ref() {
            if let Some(sensitivity) = gov.sensitivity.as_deref() {
                f(sensitivity);
            }
            if let Some(pii) = gov.pii.as_deref()
                && pii != "false"
            {
                f("pii");
                f(pii);
            }
            for compliance in &gov.compliance {
                f(compliance);
            }
        }
    }
}

impl EmbeddingSource for ArchivedEntity {
    fn name(&self) -> Option<&str> {
        self.name_str()
    }

    fn alias(&self) -> Option<&str> {
        self.alias_str()
    }

    fn resource_type(&self) -> Option<&str> {
        self.resource_type_str()
    }

    fn description(&self) -> Option<&str> {
        self.description_str()
    }

    fn visit_tags(&self, f: &mut dyn FnMut(&str)) {
        for tag in self.tags_iter() {
            f(tag);
        }
    }

    fn visit_columns(&self, f: &mut dyn FnMut(&str)) {
        for column in self.column_names_iter() {
            f(column);
        }
    }

    fn visit_nova_meta(&self, f: &mut dyn FnMut(&str)) {
        let Some(nova) = self.nova_meta() else {
            return;
        };
        for domain in nova.domains.iter() {
            f(domain.as_str());
        }
        for synonym in nova.synonyms.iter() {
            f(synonym.as_str());
        }
        for use_case in nova.use_cases.iter() {
            f(use_case.as_str());
        }
        for value in nova.example_values.iter() {
            f(value.as_str());
        }
        for measure in nova.measures.iter() {
            f(measure.name.as_str());
            for synonym in measure.synonyms.iter() {
                f(synonym.as_str());
            }
            if let Some(description) = measure.description.as_ref() {
                f(description.as_str());
            }
            if let Some(field) = measure.field.as_ref() {
                f(field.as_str());
            }
            if let Some(expr) = measure.expression.as_ref() {
                f(expr.as_str());
            }
        }
        if let Some(metric) = nova.metric.as_ref() {
            f(metric.name.as_str());
            if let Some(description) = metric.description.as_ref() {
                f(description.as_str());
            }
            if let Some(expression) = metric.expression.as_ref() {
                f(expression.as_str());
            }
            for synonym in metric.synonyms.iter() {
                f(synonym.as_str());
            }
        }
        for metric in nova.metrics.iter() {
            f(metric.name.as_str());
            if let Some(description) = metric.description.as_ref() {
                f(description.as_str());
            }
            if let Some(expression) = metric.expression.as_ref() {
                f(expression.as_str());
            }
            for synonym in metric.synonyms.iter() {
                f(synonym.as_str());
            }
        }
        if nova.canonical {
            f("canonical");
        }
        if let Some(tier) = nova.tier.as_ref() {
            f(tier.as_str());
        }
        if let Some(grain) = nova.grain.as_ref() {
            for pk in grain.primary_key.iter() {
                f(pk.as_str());
            }
            if let Some(time_field) = grain.time_field.as_ref() {
                f(time_field.as_str());
            }
            for dim in grain.dimensions.iter() {
                f(dim.as_str());
            }
        }
        if let Some(gov) = nova.governance.as_ref() {
            if let Some(sensitivity) = gov.sensitivity.as_ref() {
                f(sensitivity.as_str());
            }
            if let Some(pii) = gov.pii.as_ref()
                && pii.as_str() != "false"
            {
                f("pii");
                f(pii.as_str());
            }
            for compliance in gov.compliance.iter() {
                f(compliance.as_str());
            }
        }
    }
}

impl EmbeddingSource for JsonValue {
    fn name(&self) -> Option<&str> {
        self.get("name").and_then(|v| v.as_str())
    }

    fn alias(&self) -> Option<&str> {
        self.get("alias").and_then(|v| v.as_str())
    }

    fn resource_type(&self) -> Option<&str> {
        self.get("resource_type").and_then(|v| v.as_str())
    }

    fn description(&self) -> Option<&str> {
        self.get("description").and_then(|v| v.as_str())
    }

    fn visit_tags(&self, f: &mut dyn FnMut(&str)) {
        if let Some(tags) = self.get("tags").and_then(JsonValue::as_array) {
            for tag in tags.iter().filter_map(|v| v.as_str()) {
                f(tag);
            }
        }
    }

    fn visit_columns(&self, f: &mut dyn FnMut(&str)) {
        if let Some(columns) = self.get("columns").and_then(JsonValue::as_object) {
            for name in columns.keys() {
                f(name);
            }
        }
    }

    fn visit_nova_meta(&self, f: &mut dyn FnMut(&str)) {
        let Some(nova) = entity_nova_meta_json(self) else {
            return;
        };
        visit_string_array(nova, "synonyms", f);
        visit_string_array(nova, "domains", f);
        visit_string_array(nova, "use_cases", f);
        visit_string_array(nova, "example_values", f);
        if let Some(measures) = nova.get("measures").and_then(JsonValue::as_array) {
            for measure in measures {
                visit_measure_json(measure, f);
            }
        }
        let metric = nova.get("metric");
        let metrics = nova.get("metrics").and_then(JsonValue::as_array);
        for metric in metric.into_iter().chain(metrics.into_iter().flatten()) {
            visit_metric_json(metric, f);
        }
        if nova.get("canonical").and_then(JsonValue::as_bool) == Some(true) {
            f("canonical");
        }
        if let Some(tier) = nova.get("tier").and_then(JsonValue::as_str) {
            f(tier);
        }
        if let Some(grain) = nova.get("grain") {
            visit_grain_json(grain, f);
        }
        if let Some(gov) = nova.get("governance") {
            visit_governance_json(gov, f);
        }
    }
}

fn visit_string_array(value: &JsonValue, key: &str, f: &mut dyn FnMut(&str)) {
    if let Some(values) = value.get(key).and_then(JsonValue::as_array) {
        for entry in values.iter().filter_map(|v| v.as_str()) {
            f(entry);
        }
    }
}

fn visit_measure_json(measure: &JsonValue, f: &mut dyn FnMut(&str)) {
    if let Some(name) = measure.get("name").and_then(JsonValue::as_str) {
        f(name);
    }
    if let Some(description) = measure.get("description").and_then(JsonValue::as_str) {
        f(description);
    }
    if let Some(field) = measure.get("field").and_then(JsonValue::as_str) {
        f(field);
    }
    if let Some(expr) = measure.get("expression").and_then(JsonValue::as_str) {
        f(expr);
    }
    visit_string_array(measure, "synonyms", f);
}

fn visit_metric_json(metric: &JsonValue, f: &mut dyn FnMut(&str)) {
    if let Some(name) = metric.get("name").and_then(JsonValue::as_str) {
        f(name);
    }
    if let Some(description) = metric.get("description").and_then(JsonValue::as_str) {
        f(description);
    }
    if let Some(expression) = metric.get("expression").and_then(JsonValue::as_str) {
        f(expression);
    }
    visit_string_array(metric, "synonyms", f);
}

fn visit_grain_json(grain: &JsonValue, f: &mut dyn FnMut(&str)) {
    if let Some(primary_key) = grain.get("primary_key").and_then(JsonValue::as_array) {
        for pk in primary_key.iter().filter_map(|v| v.as_str()) {
            f(pk);
        }
    }
    if let Some(time_field) = grain.get("time_field").and_then(JsonValue::as_str) {
        f(time_field);
    }
    if let Some(dimensions) = grain.get("dimensions").and_then(JsonValue::as_array) {
        for dim in dimensions.iter().filter_map(|v| v.as_str()) {
            f(dim);
        }
    }
}

fn visit_governance_json(governance: &JsonValue, f: &mut dyn FnMut(&str)) {
    if let Some(sensitivity) = governance.get("sensitivity").and_then(JsonValue::as_str) {
        f(sensitivity);
    }
    if let Some(pii) = governance.get("pii") {
        match pii {
            JsonValue::Bool(true) => f("pii"),
            JsonValue::String(s) => {
                f("pii");
                f(s);
            }
            JsonValue::Array(values) => {
                f("pii");
                for value in values.iter().filter_map(|v| v.as_str()) {
                    f(value);
                }
            }
            _ => {}
        }
    }
    if let Some(compliance) = governance.get("compliance").and_then(JsonValue::as_array) {
        for entry in compliance.iter().filter_map(|v| v.as_str()) {
            f(entry);
        }
    }
}

fn embedding_text_from<T: EmbeddingSource>(source: &T, config: &SearchConfig) -> String {
    let max_chars = config.vector_max_chars;
    let mut text = String::new();
    if max_chars > 0 {
        text.reserve(max_chars.min(4096));
    } else {
        text.reserve(1024);
    }

    let mut seen: HashSet<String> = HashSet::new();
    let mut add_token = |token: &str| {
        let trimmed = token.trim();
        if trimmed.is_empty() {
            return;
        }
        let key = trimmed.to_lowercase();
        if !seen.insert(key) {
            return;
        }
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(trimmed);
    };

    if let Some(name) = source.name() {
        add_token(name);
    }
    if let Some(alias) = source.alias() {
        add_token(alias);
    }
    if let Some(resource_type) = source.resource_type() {
        add_token(resource_type);
    }
    if let Some(description) = source.description() {
        add_token(description);
    }

    source.visit_tags(&mut add_token);
    source.visit_columns(&mut add_token);
    source.visit_nova_meta(&mut add_token);

    if max_chars > 0 {
        truncate_to_char_boundary(&mut text, max_chars);
    }

    text
}

fn truncate_to_char_boundary(text: &mut String, max_bytes: usize) {
    if text.len() <= max_bytes {
        return;
    }
    let mut idx = max_bytes.min(text.len());
    while idx > 0 && !text.is_char_boundary(idx) {
        idx -= 1;
    }
    text.truncate(idx);
}
