// Generated at build time by build.rs (typify).
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
