#![allow(
    clippy::module_inception,
    clippy::too_many_lines,
    clippy::cognitive_complexity
)]

mod batch_get;
mod column_lineage;
mod columns;
pub(crate) mod common;
mod context;
mod coverage;
mod deep_metadata;
mod diff;
mod discovery;
mod edge_cases;
mod find_by_path;
mod get_entity;
mod impact;
mod integration;
mod levenshtein;
mod lineage;
mod list_entities;
mod manifest_loading;
mod metadata_score;
mod path_prefix;
mod recipes;
mod search;
mod sql;
mod undocumented;
mod validate_dag;
