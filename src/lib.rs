#![warn(clippy::all)]
#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

pub mod config;
pub mod dbt_types;
pub mod error;
pub mod manifest;
pub mod params;
pub mod responses;
pub mod server;
pub mod tools;
pub mod utils;
pub mod warehouse;

pub use config::DbtNovaConfig;
pub use error::{DbtNovaError, Result};
pub use manifest::search::{ManifestSearch, ManifestSearchHandle};
pub use server::mcp::DbtNovaServer;

#[cfg(test)]
mod tests;
