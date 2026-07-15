use super::auth::{
    BrowserCallback, BrowserCallbackPreflight, BrowserCallbackRequest,
    ExternalBrowserAuthenticatorRequest, ExternalBrowserAuthenticatorRequestData,
    ExternalBrowserLoginRequest, ExternalBrowserLoginRequestData, SnowflakeAuthorization,
    build_external_browser_auth_config, build_workload_identity_auth_config,
    decode_external_browser_authenticator_response, decode_external_browser_login_response,
    external_browser_session_cache, generate_keypair_jwt, normalize_account_url,
    normalize_jwt_identifier, normalize_workload_identity_provider, parse_browser_callback_request,
    public_key_fingerprint, read_browser_callback_request, resolve_workload_identity_token_source,
    validate_external_browser_runtime_for_auth_with_ci,
};
use super::{
    DEFAULT_EXTERNAL_BROWSER_TIMEOUT_SECONDS, MAX_BROWSER_CALLBACK_REQUEST_BYTES, Result,
    ResultColumn, SnowflakeAuthConfig, SnowflakeExecuteOptions, SnowflakeQueryResult,
    SnowflakeQueryStats, SnowflakeSqlClient, SnowflakeSqlConfig, SnowflakeWifTokenSource,
    build_bindings, catalog_preflight_statement, decode_statement_status_response,
    normalize_preflight_relation, parse_cell_value, preflight_show_result_has_exact_name,
    relation_preflight_statement, resolve_schema_preflight_target, rewrite_named_parameters,
    schema_preflight_statement, send_json, session_parameters, snowflake_err, summarize_error_body,
};
use crate::config::{DbtNovaConfig, ServerTransport};
use flate2::{Compression, write::GzEncoder};
use reqwest::Client;
use reqwest::StatusCode;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::Write;
use std::net::TcpListener as StdTcpListener;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Mutex as TokioMutex;
use tokio::time::timeout;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TEST_RSA_PRIVATE_KEY_PKCS8: &str = r"-----BEGIN PRIVATE KEY-----
MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQDJETqse41HRBsc
7cfcq3ak4oZWFCoZlcic525A3FfO4qW9BMtRO/iXiyCCHn8JhiL9y8j5JdVP2Q9Z
IpfElcFd3/guS9w+5RqQGgCR+H56IVUyHZWtTJbKPcwWXQdNUX0rBFcsBzCRESJL
eelOEdHIjG7LRkx5l/FUvlqsyHDVJEQsHwegZ8b8C0fz0EgT2MMEdn10t6Ur1rXz
jMB/wvCg8vG8lvciXmedyo9xJ8oMOh0wUEgxziVDMMovmC+aJctcHUAYubwoGN8T
yzcvnGqL7JSh36Pwy28iPzXZ2RLhAyJFU39vLaHdljwthUaupldlNyCfa6Ofy4qN
ctlUPlN1AgMBAAECggEAdESTQjQ70O8QIp1ZSkCYXeZjuhj081CK7jhhp/4ChK7J
GlFQZMwiBze7d6K84TwAtfQGZhQ7km25E1kOm+3hIDCoKdVSKch/oL54f/BK6sKl
qlIzQEAenho4DuKCm3I4yAw9gEc0DV70DuMTR0LEpYyXcNJY3KNBOTjN5EYQAR9s
2MeurpgK2MdJlIuZaIbzSGd+diiz2E6vkmcufJLtmYUT/k/ddWvEtz+1DnO6bRHh
xuuDMeJA/lGB/EYloSLtdyCF6sII6C6slJJtgfb0bPy7l8VtL5iDyz46IKyzdyzW
tKAn394dm7MYR1RlUBEfqFUyNK7C+pVMVoTwCC2V4QKBgQD64syfiQ2oeUlLYDm4
CcKSP3RnES02bcTyEDFSuGyyS1jldI4A8GXHJ/lG5EYgiYa1RUivge4lJrlNfjyf
dV230xgKms7+JiXqag1FI+3mqjAgg4mYiNjaao8N8O3/PD59wMPeWYImsWXNyeHS
55rUKiHERtCcvdzKl4u35ZtTqQKBgQDNKnX2bVqOJ4WSqCgHRhOm386ugPHfy+8j
m6cicmUR46ND6ggBB03bCnEG9OtGisxTo/TuYVRu3WP4KjoJs2LD5fwdwJqpgtHl
yVsk45Y1Hfo+7M6lAuR8rzCi6kHHNb0HyBmZjysHWZsn79ZM+sQnLpgaYgQGRbKV
DZWlbw7g7QKBgQCl1u+98UGXAP1jFutwbPsx40IVszP4y5ypCe0gqgon3UiY/G+1
zTLp79GGe/SjI2VpQ7AlW7TI2A0bXXvDSDi3/5Dfya9ULnFXv9yfvH1QwWToySpW
Kvd1gYSoiX84/WCtjZOr0e0HmLIb0vw0hqZA4szJSqoxQgvF22EfIWaIaQKBgQCf
34+OmMYw8fEvSCPxDxVvOwW2i7pvV14hFEDYIeZKW2W1HWBhVMzBfFB5SE8yaCQy
pRfOzj9aKOCm2FjjiErVNpkQoi6jGtLvScnhZAt/lr2TXTrl8OwVkPrIaN0bG/AS
aUYxmBPCpXu3UjhfQiWqFq/mFyzlqlgvuCc9g95HPQKBgAscKP8mLxdKwOgX8yFW
GcZ0izY/30012ajdHY+/QK5lsMoxTnn0skdS+spLxaS5ZEO4qvPVb8RAoCkWMMal
2pOhmquJQVDPDLuZHdrIiKiDM20dy9sMfHygWcZjQ4WSxf/J7T9canLZIXFhHAZT
3wc9h4G8BBCtWN2TN/LsGZdB
-----END PRIVATE KEY-----";

#[test]
fn normalize_account_url_defaults_to_https() {
    let url =
        normalize_account_url("org-account.snowflakecomputing.com/").expect("valid account url");
    assert_eq!(url, "https://org-account.snowflakecomputing.com");
}

#[test]
fn normalize_account_url_rejects_http_by_default() {
    let err = normalize_account_url("http://localhost:8080").expect_err("http should fail");
    assert!(err.to_string().contains("https"));
}

#[test]
fn normalize_account_url_rejects_paths_queries_and_credentials() {
    for input in [
        "https://acct.snowflakecomputing.com/api",
        "https://acct.snowflakecomputing.com?token=secret",
        "https://user:pass@acct.snowflakecomputing.com",
    ] {
        assert!(
            normalize_account_url(input).is_err(),
            "{input} should be rejected"
        );
    }
}

#[test]
fn snowflake_http_summary_redacts_auth_bodies() {
    let body = r#"{"code":"390303","message":"Invalid OAuth access token. ...TTTTTTTT"}"#;
    let summary = summarize_error_body(StatusCode::UNAUTHORIZED, body);
    assert!(!summary.contains("TTTTTTTT"));
    assert!(summary.contains("authorization failed"));
}

#[test]
fn external_browser_runtime_policy_rejects_configured_non_loopback_http_bind() {
    let config = DbtNovaConfig {
        server_transport: ServerTransport::StreamableHttp,
        http_host: "0.0.0.0".to_string(),
        ..DbtNovaConfig::default()
    };

    let err =
        validate_external_browser_runtime_for_auth_with_ci(&config, Some("externalbrowser"), false)
            .expect_err("externalbrowser should reject hosted non-loopback binds");
    assert!(err.to_string().contains("non-loopback"));

    validate_external_browser_runtime_for_auth_with_ci(&config, Some("keypair"), false)
        .expect("non-browser auth is allowed for hosted binds");
}

#[test]
fn external_browser_runtime_policy_allows_configured_loopback_http_bind() {
    let config = DbtNovaConfig {
        server_transport: ServerTransport::StreamableHttp,
        http_host: "127.0.0.1".to_string(),
        ..DbtNovaConfig::default()
    };

    validate_external_browser_runtime_for_auth_with_ci(&config, Some("browser"), false)
        .expect("externalbrowser is allowed on loopback binds");
}

#[test]
fn external_browser_runtime_policy_rejects_ci() {
    let config = DbtNovaConfig::default();

    let err =
        validate_external_browser_runtime_for_auth_with_ci(&config, Some("external_browser"), true)
            .expect_err("externalbrowser should reject CI");
    assert!(err.to_string().contains("CI"));
}

#[test]
fn external_browser_auth_env_requires_user() {
    let Err(err) = build_external_browser_auth_config(
        "https://org-account.snowflakecomputing.com",
        Some("org-account".to_string()),
        None,
        Duration::from_secs(DEFAULT_EXTERNAL_BROWSER_TIMEOUT_SECONDS),
        true,
        None,
    ) else {
        panic!("externalbrowser without user should fail");
    };
    assert!(err.to_string().contains("DBT_NOVA_SNOWFLAKE_USER"));
}

#[test]
fn external_browser_auth_env_resolves_aliases_and_options() {
    let auth = build_external_browser_auth_config(
        "https://org-account.snowflakecomputing.com",
        Some("org-account".to_string()),
        Some("analyst@example.com".to_string()),
        Duration::from_secs(45),
        false,
        Some(4567),
    )
    .expect("externalbrowser auth");
    let SnowflakeAuthConfig::ExternalBrowser {
        user,
        account_identifier,
        timeout,
        open_browser,
        callback_port,
        ..
    } = auth
    else {
        panic!("expected externalbrowser auth");
    };
    assert_eq!(user, "analyst@example.com");
    assert_eq!(account_identifier, "org-account");
    assert_eq!(timeout, Duration::from_secs(45));
    assert!(!open_browser);
    assert_eq!(callback_port, Some(4567));
}

#[test]
fn workload_identity_auth_config_validates_provider_and_token_source() {
    let auth =
        build_workload_identity_auth_config(Some("aws"), Some("inline-token".to_string()), None)
            .expect("wif auth config");
    let SnowflakeAuthConfig::WorkloadIdentityFederation {
        provider,
        token_source,
    } = auth
    else {
        panic!("expected workload identity auth");
    };
    assert_eq!(provider, "AWS");
    assert_eq!(
        token_source,
        SnowflakeWifTokenSource::Inline("inline-token".to_string())
    );

    assert_eq!(
        normalize_workload_identity_provider(Some("azure")).expect("provider"),
        "AZURE"
    );
    assert_eq!(
        normalize_workload_identity_provider(Some("OIDC")).expect("provider"),
        "OIDC"
    );

    let invalid_provider =
        normalize_workload_identity_provider(Some("snowflake")).expect_err("invalid provider");
    assert!(
        invalid_provider
            .to_string()
            .contains("DBT_NOVA_SNOWFLAKE_WIF_PROVIDER")
    );

    let missing_source =
        resolve_workload_identity_token_source(None, None).expect_err("missing source");
    assert!(
        missing_source
            .to_string()
            .contains("DBT_NOVA_SNOWFLAKE_WIF_TOKEN")
    );

    let ambiguous_source = resolve_workload_identity_token_source(
        Some("inline-token".to_string()),
        Some("/secure/token".to_string()),
    )
    .expect_err("ambiguous source");
    assert!(ambiguous_source.to_string().contains("Set only one"));
}

#[tokio::test]
async fn workload_identity_authorization_uses_sql_api_contract_and_rereads_token_file() {
    let token_file = tempfile::NamedTempFile::new().expect("token file");
    std::fs::write(token_file.path(), "token-v1\n").expect("write first token");
    let client = SnowflakeSqlClient::new(SnowflakeSqlConfig {
        base_url: "https://org-account.snowflakecomputing.com".to_string(),
        warehouse: "COMPUTE_WH".to_string(),
        database: None,
        schema: None,
        role: None,
        timeout: Duration::from_secs(5),
        default_statement_timeout_s: 60,
        poll_interval: Duration::from_millis(1),
        max_poll: Duration::from_secs(1),
        max_chunks: 1,
        auth: SnowflakeAuthConfig::WorkloadIdentityFederation {
            provider: "AWS".to_string(),
            token_source: SnowflakeWifTokenSource::FilePath(
                token_file.path().to_string_lossy().into_owned(),
            ),
        },
    })
    .expect("client");

    let SnowflakeAuthorization::Bearer { token, token_type } =
        client.authorization().await.expect("first authorization")
    else {
        panic!("expected bearer auth");
    };
    assert_eq!(token, "WIF.AWS.token-v1");
    assert_eq!(token_type, "WORKLOAD_IDENTITY_FEDERATION");

    std::fs::write(token_file.path(), "token-v2\n").expect("write rotated token");
    let SnowflakeAuthorization::Bearer { token, token_type } =
        client.authorization().await.expect("second authorization")
    else {
        panic!("expected bearer auth");
    };
    assert_eq!(token, "WIF.AWS.token-v2");
    assert_eq!(token_type, "WORKLOAD_IDENTITY_FEDERATION");
}

#[test]
fn external_browser_session_cache_reuses_matching_keys() {
    let first = external_browser_session_cache(
        "https://org-account.snowflakecomputing.com",
        "org-account",
        "ANALYST",
        None,
    );
    let second = external_browser_session_cache(
        "https://ORG-ACCOUNT.snowflakecomputing.com",
        "ORG-ACCOUNT",
        "analyst",
        None,
    );
    assert!(Arc::ptr_eq(&first, &second));
}

#[test]
fn external_browser_request_bodies_use_snowflake_field_names() {
    let auth_request = serde_json::to_value(ExternalBrowserAuthenticatorRequest {
        data: ExternalBrowserAuthenticatorRequestData {
            client_app_id: "dbt-nova",
            client_app_version: "0.0.0-test",
            account_name: "org-account",
            login_name: "analyst@example.com",
            authenticator: "EXTERNALBROWSER",
            browser_mode_redirect_port: "4567".to_string(),
        },
    })
    .expect("auth request JSON");
    assert_eq!(
        auth_request["data"]["LOGIN_NAME"],
        json!("analyst@example.com")
    );
    assert_eq!(auth_request["data"]["CLIENT_APP_ID"], json!("dbt-nova"));
    assert_eq!(auth_request["data"]["ACCOUNT_NAME"], json!("org-account"));
    assert_eq!(
        auth_request["data"]["AUTHENTICATOR"],
        json!("EXTERNALBROWSER")
    );
    assert_eq!(
        auth_request["data"]["BROWSER_MODE_REDIRECT_PORT"],
        json!("4567")
    );

    let login_request = serde_json::to_value(ExternalBrowserLoginRequest {
        data: ExternalBrowserLoginRequestData {
            client_app_id: "dbt-nova",
            client_app_version: "0.0.0-test",
            account_name: "org-account",
            login_name: "analyst@example.com",
            authenticator: "EXTERNALBROWSER",
            token: "callback-token",
            proof_key: Some("proof-key"),
        },
    })
    .expect("login request JSON");
    assert_eq!(login_request["data"]["CLIENT_APP_ID"], json!("dbt-nova"));
    assert_eq!(login_request["data"]["ACCOUNT_NAME"], json!("org-account"));
    assert_eq!(login_request["data"]["TOKEN"], json!("callback-token"));
    assert_eq!(login_request["data"]["PROOF_KEY"], json!("proof-key"));

    let token_only_login_request = serde_json::to_value(ExternalBrowserLoginRequest {
        data: ExternalBrowserLoginRequestData {
            client_app_id: "dbt-nova",
            client_app_version: "0.0.0-test",
            account_name: "org-account",
            login_name: "analyst@example.com",
            authenticator: "EXTERNALBROWSER",
            token: "callback-token",
            proof_key: None,
        },
    })
    .expect("token-only login request JSON");
    assert_eq!(
        token_only_login_request["data"]["TOKEN"],
        json!("callback-token")
    );
    assert!(
        token_only_login_request["data"]
            .as_object()
            .expect("login data object")
            .get("PROOF_KEY")
            .is_none()
    );
}

#[test]
fn external_browser_response_decoders_parse_success_and_sanitize_failures() {
    let auth_body = r#"{
            "success": true,
            "data": {
                "ssoUrl": "https://idp.example.com/start",
                "proofKey": "proof-key"
            }
        }"#;
    let auth = decode_external_browser_authenticator_response(StatusCode::OK, auth_body)
        .expect("authenticator response");
    assert_eq!(auth.sso_url, "https://idp.example.com/start");
    assert_eq!(auth.proof_key, "proof-key");

    let login_body = r#"{
            "success": true,
            "data": {
                "token": "session-token",
                "validityInSeconds": 3600,
                "masterToken": "master-token",
                "masterValidityInSeconds": 7200,
                "idToken": "id-token",
                "idTokenValidityInSeconds": 1800
            }
        }"#;
    let session =
        decode_external_browser_login_response(StatusCode::OK, login_body).expect("login");
    assert_eq!(session.token, "session-token");
    assert!(session.expires_at.is_some());
    assert_eq!(session.master_token.as_deref(), Some("master-token"));
    assert!(session.master_expires_at.is_some());
    assert_eq!(session.id_token.as_deref(), Some("id-token"));
    assert!(session.id_token_expires_at.is_some());

    let failure_body = r#"{
            "success": false,
            "code": "390303",
            "message": "Invalid token SECRET_TOKEN_VALUE"
        }"#;
    let err = decode_external_browser_login_response(StatusCode::OK, failure_body)
        .expect_err("login failure");
    assert!(err.to_string().contains("390303"));
    assert!(!err.to_string().contains("SECRET_TOKEN_VALUE"));
}

#[test]
fn external_browser_login_response_accepts_session_token_alias() {
    let login_body = r#"{
            "success": true,
            "data": {
                "sessionToken": "session-token",
                "validityInSeconds": "3600"
            }
        }"#;
    let session =
        decode_external_browser_login_response(StatusCode::OK, login_body).expect("login");
    assert_eq!(session.token, "session-token");
    assert!(session.expires_at.is_some());
}

#[test]
fn browser_callback_parser_extracts_token_and_checks_proof_key() {
    let request =
        "GET /?token=callback%2Ftoken&proofKey=proof-key HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
    let callback = parse_browser_callback_request(request, "proof-key").expect("browser callback");
    assert_eq!(
        callback,
        BrowserCallbackRequest::Callback(BrowserCallback {
            token: "callback/token".to_string(),
            proof_key: Some("proof-key".to_string()),
            origin: None,
        })
    );

    let err = parse_browser_callback_request(request, "other-proof").expect_err("proof mismatch");
    assert!(err.to_string().contains("proof key"));

    let uppercase_request =
        "GET /?token=callback%2Ftoken&PROOF_KEY=proof-key HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
    let callback = parse_browser_callback_request(uppercase_request, "proof-key")
        .expect("uppercase proof key browser callback");
    assert_eq!(
        callback,
        BrowserCallbackRequest::Callback(BrowserCallback {
            token: "callback/token".to_string(),
            proof_key: Some("proof-key".to_string()),
            origin: None,
        })
    );

    let token_only_request = "GET /?token=callback%2Ftoken HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
    let callback = parse_browser_callback_request(token_only_request, "proof-key")
        .expect("token-only browser callback");
    assert_eq!(
        callback,
        BrowserCallbackRequest::Callback(BrowserCallback {
            token: "callback/token".to_string(),
            proof_key: None,
            origin: None,
        })
    );
}

#[test]
fn browser_callback_parser_accepts_post_and_preflight() {
    let json_request = "POST / HTTP/1.1\r\nHost: 127.0.0.1\r\nOrigin: https://org-account.snowflakecomputing.com\r\nContent-Type: application/json\r\nContent-Length: 52\r\n\r\n{\"token\":\"callback-token\",\"proofKey\":\"proof-key\"}";
    let callback =
        parse_browser_callback_request(json_request, "proof-key").expect("json callback");
    assert_eq!(
        callback,
        BrowserCallbackRequest::Callback(BrowserCallback {
            token: "callback-token".to_string(),
            proof_key: Some("proof-key".to_string()),
            origin: Some("https://org-account.snowflakecomputing.com".to_string()),
        })
    );

    let form_request = "POST / HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: 40\r\n\r\ntoken=callback%2Ftoken&proof_key=proof-key";
    let callback =
        parse_browser_callback_request(form_request, "proof-key").expect("form callback");
    assert_eq!(
        callback,
        BrowserCallbackRequest::Callback(BrowserCallback {
            token: "callback/token".to_string(),
            proof_key: Some("proof-key".to_string()),
            origin: None,
        })
    );

    let uppercase_json_request = "POST / HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: 48\r\n\r\n{\"token\":\"callback-token\",\"PROOF_KEY\":\"proof-key\"}";
    let callback = parse_browser_callback_request(uppercase_json_request, "proof-key")
        .expect("uppercase json callback");
    assert_eq!(
        callback,
        BrowserCallbackRequest::Callback(BrowserCallback {
            token: "callback-token".to_string(),
            proof_key: Some("proof-key".to_string()),
            origin: None,
        })
    );

    let options_request = "OPTIONS / HTTP/1.1\r\nHost: 127.0.0.1\r\nOrigin: https://org-account.snowflakecomputing.com\r\nAccess-Control-Request-Headers: content-type\r\n\r\n";
    let preflight =
        parse_browser_callback_request(options_request, "proof-key").expect("preflight");
    assert_eq!(
        preflight,
        BrowserCallbackRequest::Preflight(BrowserCallbackPreflight {
            origin: Some("https://org-account.snowflakecomputing.com".to_string()),
            requested_headers: Some("content-type".to_string()),
        })
    );
}

#[test]
fn browser_callback_parser_rejects_wrong_method_path_and_missing_token() {
    for request in [
        "POST /?token=callback HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
        "GET /callback?token=callback HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
        "GET /?proofKey=proof-key HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
    ] {
        assert!(
            parse_browser_callback_request(request, "proof-key").is_err(),
            "{request} should be rejected"
        );
    }
}

#[tokio::test]
async fn browser_callback_reader_enforces_total_read_timeout() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind callback listener");
    let port = listener.local_addr().expect("listener addr").port();
    let read = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept callback");
        let started_at = std::time::Instant::now();
        let err = read_browser_callback_request(&mut socket, Duration::from_millis(60))
            .await
            .expect_err("slow callback should time out");
        (started_at.elapsed(), err.to_string())
    });

    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect callback");
    stream
        .write_all(b"POST / HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 4\r\n\r\na")
        .await
        .expect("write first chunk");
    tokio::time::sleep(Duration::from_millis(40)).await;
    stream.write_all(b"b").await.expect("write second chunk");
    tokio::time::sleep(Duration::from_millis(40)).await;
    let _ = stream.write_all(b"c").await;

    let (elapsed, error) = read.await.expect("read task");
    assert!(error.contains("timed out reading"));
    assert!(
        elapsed < Duration::from_millis(120),
        "callback read elapsed {elapsed:?}, expected total timeout bound"
    );
}

#[tokio::test]
async fn browser_callback_reader_rejects_oversized_requests() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind callback listener");
    let port = listener.local_addr().expect("listener addr").port();
    let read = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept callback");
        read_browser_callback_request(&mut socket, Duration::from_secs(1))
            .await
            .expect_err("oversized callback should fail")
            .to_string()
    });

    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect callback");
    let request = format!(
        "POST / HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\n\r\n",
        MAX_BROWSER_CALLBACK_REQUEST_BYTES + 1
    );
    stream
        .write_all(request.as_bytes())
        .await
        .expect("write oversized request");

    let error = read.await.expect("read task");
    assert!(error.contains("too large"));
}

#[tokio::test]
async fn external_browser_login_flow_exchanges_callback_token() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/session/authenticator-request"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "success": true,
            "data": {
                "ssoUrl": "https://idp.example.com/start",
                "proofKey": "proof-key"
            }
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/session/v1/login-request"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "success": true,
            "data": {
                "token": "session-token",
                "validityInSeconds": 3600
            }
        })))
        .mount(&server)
        .await;

    let callback_port = unused_loopback_port();
    let client = SnowflakeSqlClient::new(SnowflakeSqlConfig {
        base_url: server.uri(),
        warehouse: "COMPUTE_WH".to_string(),
        database: None,
        schema: None,
        role: None,
        timeout: Duration::from_secs(5),
        default_statement_timeout_s: 60,
        poll_interval: Duration::from_millis(1),
        max_poll: Duration::from_secs(1),
        max_chunks: 1,
        auth: SnowflakeAuthConfig::ExternalBrowser {
            user: "analyst@example.com".to_string(),
            account_identifier: "org-account".to_string(),
            timeout: Duration::from_secs(5),
            open_browser: false,
            callback_port: Some(callback_port),
            session_cache: Arc::new(TokioMutex::new(None)),
        },
    })
    .expect("client");

    let login = tokio::spawn(async move {
        client
            .login_external_browser(
                "org-account",
                "analyst@example.com",
                Duration::from_secs(5),
                false,
                Some(callback_port),
            )
            .await
    });

    let preflight_request = format!(
        "OPTIONS / HTTP/1.1\r\nHost: 127.0.0.1:{callback_port}\r\nOrigin: https://org-account.snowflakecomputing.com\r\nAccess-Control-Request-Headers: content-type\r\n\r\n"
    );
    let preflight_response = send_raw_browser_callback(callback_port, &preflight_request)
        .await
        .expect("preflight response");
    assert!(preflight_response.starts_with("HTTP/1.1 204 No Content"));

    let callback_body = r#"{"token":"callback-token"}"#;
    let callback_request = format!(
        "POST / HTTP/1.1\r\nHost: 127.0.0.1:{callback_port}\r\nOrigin: https://org-account.snowflakecomputing.com\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{callback_body}",
        callback_body.len()
    );
    let callback_response = send_raw_browser_callback(callback_port, &callback_request)
        .await
        .expect("callback response");
    assert!(callback_response.starts_with("HTTP/1.1 200 OK"));

    let session = login.await.expect("login task").expect("login result");
    assert_eq!(session.token, "session-token");
    assert!(session.expires_at.is_some());
}

fn unused_loopback_port() -> u16 {
    let listener = StdTcpListener::bind("127.0.0.1:0").expect("bind free port");
    listener.local_addr().expect("local addr").port()
}

async fn send_raw_browser_callback(port: u16, request: &str) -> Result<String> {
    let mut last_err = None;
    for _ in 0..100 {
        match TcpStream::connect(("127.0.0.1", port)).await {
            Ok(mut stream) => {
                stream
                    .write_all(request.as_bytes())
                    .await
                    .map_err(|err| snowflake_err(format!("write callback: {err}")))?;
                let mut response = String::new();
                stream
                    .read_to_string(&mut response)
                    .await
                    .map_err(|err| snowflake_err(format!("read callback: {err}")))?;
                return Ok(response);
            }
            Err(err) => {
                last_err = Some(err);
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
    }
    Err(snowflake_err(format!(
        "callback listener did not accept connection: {}",
        last_err.map_or_else(|| "unknown error".to_string(), |err| err.to_string())
    )))
}

fn test_snowflake_config(default_statement_timeout_s: u64) -> SnowflakeSqlConfig {
    SnowflakeSqlConfig {
        base_url: "https://org-account.snowflakecomputing.com".to_string(),
        warehouse: "COMPUTE_WH".to_string(),
        database: None,
        schema: None,
        role: None,
        timeout: Duration::from_secs(5),
        default_statement_timeout_s,
        poll_interval: Duration::from_millis(1),
        max_poll: Duration::from_secs(1),
        max_chunks: 1,
        auth: SnowflakeAuthConfig::OAuth {
            token: "oauth-token".to_string(),
        },
    }
}

#[test]
fn execute_options_allow_statement_timeout_zero_sentinel() {
    let request_override = SnowflakeExecuteOptions {
        statement_timeout_s: Some(0),
        ..SnowflakeExecuteOptions::default()
    }
    .resolve(&test_snowflake_config(60));
    assert_eq!(request_override.statement_timeout_s, 0);

    let config_default = SnowflakeExecuteOptions::default().resolve(&test_snowflake_config(0));
    assert_eq!(config_default.statement_timeout_s, 0);
}

#[test]
fn execute_options_use_config_default_max_chunks_when_unset() {
    let mut config = test_snowflake_config(60);
    config.max_chunks = 7;

    let config_default = SnowflakeExecuteOptions::default().resolve(&config);
    assert_eq!(config_default.max_chunks, 7);

    let request_override = SnowflakeExecuteOptions {
        max_chunks: Some(2),
        ..SnowflakeExecuteOptions::default()
    }
    .resolve(&config);
    assert_eq!(request_override.max_chunks, 2);
}

#[tokio::test]
async fn poll_statement_respects_max_poll_when_interval_is_larger() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v2/statements/statement-handle/cancel"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "ok"})))
        .mount(&server)
        .await;

    let mut config = test_snowflake_config(60);
    config.base_url = server.uri();
    let client = SnowflakeSqlClient::new(config).expect("client");

    let result = timeout(
        Duration::from_millis(200),
        client.poll_statement(
            "statement-handle",
            Duration::from_mins(1),
            Duration::from_millis(20),
        ),
    )
    .await
    .expect("poll timeout should be locally bounded");
    let err = result.expect_err("statement should time out");
    assert!(err.to_string().contains("Timed out waiting"));
}

#[tokio::test]
async fn send_json_decodes_gzip_responses() {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(br#"{"data":[["compressed"]]}"#)
        .expect("write gzip body");
    let body = encoder.finish().expect("finish gzip body");
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/partition"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-encoding", "gzip")
                .insert_header("content-type", "application/json")
                .set_body_bytes(body),
        )
        .mount(&server)
        .await;

    let client = Client::builder().gzip(true).build().expect("client");
    let response: Value = send_json(client.get(format!("{}/partition", server.uri())))
        .await
        .expect("decode gzip JSON");
    assert_eq!(response["data"][0][0], json!("compressed"));
}

#[test]
fn statement_status_decodes_429_query_status_as_pending() {
    let body = r#"{
            "code": "333333",
            "message": "Statement is still executing",
            "statementHandle": "536fad38-b564-4dc5-9892-a4543504df6c",
            "statementStatusUrl": "/api/v2/statements/536fad38-b564-4dc5-9892-a4543504df6c"
        }"#;
    let response =
        decode_statement_status_response(StatusCode::TOO_MANY_REQUESTS, body).expect("status");
    assert!(response.is_pending());
    assert_eq!(
        response.statement_handle.as_deref(),
        Some("536fad38-b564-4dc5-9892-a4543504df6c")
    );
}

#[test]
fn statement_status_decodes_async_query_status_as_pending() {
    let body = r#"{
            "code": "333334",
            "message": "Asynchronous execution in progress. Use provided query id to perform query monitoring and management.",
            "statementHandle": "536fad38-b564-4dc5-9892-a4543504df6c",
            "statementStatusUrl": "/api/v2/statements/536fad38-b564-4dc5-9892-a4543504df6c"
        }"#;
    let response = decode_statement_status_response(StatusCode::ACCEPTED, body).expect("status");

    assert!(response.is_pending());
    assert_eq!(response.failure_message(), None);
    assert_eq!(
        response.statement_handle.as_deref(),
        Some("536fad38-b564-4dc5-9892-a4543504df6c")
    );
}

#[test]
fn statement_status_url_error_is_terminal() {
    let body = r#"{
            "code": "604",
            "message": "Statement was canceled",
            "sqlState": "57014",
            "statementHandle": "536fad38-b564-4dc5-9892-a4543504df6c",
            "statementStatusUrl": "/api/v2/statements/536fad38-b564-4dc5-9892-a4543504df6c"
        }"#;
    let response = decode_statement_status_response(StatusCode::OK, body).expect("terminal status");

    assert!(!response.is_pending());
    let message = response.failure_message().expect("failure message");
    assert!(message.contains("604"));
    assert!(message.contains("57014"));
    assert!(message.contains("Statement was canceled"));
}

#[test]
fn statement_status_keeps_non_pending_429_as_error() {
    let body = r#"{"code":"390505","message":"Too many requests."}"#;
    let err = decode_statement_status_response(StatusCode::TOO_MANY_REQUESTS, body)
        .expect_err("rate limit should stay an error");
    assert!(err.to_string().contains("390505"));
}

#[test]
fn session_parameters_use_sql_api_field_shapes() {
    let params = session_parameters(250);
    assert_eq!(params["binary_output_format"], json!("HEX"));
    assert_eq!(params["rows_per_resultset"], json!(250));
    assert!(!params.contains_key("BINARY_OUTPUT_FORMAT"));
}

#[test]
fn normalize_jwt_identifier_strips_locator_region_suffixes() {
    assert_eq!(normalize_jwt_identifier("xy12345.us-east-1"), "XY12345");
    assert_eq!(normalize_jwt_identifier("xy12345.us-east-2.aws"), "XY12345");
    assert_eq!(
        normalize_jwt_identifier("xy12345.fhplus.us-gov-west-1.aws"),
        "XY12345"
    );
}

#[test]
fn normalize_jwt_identifier_preserves_organization_account_names() {
    assert_eq!(
        normalize_jwt_identifier("myorg.myaccount"),
        "MYORG-MYACCOUNT"
    );
    assert_eq!(
        normalize_jwt_identifier("myorg.us-east-1"),
        "MYORG-US-EAST-1"
    );
    assert_eq!(
        normalize_jwt_identifier("myorg2.us-east-1"),
        "MYORG2-US-EAST-1"
    );
    assert_eq!(
        normalize_jwt_identifier("myorg-myaccount"),
        "MYORG-MYACCOUNT"
    );
}

#[test]
fn public_key_fingerprint_uses_snowflake_sha256_prefix() {
    let fingerprint = public_key_fingerprint(TEST_RSA_PRIVATE_KEY_PKCS8).expect("fingerprint");
    assert!(fingerprint.starts_with("SHA256:"));
    assert_eq!(fingerprint.len(), "SHA256:".len() + 44);
}

#[test]
fn generate_keypair_jwt_returns_signed_token() {
    let token = generate_keypair_jwt("myorg-myaccount", "svc_user", TEST_RSA_PRIVATE_KEY_PKCS8)
        .expect("jwt");
    assert_eq!(token.split('.').count(), 3);
}

#[test]
fn rewrite_named_parameters_uses_snowflake_positional_binds() {
    let params = HashMap::from([("date".to_string(), json!("2024-01-01"))]);
    let rewritten = rewrite_named_parameters(
        "select 'literal :date', amount::number from orders where order_date >= :date",
        &params,
    )
    .expect("rewrite");
    assert_eq!(
        rewritten.sql,
        "select 'literal :date', amount::number from orders where order_date >= ?"
    );
    assert_eq!(rewritten.ordered_parameters, vec!["date".to_string()]);
}

#[test]
fn rewrite_named_parameters_skips_snowflake_variant_paths() {
    let params = HashMap::from([("country".to_string(), json!("GB"))]);
    let rewritten = rewrite_named_parameters(
        "select payload:customer_id::string, metadata:tags[0] from events where country = :country",
        &params,
    )
    .expect("rewrite");
    assert_eq!(
        rewritten.sql,
        "select payload:customer_id::string, metadata:tags[0] from events where country = ?"
    );
    assert_eq!(rewritten.ordered_parameters, vec!["country".to_string()]);
}

#[test]
fn rewrite_named_parameters_skips_dollar_quoted_literals() {
    let params = HashMap::from([("id".to_string(), json!(42))]);
    let rewritten = rewrite_named_parameters(
        "select $$literal :missing\nand 'quoted' text$$ as body where id = :id",
        &params,
    )
    .expect("rewrite");
    assert_eq!(
        rewritten.sql,
        "select $$literal :missing\nand 'quoted' text$$ as body where id = ?"
    );
    assert_eq!(rewritten.ordered_parameters, vec!["id".to_string()]);
}

#[test]
fn rewrite_named_parameters_skips_backslash_escaped_quotes() {
    let params = HashMap::from([("id".to_string(), json!(42))]);
    let rewritten =
        rewrite_named_parameters("select 'can\\'t :missing' as body where id = :id", &params)
            .expect("rewrite");
    assert_eq!(
        rewritten.sql,
        "select 'can\\'t :missing' as body where id = ?"
    );
    assert_eq!(rewritten.ordered_parameters, vec!["id".to_string()]);
}

#[test]
fn build_bindings_infers_and_numbers_by_sql_order() {
    let params = HashMap::from([
        ("country".to_string(), json!("GB")),
        ("min_amount".to_string(), json!(10)),
    ]);
    let order = vec!["country".to_string(), "min_amount".to_string()];
    let bindings = build_bindings(&order, &params, None).expect("bindings");

    assert_eq!(bindings["1"].type_name, "TEXT");
    assert_eq!(bindings["1"].value, json!("GB"));
    assert_eq!(bindings["2"].type_name, "FIXED");
    assert_eq!(bindings["2"].value, json!("10"));
}

#[test]
fn build_bindings_requires_explicit_type_for_null() {
    let params = HashMap::from([("deleted_at".to_string(), Value::Null)]);
    let order = vec!["deleted_at".to_string()];
    let err = build_bindings(&order, &params, None).expect_err("null should require type");
    assert!(err.to_string().contains("requires explicit"));
}

#[test]
fn parse_cell_value_converts_snowflake_strings_by_metadata() {
    let int_field = ResultColumn {
        name: "count".to_string(),
        type_name: "FIXED".to_string(),
        scale: Some(json!(0)),
    };
    let bool_field = ResultColumn {
        name: "flag".to_string(),
        type_name: "BOOLEAN".to_string(),
        scale: None,
    };
    let variant_field = ResultColumn {
        name: "payload".to_string(),
        type_name: "VARIANT".to_string(),
        scale: None,
    };

    assert_eq!(parse_cell_value(&json!("42"), &int_field), json!(42));
    assert_eq!(parse_cell_value(&json!("true"), &bool_field), json!(true));
    assert_eq!(
        parse_cell_value(&json!("{\"a\":1}"), &variant_field),
        json!({"a": 1})
    );
}

#[test]
fn parse_cell_value_preserves_fixed_numeric_precision() {
    let decimal_field = ResultColumn {
        name: "amount".to_string(),
        type_name: "DECIMAL".to_string(),
        scale: Some(json!(6)),
    };
    let large_integer_field = ResultColumn {
        name: "external_id".to_string(),
        type_name: "NUMBER".to_string(),
        scale: Some(json!(0)),
    };
    let missing_scale_field = ResultColumn {
        name: "metric".to_string(),
        type_name: "FIXED".to_string(),
        scale: None,
    };

    assert_eq!(
        parse_cell_value(&json!("12345678901234567890.123456"), &decimal_field),
        json!("12345678901234567890.123456")
    );
    assert_eq!(
        parse_cell_value(&json!("9007199254740993"), &large_integer_field),
        json!("9007199254740993")
    );
    assert_eq!(
        parse_cell_value(&json!("42"), &missing_scale_field),
        json!("42")
    );
}

#[test]
fn parse_cell_value_preserves_non_finite_float_text() {
    let float_field = ResultColumn {
        name: "ratio".to_string(),
        type_name: "FLOAT".to_string(),
        scale: None,
    };

    assert_eq!(parse_cell_value(&json!("1.25"), &float_field), json!(1.25));
    assert_eq!(parse_cell_value(&json!("NaN"), &float_field), json!("NaN"));
    assert_eq!(parse_cell_value(&json!("inf"), &float_field), json!("inf"));
    assert_eq!(
        parse_cell_value(&json!("-inf"), &float_field),
        json!("-inf")
    );
}

#[test]
fn normalize_preflight_relation_uppercases_safe_unquoted_segments() {
    let relation = normalize_preflight_relation("analytics.orders").expect("valid relation");
    assert_eq!(relation, "ANALYTICS.ORDERS");
}

#[test]
fn normalize_preflight_relation_rejects_injection() {
    let err = normalize_preflight_relation("orders;drop").expect_err("invalid relation");
    assert!(err.to_string().contains("Invalid relation"));
}

#[test]
fn preflight_statements_are_bounded_and_safe() {
    assert_eq!(
        catalog_preflight_statement("ANALYTICS"),
        "SHOW DATABASES STARTS WITH 'ANALYTICS' LIMIT 1"
    );
    assert_eq!(
        catalog_preflight_statement("ANALYTICS_REPORTING"),
        "SHOW DATABASES STARTS WITH 'ANALYTICS_REPORTING' LIMIT 1"
    );
    assert_eq!(
        schema_preflight_statement("ANALYTICS", "REPORTING"),
        "SHOW SCHEMAS IN DATABASE ANALYTICS STARTS WITH 'REPORTING' LIMIT 1"
    );
    assert_eq!(
        schema_preflight_statement("ANALYTICS", "REPORTING_SCHEMA"),
        "SHOW SCHEMAS IN DATABASE ANALYTICS STARTS WITH 'REPORTING_SCHEMA' LIMIT 1"
    );
    assert_eq!(
        relation_preflight_statement("ANALYTICS.REPORTING.ORDERS"),
        "SELECT 1 AS relation_access_check FROM ANALYTICS.REPORTING.ORDERS LIMIT 1"
    );
}

#[test]
fn schema_preflight_target_normalizes_default_catalog() {
    let target =
        resolve_schema_preflight_target(None, Some("analytics"), "reporting").expect("target");
    assert_eq!(target, ("ANALYTICS".to_string(), "REPORTING".to_string()));
}

#[test]
fn schema_preflight_target_rejects_unsafe_default_catalog() {
    let err = resolve_schema_preflight_target(None, Some("analytics;drop"), "reporting")
        .expect_err("unsafe default catalog should fail");
    assert!(err.to_string().contains("Invalid catalog identifier"));
}

#[test]
fn preflight_show_result_requires_exact_name_match() {
    let mut result = SnowflakeQueryResult {
        statement_id: "01".to_string(),
        state: "SUCCEEDED".to_string(),
        provider: "snowflake".to_string(),
        account_url: "https://example.snowflakecomputing.com".to_string(),
        warehouse: "COMPUTE_WH".to_string(),
        database: None,
        schema: None,
        role: None,
        columns: vec!["created_on".to_string(), "name".to_string()],
        column_types: vec!["TIMESTAMP".to_string(), "TEXT".to_string()],
        rows: vec![vec![Value::Null, json!("ANALYTICS_REPORTING_DEV")]],
        elapsed_ms: 1,
        fetched_chunks: 1,
        stats: SnowflakeQueryStats {
            total_row_count: Some(1),
            total_byte_count: None,
            total_chunk_count: Some(1),
        },
        truncated: false,
    };

    assert!(!preflight_show_result_has_exact_name(
        &result,
        "ANALYTICS_REPORTING"
    ));
    result.rows = vec![vec![Value::Null, json!("ANALYTICS_REPORTING")]];
    assert!(preflight_show_result_has_exact_name(
        &result,
        "analytics_reporting"
    ));
}
