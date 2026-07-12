use std::fs;
use std::path::Path;

fn read_doc(path: &str) -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(path))
        .unwrap_or_else(|error| panic!("failed to read {path}: {error}"))
}

#[test]
fn hosted_identity_docs_lock_default_off_guardrails() {
    let threat_model = read_doc("docs/development/hosted-identity-threat-model.md");
    let contract = read_doc("docs/development/hosted-identity-contract.md");
    let adrs = read_doc("docs/development/adrs.md");

    for required in [
        "default-off",
        "proxy-first",
        "one-manifest metadata bridge",
        "Identity is not authorization",
        "tenant routing",
        "per-entity",
        "warehouse credential brokering",
        "semantic-layer authorization",
        "issuer, audience, expiry, not-before, signature, and algorithm",
    ] {
        assert!(
            threat_model.contains(required)
                || contract.contains(required)
                || adrs.contains(required),
            "hosted identity docs must mention `{required}`"
        );
    }
}

#[test]
fn hosted_identity_contract_is_not_documented_as_runtime_config_yet() {
    let contract = read_doc("docs/development/hosted-identity-contract.md");
    let config_reference = read_doc("docs/configuration/reference.md");

    assert!(contract.contains("DBT_NOVA_AUTH_MODE"));
    assert!(contract.contains("proxy_signed_headers"));
    assert!(contract.contains("jwt"));
    assert!(
        !config_reference.contains("DBT_NOVA_AUTH_MODE"),
        "DBT_NOVA_AUTH_MODE should stay out of the runtime config reference until implemented"
    );
}
