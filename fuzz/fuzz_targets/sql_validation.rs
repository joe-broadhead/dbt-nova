#![no_main]

use dbt_nova::tools::sql::validate_sql_statement_for_provider;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(sql) = std::str::from_utf8(data) else {
        return;
    };
    for provider in ["generic", "duckdb", "databricks", "snowflake"] {
        let _ = validate_sql_statement_for_provider(sql, provider);
    }
});
