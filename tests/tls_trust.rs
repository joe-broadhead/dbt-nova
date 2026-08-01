use std::ffi::{OsStr, OsString};
use std::sync::LazyLock;
use std::time::Duration;

use dbt_nova::warehouse::databricks::{DatabricksSqlClient, DatabricksSqlConfig};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::rustls::pki_types::pem::PemObject;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};

static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

const CA_CERT_PEM: &str = r"-----BEGIN CERTIFICATE-----
MIIBnTCCAUOgAwIBAgIUUJ3mcRyhBlr7CqofxGEv8dLb6nYwCgYIKoZIzj0EAwIw
GzEZMBcGA1UEAwwQZGJ0LW5vdmEtdGVzdC1jYTAgFw0yNjA4MDEyMzE5NThaGA8y
MTI2MDcwODIzMTk1OFowGzEZMBcGA1UEAwwQZGJ0LW5vdmEtdGVzdC1jYTBZMBMG
ByqGSM49AgEGCCqGSM49AwEHA0IABCiNGLNRCmUF4vR8owt8brU0p1Z4RhytBy1k
SuAS+cvextVJ+11E4C5msqSh+QGmGold6/jBtKGYriCjItdCBvSjYzBhMB0GA1Ud
DgQWBBSQJcLpU5JQ8IPR1ffMGCMkPArFkDAfBgNVHSMEGDAWgBSQJcLpU5JQ8IPR
1ffMGCMkPArFkDAPBgNVHRMBAf8EBTADAQH/MA4GA1UdDwEB/wQEAwIBBjAKBggq
hkjOPQQDAgNIADBFAiBOF+qta4kDwAs6QvU2bxR0iyI90moQneaO3WJFItDA4QIh
AIyN7kTJeH0IenKUpeN+Sxm7D21JKDPu+mCcCMK9sLp6
-----END CERTIFICATE-----
";

const SERVER_CERT_PEM: &str = r"-----BEGIN CERTIFICATE-----
MIIBxjCCAWygAwIBAgIUX4t6+TL6jDGZa1u/VvOAQmjNsjwwCgYIKoZIzj0EAwIw
GzEZMBcGA1UEAwwQZGJ0LW5vdmEtdGVzdC1jYTAgFw0yNjA4MDEyMzE5NThaGA8y
MTI2MDcwODIzMTk1OFowFDESMBAGA1UEAwwJbG9jYWxob3N0MFkwEwYHKoZIzj0C
AQYIKoZIzj0DAQcDQgAEml0YIW0+cs/TMwDnuXsvEUqSeGxA50zjynxHv4kuq2iX
p0jzAQbP7hMHq9/u/0M/nCmwQg9h7FwTygyBTenNcqOBkjCBjzAaBgNVHREEEzAR
gglsb2NhbGhvc3SHBH8AAAEwDAYDVR0TAQH/BAIwADAOBgNVHQ8BAf8EBAMCB4Aw
EwYDVR0lBAwwCgYIKwYBBQUHAwEwHQYDVR0OBBYEFHlDYJ2k3o1hFGuNVloTd2j/
LRs9MB8GA1UdIwQYMBaAFJAlwulTklDwg9HV98wYIyQ8CsWQMAoGCCqGSM49BAMC
A0gAMEUCIHh2ELxO1lE8bkkMtbKsvFPrx0NUqn3/rtIpr2TZb52aAiEAwJNduaCt
08H9Pcsjo+DXVpAzOek0mAtwqy5vLf35Iwo=
-----END CERTIFICATE-----
";

const SERVER_KEY_PEM: &str = r"-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgFsAUpKiCT3HyYEOS
0C6jKLdFrvbf3lMNgh+L1X8yUt+hRANCAASaXRghbT5yz9MzAOe5ey8RSpJ4bEDn
TOPKfEe/iS6raJenSPMBBs/uEwer3+7/Qz+cKbBCD2HsXBPKDIFN6c1y
-----END PRIVATE KEY-----
";

struct EnvGuard {
    key: &'static str,
    previous: Option<OsString>,
}

impl EnvGuard {
    fn set(key: &'static str, value: impl AsRef<OsStr>) -> Self {
        let previous = std::env::var_os(key);
        // SAFETY: this integration-test binary serializes environment mutation with ENV_LOCK.
        unsafe { std::env::set_var(key, value) };
        Self { key, previous }
    }

    fn remove(key: &'static str) -> Self {
        let previous = std::env::var_os(key);
        // SAFETY: this integration-test binary serializes environment mutation with ENV_LOCK.
        unsafe { std::env::remove_var(key) };
        Self { key, previous }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        // SAFETY: this integration-test binary serializes environment mutation with ENV_LOCK.
        unsafe {
            match &self.previous {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }
}

async fn spawn_tls_server(response_body: String) -> (String, JoinHandle<()>) {
    let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
    let certificate = CertificateDer::from_pem_slice(SERVER_CERT_PEM.as_bytes())
        .expect("valid server certificate fixture");
    let private_key = PrivateKeyDer::from_pem_slice(SERVER_KEY_PEM.as_bytes())
        .expect("valid server private key fixture");
    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![certificate], private_key)
        .expect("matching TLS certificate and key");
    let acceptor = TlsAcceptor::from(std::sync::Arc::new(config));
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind TLS test listener");
    let address = listener.local_addr().expect("TLS test listener address");

    let task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept TLS test client");
        let Ok(mut stream) = acceptor.accept(stream).await else {
            return;
        };

        let mut request = Vec::new();
        loop {
            let mut chunk = [0_u8; 1024];
            let read = stream.read(&mut chunk).await.expect("read HTTPS request");
            if read == 0 {
                return;
            }
            request.extend_from_slice(&chunk[..read]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }

        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write HTTPS response");
        stream.shutdown().await.expect("close HTTPS response");
    });

    (
        format!(
            "https://127.0.0.1:{address_port}",
            address_port = address.port()
        ),
        task,
    )
}

#[tokio::test(flavor = "current_thread")]
async fn ssl_cert_file_adds_private_ca_without_disabling_verification() {
    let _env_lock = ENV_LOCK.lock().await;
    let _ssl_cert_file = EnvGuard::remove("SSL_CERT_FILE");
    let _ssl_cert_dir = EnvGuard::remove("SSL_CERT_DIR");

    let (untrusted_url, untrusted_server) = spawn_tls_server("ok".to_string()).await;
    let untrusted_result = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .expect("default client")
        .get(untrusted_url)
        .send()
        .await;
    assert!(
        untrusted_result.is_err(),
        "the private CA must not be trusted without explicit configuration"
    );
    untrusted_server.await.expect("untrusted TLS server task");

    let temp = TempDir::new().expect("temporary CA directory");
    let ca_path = temp.path().join("corporate-ca.pem");
    std::fs::write(&ca_path, CA_CERT_PEM).expect("write private CA bundle");
    let _ssl_cert_file = EnvGuard::set("SSL_CERT_FILE", &ca_path);

    let (trusted_url, trusted_server) = spawn_tls_server("ok".to_string()).await;
    let response = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .expect("client with private CA")
        .get(trusted_url)
        .send()
        .await
        .expect("private CA should be trusted through SSL_CERT_FILE");

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.text().await.expect("HTTPS response body"), "ok");
    trusted_server.await.expect("trusted TLS server task");
}

#[tokio::test(flavor = "current_thread")]
async fn requests_ca_bundle_reaches_databricks_execute_path() {
    let _env_lock = ENV_LOCK.lock().await;
    let _ssl_cert_file = EnvGuard::remove("SSL_CERT_FILE");
    let _ssl_cert_dir = EnvGuard::remove("SSL_CERT_DIR");
    let _requests_ca_bundle = EnvGuard::remove("REQUESTS_CA_BUNDLE");
    let _curl_ca_bundle = EnvGuard::remove("CURL_CA_BUNDLE");

    let temp = TempDir::new().expect("temporary CA directory");
    let ca_path = temp.path().join("corporate-ca.pem");
    std::fs::write(&ca_path, CA_CERT_PEM).expect("write private CA bundle");
    let _requests_ca_bundle = EnvGuard::set("REQUESTS_CA_BUNDLE", &ca_path);

    let response_body = serde_json::json!({
        "statement_id": "stmt-corporate-ca",
        "status": { "state": "SUCCEEDED" },
        "manifest": {
            "schema": {
                "columns": [{ "name": "smoke_test", "type_name": "INT" }]
            },
            "chunks": [{ "chunk_index": 0 }],
            "truncated": false,
            "total_chunk_count": 1,
            "total_row_count": 1,
            "total_byte_count": 1
        },
        "result": {
            "chunk_index": 0,
            "data_array": [["1"]]
        }
    })
    .to_string();
    let (host, server) = spawn_tls_server(response_body).await;
    let client = DatabricksSqlClient::new(DatabricksSqlConfig {
        host,
        token: "test-token".to_string(),
        warehouse_id: "test-warehouse".to_string(),
        timeout: Duration::from_secs(3),
        default_wait_timeout_s: 5,
        poll_interval: Duration::from_millis(5),
        max_poll: Duration::from_secs(1),
        max_get_retries: 0,
    })
    .expect("Databricks client with custom CA bundle");

    let result = client
        .query("select 1 as smoke_test")
        .await
        .expect("Databricks request should trust REQUESTS_CA_BUNDLE");

    assert_eq!(result.columns, ["smoke_test"]);
    assert_eq!(result.rows.len(), 1);
    server.await.expect("Databricks TLS server task");
}
