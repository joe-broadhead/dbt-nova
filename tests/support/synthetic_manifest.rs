use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use serde_json::{Map, Value as JsonValue, json};

#[derive(Clone, Copy, Debug)]
pub struct SyntheticManifestConfig {
    pub models: usize,
    pub packages: usize,
    pub columns_per_model: usize,
    pub ref_fanout: usize,
    pub metric_every: usize,
}

impl Default for SyntheticManifestConfig {
    fn default() -> Self {
        Self {
            models: 300,
            packages: 3,
            columns_per_model: 8,
            ref_fanout: 2,
            metric_every: 10,
        }
    }
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct SyntheticManifestSummary {
    pub models: usize,
    pub packages: usize,
    pub columns_per_model: usize,
    pub ref_fanout: usize,
    pub indicator_count: usize,
    pub target_unique_id: String,
}

pub fn write_synthetic_manifest(
    path: &Path,
    config: SyntheticManifestConfig,
) -> io::Result<SyntheticManifestSummary> {
    let packages = config.packages.max(1);
    let models = config.models.max(1);
    let columns_per_model = config.columns_per_model.max(1);
    let metric_every = config.metric_every.max(1);

    let mut nodes = Map::new();
    let mut parent_map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut child_map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut indicator_count = 0usize;

    for index in 0..models {
        let unique_id = model_unique_id(index, packages);
        child_map.entry(unique_id).or_default();
    }

    for index in 0..models {
        let name = model_name(index);
        let package_name = package_name(index, packages);
        let unique_id = model_unique_id(index, packages);
        let dependencies = dependencies_for(index, packages, config.ref_fanout);
        for dependency in &dependencies {
            child_map
                .entry(dependency.clone())
                .or_default()
                .push(unique_id.clone());
        }
        parent_map.insert(unique_id.clone(), dependencies.clone());

        let indicators = indicators_for(index, metric_every);
        indicator_count += indicators.len();
        let columns = columns_for(columns_per_model);
        let raw_refs = dependencies
            .iter()
            .map(|dependency| {
                let dependency_name = dependency.rsplit('.').next().unwrap_or(dependency);
                format!("{{{{ ref('{dependency_name}') }}}}")
            })
            .collect::<Vec<_>>()
            .join(", ");
        let raw_from = if raw_refs.is_empty() {
            "raw.synthetic_source".to_string()
        } else {
            raw_refs
        };

        nodes.insert(
            unique_id.clone(),
            json!({
                "name": name,
                "resource_type": "model",
                "package_name": package_name,
                "description": format!(
                    "Synthetic revenue and customer model {index} for lexical scale measurement."
                ),
                "database": "analytics",
                "schema": format!("pkg_{:02}", index % packages),
                "relation_name": format!("analytics.pkg_{:02}.{name}", index % packages),
                "path": format!("models/{package_name}/{name}.sql"),
                "original_file_path": format!("models/{package_name}/{name}.sql"),
                "unique_id": unique_id,
                "fqn": [package_name, "models".to_string(), name.clone()],
                "alias": name,
                "checksum": {"name": "sha256", "checksum": format!("{index:064}")},
                "config": {
                    "enabled": true,
                    "meta": {
                        "nova": {
                            "role": if index % 5 == 0 { "dimension" } else { "fact" },
                            "canonical": index % 10 == 0,
                            "grain": {"entities": ["customer_id"], "time": "order_date"},
                            "synonyms": [
                                "orders",
                                "customer revenue",
                                "sales activity"
                            ],
                            "measures": [
                                {
                                    "name": "gross_revenue",
                                    "expression": "sum(gross_revenue)",
                                    "synonyms": ["sales", "bookings"]
                                },
                                {
                                    "name": "order_count",
                                    "expression": "count(distinct order_id)",
                                    "synonyms": ["orders"]
                                }
                            ],
                            "metrics": indicators
                        }
                    }
                },
                "tags": ["synthetic", "revenue"],
                "columns": columns,
                "depends_on": {"nodes": dependencies, "macros": []},
                "raw_code": format!(
                    "select customer_id, order_id, order_date, gross_revenue from {raw_from}"
                ),
                "compiled_code": "select customer_id, order_id, order_date, gross_revenue from analytics.synthetic_source"
            }),
        );
    }

    let manifest = json!({
        "metadata": {
            "dbt_schema_version": "https://schemas.getdbt.com/dbt/manifest/v12.json",
            "dbt_version": "1.10.2",
            "project_name": "synthetic_scale"
        },
        "selectors": {},
        "nodes": nodes,
        "sources": {},
        "macros": {},
        "docs": {},
        "exposures": {},
        "metrics": {},
        "semantic_models": {},
        "parent_map": parent_map,
        "child_map": child_map
    });

    let bytes = serde_json::to_vec_pretty(&manifest).map_err(io::Error::other)?;
    std::fs::write(path, bytes)?;

    Ok(SyntheticManifestSummary {
        models,
        packages,
        columns_per_model,
        ref_fanout: config.ref_fanout,
        indicator_count,
        target_unique_id: model_unique_id(models / 2, packages),
    })
}

fn model_name(index: usize) -> String {
    format!("fct_orders_{index:05}")
}

fn package_name(index: usize, packages: usize) -> String {
    format!("pkg_{:02}", index % packages)
}

fn model_unique_id(index: usize, packages: usize) -> String {
    format!(
        "model.{}.{}",
        package_name(index, packages),
        model_name(index)
    )
}

fn dependencies_for(index: usize, packages: usize, ref_fanout: usize) -> Vec<String> {
    let dependency_count = ref_fanout.min(index);
    (1..=dependency_count)
        .map(|offset| model_unique_id(index - offset, packages))
        .collect()
}

fn indicators_for(index: usize, metric_every: usize) -> Vec<JsonValue> {
    if index
        .checked_rem(metric_every)
        .is_some_and(|value| value != 0)
    {
        return Vec::new();
    }
    vec![json!({
        "name": "gross_revenue_per_customer",
        "expression": "sum(gross_revenue) / nullif(count(distinct customer_id), 0)",
        "synonyms": ["customer value", "arpc"]
    })]
}

fn columns_for(columns_per_model: usize) -> Map<String, JsonValue> {
    let mut columns = Map::new();
    for index in 0..columns_per_model {
        let (name, data_type, description, nova) = column_profile(index);
        columns.insert(
            name.to_string(),
            json!({
                "name": name,
                "description": description,
                "data_type": data_type,
                "meta": {"nova": nova},
                "config": {"meta": {}, "tags": []},
                "constraints": [],
                "tags": []
            }),
        );
    }
    columns
}

fn column_profile(index: usize) -> (String, &'static str, &'static str, JsonValue) {
    match index {
        0 => (
            "customer_id".to_string(),
            "string",
            "Synthetic customer identifier.",
            json!({"role": "identifier", "semantic_type": "customer_id"}),
        ),
        1 => (
            "order_id".to_string(),
            "string",
            "Synthetic order identifier.",
            json!({"role": "identifier", "semantic_type": "order_id"}),
        ),
        2 => (
            "order_date".to_string(),
            "date",
            "Synthetic order date.",
            json!({"role": "time", "semantic_type": "date", "synonyms": ["reporting date"]}),
        ),
        3 => (
            "gross_revenue".to_string(),
            "numeric",
            "Synthetic gross revenue amount.",
            json!({"role": "measure", "semantic_type": "money", "synonyms": ["gross sales"]}),
        ),
        4 => (
            "net_revenue".to_string(),
            "numeric",
            "Synthetic net revenue amount.",
            json!({"role": "measure", "semantic_type": "money", "synonyms": ["net sales"]}),
        ),
        5 => (
            "order_status".to_string(),
            "string",
            "Synthetic order status.",
            json!({"role": "dimension", "semantic_type": "status"}),
        ),
        6 => (
            "country_code".to_string(),
            "string",
            "Synthetic market country code.",
            json!({
                "role": "dimension",
                "semantic_type": "country_code",
                "synonyms": ["market"],
                "example_values": ["US", "GB"]
            }),
        ),
        7 => (
            "sales_channel".to_string(),
            "string",
            "Synthetic sales channel.",
            json!({"role": "dimension", "semantic_type": "channel"}),
        ),
        _ => (
            format!("attribute_{index:02}"),
            "string",
            "Synthetic descriptive attribute.",
            json!({"role": "dimension", "semantic_type": "attribute"}),
        ),
    }
}
