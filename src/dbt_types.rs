//! Public schema-derived dbt/Nova type exports.
//!
//! Runtime manifest ingestion intentionally uses the streaming parser under
//! `manifest::loader` so large manifests do not need to deserialize through this
//! generated module. The `dbt_nova::dbt_types` export is kept as a compatibility
//! surface for downstream tooling that wants Rust types generated from the
//! checked-in dbt and Nova JSON schemas.

use serde::{Deserialize, Serialize};

#[allow(
    clippy::all,
    clippy::pedantic,
    clippy::clone_on_copy,
    clippy::derivable_impls,
    clippy::doc_markdown,
    clippy::to_string_trait_impl,
    clippy::wildcard_imports,
    irrefutable_let_patterns
)]
mod generated {
    #![allow(clippy::all, clippy::pedantic, clippy::restriction, clippy::nursery)]
    use super::*;
    include!(concat!(env!("OUT_DIR"), "/dbt_types.rs"));
}

pub use generated::*;
