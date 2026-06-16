use std::path::PathBuf;

#[test]
#[ignore = "writes tests/fixtures/tokenomics.duckdb for local agent eval runs"]
fn create_tokenomics_duckdb_fixture() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("tokenomics.duckdb");
    if path.exists() {
        std::fs::remove_file(&path).expect("remove existing fixture db");
    }

    let connection = duckdb::Connection::open(&path).expect("open fixture db");
    connection
        .execute_batch(
            "
            CREATE SCHEMA nova_tokenomics;
            CREATE TABLE nova_tokenomics.base__amplitude_sessions (
                session_date DATE,
                country_code TEXT,
                channel TEXT,
                sessions INTEGER,
                converted_sessions INTEGER,
                checkout_started_sessions INTEGER,
                checkout_completed_sessions INTEGER
            );

            INSERT INTO nova_tokenomics.base__amplitude_sessions VALUES
                ('2026-05-24', 'GB', 'digital', 1000, 120, 300, 180),
                ('2025-05-25', 'GB', 'digital', 800, 80, 250, 125),
                ('2026-05-24', 'US', 'digital', 2000, 300, 700, 420),
                ('2025-05-25', 'US', 'digital', 1600, 192, 500, 275),
                ('2026-05-24', 'GB', 'store', 500, 40, 0, 0),
                ('2025-05-25', 'GB', 'store', 450, 36, 0, 0);
            ",
        )
        .expect("seed fixture db");
}
