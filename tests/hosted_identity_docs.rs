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
fn hosted_identity_contract_documents_proxy_and_jwt_modes() {
    let contract = read_doc("docs/development/hosted-identity-contract.md");
    let config_reference = read_doc("docs/configuration/reference.md");
    let hosted_deployment = read_doc("docs/operations/hosted-deployment.md");

    assert!(contract.contains("DBT_NOVA_AUTH_MODE"));
    assert!(contract.contains("proxy_signed_headers"));
    assert!(contract.contains("jwt"));
    assert!(contract.contains("Proxy Envelope"));
    assert!(contract.contains("HMAC-SHA256"));
    assert!(config_reference.contains("DBT_NOVA_AUTH_MODE"));
    assert!(config_reference.contains("proxy_signed_headers` and `jwt` are enforced"));
    assert!(hosted_deployment.contains("Proxy-Signed Identity Mode"));
    assert!(hosted_deployment.contains("JWT Identity Mode"));
    assert!(contract.contains("HS* algorithms are never accepted"));
    assert!(config_reference.contains("asymmetric/EdDSA algorithms only"));
}
