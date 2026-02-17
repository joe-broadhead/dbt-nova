use dbt_nova::config::DbtNovaConfig;

fn main() {
    let config = DbtNovaConfig::default();
    let json = match serde_json::to_string_pretty(&config) {
        Ok(json) => json,
        Err(err) => {
            eprintln!("Failed to serialize default config: {err}");
            std::process::exit(1);
        }
    };
    println!("{json}");
}
