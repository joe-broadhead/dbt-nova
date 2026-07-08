use super::*;

pub(super) enum SnowflakeAuthorization {
    Bearer {
        token: String,
        token_type: &'static str,
    },
    Session {
        token: String,
    },
}

#[derive(Serialize)]
pub(super) struct ExternalBrowserAuthenticatorRequest<'a> {
    pub(super) data: ExternalBrowserAuthenticatorRequestData<'a>,
}

#[derive(Serialize)]
pub(super) struct ExternalBrowserAuthenticatorRequestData<'a> {
    #[serde(rename = "CLIENT_APP_ID")]
    pub(super) client_app_id: &'static str,
    #[serde(rename = "CLIENT_APP_VERSION")]
    pub(super) client_app_version: &'static str,
    #[serde(rename = "ACCOUNT_NAME")]
    pub(super) account_name: &'a str,
    #[serde(rename = "LOGIN_NAME")]
    pub(super) login_name: &'a str,
    #[serde(rename = "AUTHENTICATOR")]
    pub(super) authenticator: &'static str,
    #[serde(rename = "BROWSER_MODE_REDIRECT_PORT")]
    pub(super) browser_mode_redirect_port: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct ExternalBrowserAuthenticatorResponse {
    #[serde(default)]
    pub(super) success: Option<bool>,
    #[serde(default)]
    pub(super) data: Option<ExternalBrowserAuthenticatorResponseData>,
    #[serde(default)]
    pub(super) code: Option<String>,
    #[serde(default)]
    pub(super) message: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ExternalBrowserAuthenticatorResponseData {
    #[serde(rename = "ssoUrl", alias = "SSO_URL", default)]
    pub(super) sso_url: Option<String>,
    #[serde(rename = "proofKey", alias = "PROOF_KEY", default)]
    pub(super) proof_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ExternalBrowserAuthenticatorInfo {
    pub(super) sso_url: String,
    pub(super) proof_key: String,
}

#[derive(Serialize)]
pub(super) struct ExternalBrowserLoginRequest<'a> {
    pub(super) data: ExternalBrowserLoginRequestData<'a>,
}

#[derive(Serialize)]
pub(super) struct ExternalBrowserLoginRequestData<'a> {
    #[serde(rename = "CLIENT_APP_ID")]
    pub(super) client_app_id: &'static str,
    #[serde(rename = "CLIENT_APP_VERSION")]
    pub(super) client_app_version: &'static str,
    #[serde(rename = "ACCOUNT_NAME")]
    pub(super) account_name: &'a str,
    #[serde(rename = "LOGIN_NAME")]
    pub(super) login_name: &'a str,
    #[serde(rename = "AUTHENTICATOR")]
    pub(super) authenticator: &'static str,
    #[serde(rename = "TOKEN")]
    pub(super) token: &'a str,
    #[serde(rename = "PROOF_KEY", skip_serializing_if = "Option::is_none")]
    pub(super) proof_key: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ExternalBrowserLoginResponse {
    #[serde(default)]
    pub(super) success: Option<bool>,
    #[serde(default)]
    pub(super) data: Option<ExternalBrowserLoginResponseData>,
    #[serde(default)]
    pub(super) code: Option<String>,
    #[serde(default)]
    pub(super) message: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ExternalBrowserLoginResponseData {
    #[serde(rename = "token", alias = "sessionToken", default)]
    pub(super) token: Option<String>,
    #[serde(rename = "validityInSeconds", alias = "validity_in_seconds", default)]
    pub(super) validity_in_seconds: Option<Value>,
    #[serde(rename = "masterToken", alias = "master_token", default)]
    pub(super) master_token: Option<String>,
    #[serde(
        rename = "masterValidityInSeconds",
        alias = "master_validity_in_seconds",
        default
    )]
    pub(super) master_validity_in_seconds: Option<Value>,
    #[serde(rename = "idToken", alias = "id_token", default)]
    pub(super) id_token: Option<String>,
    #[serde(
        rename = "idTokenValidityInSeconds",
        alias = "id_token_validity_in_seconds",
        default
    )]
    pub(super) id_token_validity_in_seconds: Option<Value>,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct BrowserCallback {
    pub(super) token: String,
    pub(super) proof_key: Option<String>,
    pub(super) origin: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum BrowserCallbackRequest {
    Callback(BrowserCallback),
    Preflight(BrowserCallbackPreflight),
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct BrowserCallbackPreflight {
    pub(super) origin: Option<String>,
    pub(super) requested_headers: Option<String>,
}

pub(super) fn decode_external_browser_authenticator_response(
    status: StatusCode,
    body: &str,
) -> Result<ExternalBrowserAuthenticatorInfo> {
    let response = decode_json_response::<ExternalBrowserAuthenticatorResponse>(status, body)?;
    if response.success == Some(false) {
        return Err(snowflake_err(format!(
            "external browser authenticator request failed: {}",
            summarize_snowflake_auth_response(
                response.code.as_deref(),
                response.message.as_deref()
            )
        )));
    }
    let data = response
        .data
        .ok_or_else(|| snowflake_err("external browser authenticator response missing data"))?;
    let sso_url = data
        .sso_url
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| snowflake_err("external browser authenticator response missing ssoUrl"))?;
    let proof_key = data
        .proof_key
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| snowflake_err("external browser authenticator response missing proofKey"))?;
    Ok(ExternalBrowserAuthenticatorInfo { sso_url, proof_key })
}

pub(super) fn decode_external_browser_login_response(
    status: StatusCode,
    body: &str,
) -> Result<SnowflakeSession> {
    let response = decode_json_response::<ExternalBrowserLoginResponse>(status, body)?;
    if response.success == Some(false) {
        return Err(snowflake_err(format!(
            "external browser login failed: {}",
            summarize_snowflake_auth_response(
                response.code.as_deref(),
                response.message.as_deref()
            )
        )));
    }
    let data = response
        .data
        .ok_or_else(|| snowflake_err("external browser login response missing data"))?;
    let token = data
        .token
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| snowflake_err("external browser login response missing session token"))?;
    Ok(SnowflakeSession {
        token,
        expires_at: expires_at_from_validity(data.validity_in_seconds.as_ref()),
        master_token: data.master_token.filter(|value| !value.trim().is_empty()),
        master_expires_at: expires_at_from_validity(data.master_validity_in_seconds.as_ref()),
        id_token: data.id_token.filter(|value| !value.trim().is_empty()),
        id_token_expires_at: expires_at_from_validity(data.id_token_validity_in_seconds.as_ref()),
    })
}

pub(super) fn summarize_snowflake_auth_response(
    code: Option<&str>,
    _message: Option<&str>,
) -> String {
    let code = code.unwrap_or("unknown");
    format!("{code}: request failed; check Snowflake externalbrowser configuration")
}

pub(super) fn expires_at_from_validity(value: Option<&Value>) -> Option<Instant> {
    parse_optional_u64(value)
        .and_then(|seconds| Instant::now().checked_add(Duration::from_secs(seconds)))
}

pub(super) async fn bind_browser_callback_listener(
    callback_port: Option<u16>,
) -> Result<TcpListener> {
    TcpListener::bind(("127.0.0.1", callback_port.unwrap_or(0)))
        .await
        .map_err(|err| snowflake_err(format!("failed to bind external browser callback: {err}")))
}

pub(super) async fn receive_browser_callback(
    listener: TcpListener,
    expected_proof_key: &str,
    auth_timeout: Duration,
) -> Result<BrowserCallback> {
    let started_at = Instant::now();
    loop {
        let Some(remaining) = auth_timeout.checked_sub(started_at.elapsed()) else {
            return Err(snowflake_err(
                "timed out waiting for Snowflake browser SSO callback",
            ));
        };
        let accept = timeout(remaining, listener.accept())
            .await
            .map_err(|_| snowflake_err("timed out waiting for Snowflake browser SSO callback"))?;
        let (mut socket, peer_addr) = accept
            .map_err(|err| snowflake_err(format!("failed to accept browser callback: {err}")))?;
        if !peer_addr.ip().is_loopback() {
            let _ = write_browser_callback_response(&mut socket, false, None).await;
            return Err(snowflake_err(
                "external browser callback must originate from loopback",
            ));
        }

        let parsed = read_browser_callback_request(&mut socket, remaining)
            .await
            .and_then(|request| parse_browser_callback_request(&request, expected_proof_key));
        match parsed {
            Ok(BrowserCallbackRequest::Callback(callback)) => {
                let _ =
                    write_browser_callback_response(&mut socket, true, callback.origin.as_deref())
                        .await;
                return Ok(callback);
            }
            Ok(BrowserCallbackRequest::Preflight(preflight)) => {
                let _ = write_browser_preflight_response(&mut socket, &preflight).await;
            }
            Err(err) => {
                let _ = write_browser_callback_response(&mut socket, false, None).await;
                return Err(err);
            }
        }
    }
}

pub(super) async fn read_browser_callback_request(
    socket: &mut tokio::net::TcpStream,
    auth_timeout: Duration,
) -> Result<String> {
    let mut request = Vec::with_capacity(1024);
    let mut buffer = [0u8; 1024];
    let mut expected_length = None;
    let deadline = Instant::now()
        .checked_add(auth_timeout)
        .ok_or_else(|| snowflake_err("Snowflake browser SSO callback timeout is too large"))?;
    loop {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return Err(snowflake_err(
                "timed out reading Snowflake browser SSO callback",
            ));
        };
        let bytes_read = timeout(remaining, socket.read(&mut buffer))
            .await
            .map_err(|_| snowflake_err("timed out reading Snowflake browser SSO callback"))?
            .map_err(|err| snowflake_err(format!("failed to read browser callback: {err}")))?;
        if bytes_read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..bytes_read]);
        if request.len() > MAX_BROWSER_CALLBACK_REQUEST_BYTES {
            return Err(snowflake_err(
                "external browser callback request is too large",
            ));
        }
        if expected_length.is_none()
            && let Some(header_end) = browser_callback_header_end(&request)
        {
            let headers = std::str::from_utf8(&request[..header_end])
                .map_err(|_| snowflake_err("external browser callback was not valid UTF-8"))?;
            let content_length = parse_content_length_header(headers)?;
            let total_length = header_end
                .checked_add(4)
                .and_then(|value| value.checked_add(content_length))
                .ok_or_else(|| snowflake_err("external browser callback request is too large"))?;
            if total_length > MAX_BROWSER_CALLBACK_REQUEST_BYTES {
                return Err(snowflake_err(
                    "external browser callback request is too large",
                ));
            }
            expected_length = Some(total_length);
        }
        if expected_length.is_some_and(|length| request.len() >= length) {
            break;
        }
    }

    String::from_utf8(request)
        .map_err(|_| snowflake_err("external browser callback was not valid UTF-8"))
}

pub(super) async fn write_browser_callback_response(
    socket: &mut tokio::net::TcpStream,
    ok: bool,
    origin: Option<&str>,
) -> Result<()> {
    let body = if ok {
        "Snowflake authentication complete. You can close this tab."
    } else {
        "Snowflake authentication failed. Return to dbt-nova for details."
    };
    let status = if ok { "200 OK" } else { "400 Bad Request" };
    let cors = origin.map_or_else(String::new, |origin| {
        format!("Access-Control-Allow-Origin: {origin}\r\nVary: Origin\r\n")
    });
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\n{cors}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    socket
        .write_all(response.as_bytes())
        .await
        .map_err(|err| snowflake_err(format!("failed to write browser callback response: {err}")))
}

pub(super) async fn write_browser_preflight_response(
    socket: &mut tokio::net::TcpStream,
    preflight: &BrowserCallbackPreflight,
) -> Result<()> {
    let origin = preflight.origin.as_deref().unwrap_or("null");
    let requested_headers = preflight
        .requested_headers
        .as_deref()
        .unwrap_or("content-type");
    let response = format!(
        "HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: {origin}\r\nAccess-Control-Allow-Methods: POST, GET, OPTIONS\r\nAccess-Control-Allow-Headers: {requested_headers}\r\nAccess-Control-Max-Age: 86400\r\nVary: Origin\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    socket
        .write_all(response.as_bytes())
        .await
        .map_err(|err| snowflake_err(format!("failed to write browser callback response: {err}")))
}

pub(super) fn parse_browser_callback_request(
    request: &str,
    expected_proof_key: &str,
) -> Result<BrowserCallbackRequest> {
    let request_line = request
        .lines()
        .next()
        .ok_or_else(|| snowflake_err("external browser callback was empty"))?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default();
    let version = parts.next().unwrap_or_default();
    if !matches!(method, "GET" | "POST" | "OPTIONS")
        || !version.starts_with("HTTP/")
        || parts.next().is_some()
    {
        return Err(snowflake_err(
            "external browser callback must be an HTTP GET or POST request",
        ));
    }
    if !target.starts_with('/') {
        return Err(snowflake_err(
            "external browser callback target must be a rooted path",
        ));
    }
    let url = Url::parse(&format!("http://127.0.0.1{target}"))
        .map_err(|err| snowflake_err(format!("invalid external browser callback URL: {err}")))?;
    if url.path() != "/" {
        return Err(snowflake_err(
            "external browser callback path must be the root path",
        ));
    }

    if method == "OPTIONS" {
        return Ok(BrowserCallbackRequest::Preflight(
            BrowserCallbackPreflight {
                origin: request_header_value(request, "Origin").map(str::to_string),
                requested_headers: request_header_value(request, "Access-Control-Request-Headers")
                    .map(str::to_string),
            },
        ));
    }

    let (token, proof_key) = if method == "GET" {
        token_and_proof_key_from_pairs(
            url.query_pairs()
                .map(|(key, value)| (key.into_owned(), value.into_owned())),
        )
    } else {
        token_and_proof_key_from_post_body(
            browser_callback_body(request),
            request_header_value(request, "Content-Type").unwrap_or_default(),
        )?
    };

    if proof_key
        .as_deref()
        .is_some_and(|value| value != expected_proof_key)
    {
        return Err(snowflake_err(
            "external browser callback proof key did not match",
        ));
    }

    let token = token
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| snowflake_err("external browser callback missing token"))?;
    Ok(BrowserCallbackRequest::Callback(BrowserCallback {
        token,
        proof_key,
        origin: request_header_value(request, "Origin").map(str::to_string),
    }))
}

pub(super) fn browser_callback_header_end(request: &[u8]) -> Option<usize> {
    request.windows(4).position(|window| window == b"\r\n\r\n")
}

pub(super) fn parse_content_length_header(headers: &str) -> Result<usize> {
    let Some(value) = request_header_value(headers, "Content-Length") else {
        return Ok(0);
    };
    value.parse::<usize>().map_err(|err| {
        snowflake_err(format!(
            "invalid external browser callback Content-Length: {err}"
        ))
    })
}

pub(super) fn request_header_value<'a>(request: &'a str, name: &str) -> Option<&'a str> {
    request.lines().skip(1).find_map(|line| {
        let (header, value) = line.split_once(':')?;
        header
            .trim()
            .eq_ignore_ascii_case(name)
            .then_some(value.trim())
    })
}

pub(super) fn browser_callback_body(request: &str) -> &str {
    request
        .split_once("\r\n\r\n")
        .map(|(_, body)| body)
        .unwrap_or_default()
}

pub(super) fn token_and_proof_key_from_post_body(
    body: &str,
    content_type: &str,
) -> Result<(Option<String>, Option<String>)> {
    if content_type
        .to_ascii_lowercase()
        .starts_with("application/json")
    {
        let parsed = serde_json::from_str::<Value>(body)
            .map_err(|err| snowflake_err(format!("invalid browser callback JSON body: {err}")))?;
        let token = parsed
            .get("token")
            .and_then(Value::as_str)
            .map(str::to_string);
        let proof_key = parsed
            .get("proofKey")
            .or_else(|| parsed.get("proof_key"))
            .or_else(|| parsed.get("PROOF_KEY"))
            .and_then(Value::as_str)
            .map(str::to_string);
        return Ok((token, proof_key));
    }

    let form_url = Url::parse(&format!("http://127.0.0.1/?{body}")).map_err(|err| {
        snowflake_err(format!("invalid browser callback form-encoded body: {err}"))
    })?;
    Ok(token_and_proof_key_from_pairs(form_url.query_pairs().map(
        |(key, value)| (key.into_owned(), value.into_owned()),
    )))
}

pub(super) fn token_and_proof_key_from_pairs<I>(pairs: I) -> (Option<String>, Option<String>)
where
    I: IntoIterator<Item = (String, String)>,
{
    let mut token = None;
    let mut proof_key = None;
    for (key, value) in pairs {
        match key.as_str() {
            "token" => token = Some(value),
            "proofKey" | "proof_key" | "PROOF_KEY" => proof_key = Some(value),
            _ => {}
        }
    }
    (token, proof_key)
}

pub(super) fn open_external_browser_url(url: &str, open_browser: bool) {
    if !open_browser {
        eprintln!("Open this Snowflake SSO URL to authenticate dbt-nova:\n{url}");
        return;
    }

    let mut command = browser_open_command(url);
    if let Err(err) = command.spawn() {
        warn!(
            error = %err,
            "failed to open system browser for Snowflake externalbrowser auth"
        );
        eprintln!("Open this Snowflake SSO URL to authenticate dbt-nova:\n{url}");
    }
}

pub(super) fn browser_open_command(url: &str) -> Command {
    #[cfg(target_os = "macos")]
    {
        let mut command = Command::new("open");
        command.arg(url);
        command
    }
    #[cfg(target_os = "windows")]
    {
        let mut command = Command::new("rundll32");
        command.args(["url.dll,FileProtocolHandler", url]);
        command
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        let mut command = Command::new("xdg-open");
        command.arg(url);
        command
    }
}

pub(super) fn normalize_account_url(input: &str) -> Result<String> {
    let trimmed = input.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(DbtNovaError::InvalidParams(
            "Snowflake account URL cannot be empty".to_string(),
        ));
    }
    let url = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    };
    let parsed = Url::parse(&url).map_err(|err| {
        DbtNovaError::InvalidParams(format!("Invalid Snowflake account URL '{input}': {err}"))
    })?;
    if parsed.scheme() != "https" {
        return Err(DbtNovaError::InvalidParams(
            "Snowflake account URL must use https://".to_string(),
        ));
    }
    let host = parsed.host_str().ok_or_else(|| {
        DbtNovaError::InvalidParams("Snowflake account URL must include a host".to_string())
    })?;
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(DbtNovaError::InvalidParams(
            "Snowflake account URL must not include credentials".to_string(),
        ));
    }
    if parsed.path() != "/" || parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(DbtNovaError::InvalidParams(
            "Snowflake account URL must not include a path, query, or fragment".to_string(),
        ));
    }

    let mut normalized = format!("https://{host}");
    if let Some(port) = parsed.port() {
        normalized.push(':');
        normalized.push_str(&port.to_string());
    }
    Ok(normalized)
}

pub(super) fn resolve_auth_from_env(
    account: Option<String>,
    base_url: &str,
) -> Result<SnowflakeAuthConfig> {
    let auth = read_optional_env("DBT_NOVA_SNOWFLAKE_AUTH")
        .map(|value| value.to_ascii_lowercase())
        .or_else(|| {
            if read_optional_env("DBT_NOVA_SNOWFLAKE_PAT").is_some() {
                Some("pat".to_string())
            } else if read_optional_env("DBT_NOVA_SNOWFLAKE_OAUTH_TOKEN").is_some() {
                Some("oauth".to_string())
            } else {
                Some("keypair".to_string())
            }
        })
        .unwrap_or_else(|| "keypair".to_string());

    resolve_auth_from_mode(&auth, account, base_url)
}

pub(super) fn resolve_auth_from_mode(
    auth: &str,
    account: Option<String>,
    base_url: &str,
) -> Result<SnowflakeAuthConfig> {
    match auth {
        "oauth" => Ok(SnowflakeAuthConfig::OAuth {
            token: read_required_env(
                "DBT_NOVA_SNOWFLAKE_OAUTH_TOKEN",
                "DBT_NOVA_SNOWFLAKE_OAUTH_TOKEN is required for Snowflake OAuth auth",
            )?,
        }),
        "pat" | "programmatic_access_token" => Ok(SnowflakeAuthConfig::ProgrammaticAccessToken {
            token: read_required_env(
                "DBT_NOVA_SNOWFLAKE_PAT",
                "DBT_NOVA_SNOWFLAKE_PAT is required for Snowflake PAT auth",
            )?,
        }),
        "externalbrowser" | "external_browser" | "browser" => {
            ensure_external_browser_allowed_from_env(streamable_http_env_binds_non_loopback())?;
            let timeout = Duration::from_secs(
                env_u64(
                    "DBT_NOVA_SNOWFLAKE_EXTERNAL_BROWSER_TIMEOUT_S",
                    DEFAULT_EXTERNAL_BROWSER_TIMEOUT_SECONDS,
                )
                .max(1),
            );
            let open_browser = env_bool("DBT_NOVA_SNOWFLAKE_EXTERNAL_BROWSER_OPEN", true)?;
            let callback_port =
                env_u16_optional("DBT_NOVA_SNOWFLAKE_EXTERNAL_BROWSER_CALLBACK_PORT")?;
            build_external_browser_auth_config(
                base_url,
                account,
                read_optional_env("DBT_NOVA_SNOWFLAKE_USER"),
                timeout,
                open_browser,
                callback_port,
            )
        }
        "keypair" | "snowflake_jwt" => {
            if read_optional_env("DBT_NOVA_SNOWFLAKE_PRIVATE_KEY_PASSPHRASE").is_some() {
                return Err(DbtNovaError::InvalidParams(
                    "Encrypted Snowflake private keys are not supported by dbt-nova yet; provide an unencrypted PKCS#8 or RSA PEM key".to_string(),
                ));
            }
            let user = read_required_env(
                "DBT_NOVA_SNOWFLAKE_USER",
                "DBT_NOVA_SNOWFLAKE_USER is required for Snowflake keypair auth",
            )?;
            let account_identifier = read_optional_env("DBT_NOVA_SNOWFLAKE_JWT_ACCOUNT")
                .or(account)
                .ok_or_else(|| {
                    DbtNovaError::InvalidParams(
                        "DBT_NOVA_SNOWFLAKE_JWT_ACCOUNT or DBT_NOVA_SNOWFLAKE_ACCOUNT is required for Snowflake keypair auth".to_string(),
                    )
                })?;
            let private_key_pem = resolve_private_key_pem()?;
            Ok(SnowflakeAuthConfig::KeyPair {
                user,
                account_identifier: normalize_jwt_identifier(&account_identifier),
                private_key_pem,
            })
        }
        other => Err(DbtNovaError::InvalidParams(format!(
            "Unsupported DBT_NOVA_SNOWFLAKE_AUTH '{other}' (expected {SUPPORTED_SNOWFLAKE_AUTH_MODES})"
        ))),
    }
}

pub(super) fn validate_external_browser_runtime(config: &DbtNovaConfig) -> Result<()> {
    validate_external_browser_runtime_for_auth(
        config,
        read_optional_env("DBT_NOVA_SNOWFLAKE_AUTH").as_deref(),
    )
}

pub(super) fn validate_external_browser_runtime_for_auth(
    config: &DbtNovaConfig,
    auth_mode: Option<&str>,
) -> Result<()> {
    validate_external_browser_runtime_for_auth_with_ci(config, auth_mode, env_bool("CI", false)?)
}

pub(super) fn validate_external_browser_runtime_for_auth_with_ci(
    config: &DbtNovaConfig,
    auth_mode: Option<&str>,
    running_in_ci: bool,
) -> Result<()> {
    if auth_mode.is_some_and(auth_mode_is_external_browser) {
        ensure_external_browser_allowed(config.http_transport_binds_non_loopback(), running_in_ci)?;
    }
    Ok(())
}

pub(super) fn auth_mode_is_external_browser(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "externalbrowser" | "external_browser" | "browser"
    )
}

pub(super) fn build_external_browser_auth_config(
    base_url: &str,
    account: Option<String>,
    user: Option<String>,
    timeout: Duration,
    open_browser: bool,
    callback_port: Option<u16>,
) -> Result<SnowflakeAuthConfig> {
    let user = user
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            DbtNovaError::InvalidParams(
                "DBT_NOVA_SNOWFLAKE_USER is required for Snowflake externalbrowser auth"
                    .to_string(),
            )
        })?;
    let account_identifier = account
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            DbtNovaError::InvalidParams(
                "DBT_NOVA_SNOWFLAKE_ACCOUNT is required for Snowflake externalbrowser auth; DBT_NOVA_SNOWFLAKE_ACCOUNT_URL alone is not enough for browser SSO login".to_string(),
            )
        })?;
    let session_cache =
        external_browser_session_cache(base_url, &account_identifier, &user, callback_port);
    Ok(SnowflakeAuthConfig::ExternalBrowser {
        user,
        account_identifier,
        timeout,
        open_browser,
        callback_port,
        session_cache,
    })
}

pub(super) fn external_browser_session_cache(
    base_url: &str,
    account_identifier: &str,
    user: &str,
    callback_port: Option<u16>,
) -> ExternalBrowserSessionCache {
    let key = format!(
        "{}|{}|{}|{}",
        base_url.to_ascii_lowercase(),
        account_identifier.to_ascii_lowercase(),
        user.to_ascii_lowercase(),
        callback_port.map_or_else(|| "ephemeral".to_string(), |port| port.to_string())
    );
    let caches = EXTERNAL_BROWSER_SESSION_CACHES.get_or_init(|| StdMutex::new(HashMap::new()));
    let mut caches = match caches.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    caches
        .entry(key)
        .or_insert_with(|| Arc::new(TokioMutex::new(None)))
        .clone()
}

pub(super) fn ensure_external_browser_allowed_from_env(non_loopback_http_bind: bool) -> Result<()> {
    ensure_external_browser_allowed(non_loopback_http_bind, env_bool("CI", false)?)
}

pub(super) fn ensure_external_browser_allowed(
    non_loopback_http_bind: bool,
    running_in_ci: bool,
) -> Result<()> {
    if running_in_ci {
        return Err(DbtNovaError::InvalidParams(
            "Snowflake externalbrowser auth is interactive and cannot run in CI; use keypair, oauth, or pat auth".to_string(),
        ));
    }
    if non_loopback_http_bind {
        return Err(DbtNovaError::InvalidParams(
            "Snowflake externalbrowser auth is local-only and cannot be used with non-loopback streamable HTTP binds; use keypair, oauth, or pat auth for hosted deployments".to_string(),
        ));
    }
    Ok(())
}

pub(super) fn streamable_http_env_binds_non_loopback() -> bool {
    let transport = read_optional_env("DBT_NOVA_SERVER_TRANSPORT")
        .map_or_else(|| "stdio".to_string(), |value| value.to_ascii_lowercase());
    if !matches!(
        transport.as_str(),
        "streamable_http" | "streamable-http" | "http"
    ) {
        return false;
    }
    let host = read_optional_env("DBT_NOVA_HTTP_HOST").unwrap_or_else(|| {
        if read_optional_env("PORT").is_some() {
            "0.0.0.0".to_string()
        } else {
            "127.0.0.1".to_string()
        }
    });
    !is_loopback_host(&host)
}

pub(super) fn is_loopback_host(host: &str) -> bool {
    let normalized = host
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    matches!(normalized.as_str(), "localhost" | "127.0.0.1" | "::1")
}

pub(super) fn resolve_private_key_pem() -> Result<String> {
    if let Some(value) = read_optional_env("DBT_NOVA_SNOWFLAKE_PRIVATE_KEY_PEM") {
        if value.contains('\n') {
            return Ok(value);
        }
        return Ok(value.replace("\\n", "\n"));
    }

    let path = read_required_env(
        "DBT_NOVA_SNOWFLAKE_PRIVATE_KEY_PATH",
        "DBT_NOVA_SNOWFLAKE_PRIVATE_KEY_PATH or DBT_NOVA_SNOWFLAKE_PRIVATE_KEY_PEM is required for Snowflake keypair auth",
    )?;
    std::fs::read_to_string(&path).map_err(|err| {
        DbtNovaError::InvalidParams(format!(
            "Failed to read DBT_NOVA_SNOWFLAKE_PRIVATE_KEY_PATH '{path}': {err}"
        ))
    })
}

pub(super) fn normalize_jwt_identifier(value: &str) -> String {
    strip_locator_region_suffix(value.trim())
        .replace('.', "-")
        .to_ascii_uppercase()
}

pub(super) fn strip_locator_region_suffix(value: &str) -> &str {
    let mut segments = value.split('.');
    let Some(locator) = segments.next() else {
        return value;
    };
    let suffix: Vec<&str> = segments.collect();
    if suffix.is_empty() {
        return value;
    }

    let last = suffix
        .last()
        .map(|segment| segment.to_ascii_lowercase())
        .unwrap_or_default();
    let region_segments = if matches!(last.as_str(), "aws" | "azure" | "gcp") {
        &suffix[..suffix.len().saturating_sub(1)]
    } else {
        suffix.as_slice()
    };

    let region_segments = match region_segments {
        ["fhplus" | "dod", rest @ ..] => rest,
        other => other,
    };

    let has_explicit_locator_suffix = suffix.len() > 1;
    if region_segments.len() == 1
        && looks_like_snowflake_region(region_segments[0])
        && (has_explicit_locator_suffix || looks_like_generated_account_locator(locator))
    {
        locator
    } else {
        value
    }
}

pub(super) fn looks_like_generated_account_locator(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    bytes.len() == 7
        && bytes[..2].iter().all(u8::is_ascii_alphabetic)
        && bytes[2..].iter().all(u8::is_ascii_digit)
}

pub(super) fn looks_like_snowflake_region(segment: &str) -> bool {
    let region = segment.to_ascii_lowercase();
    let has_region_shape = region.contains('-') || region.chars().any(|ch| ch.is_ascii_digit());
    let has_region_hint = [
        "af",
        "ap",
        "asia",
        "au",
        "australia",
        "ca",
        "canada",
        "central",
        "cn",
        "east",
        "eu",
        "europe",
        "france",
        "germany",
        "il",
        "india",
        "japan",
        "korea",
        "me",
        "north",
        "norway",
        "sa",
        "south",
        "sweden",
        "switzerland",
        "uae",
        "uk",
        "us",
        "west",
    ]
    .iter()
    .any(|hint| {
        region == *hint
            || region.starts_with(&format!("{hint}-"))
            || region.ends_with(&format!("-{hint}"))
            || region.contains(&format!("-{hint}-"))
    });

    has_region_shape && has_region_hint
}

#[derive(Serialize)]
pub(super) struct SnowflakeJwtClaims {
    pub(super) iss: String,
    pub(super) sub: String,
    pub(super) iat: u64,
    pub(super) exp: u64,
}

pub(super) fn generate_keypair_jwt(
    account_identifier: &str,
    user: &str,
    private_key_pem: &str,
) -> Result<String> {
    let fingerprint = public_key_fingerprint(private_key_pem)?;
    let account = normalize_jwt_identifier(account_identifier);
    let user = user.trim().to_ascii_uppercase();
    let qualified_user = format!("{account}.{user}");
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| snowflake_err(format!("system clock before UNIX epoch: {err}")))?
        .as_secs();
    let claims = SnowflakeJwtClaims {
        iss: format!("{qualified_user}.{fingerprint}"),
        sub: qualified_user,
        iat: now,
        exp: now.saturating_add(DEFAULT_JWT_LIFETIME_SECONDS),
    };
    let key = EncodingKey::from_rsa_pem(private_key_pem.as_bytes())
        .map_err(|err| snowflake_err(format!("failed to parse Snowflake private key: {err}")))?;
    jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &key)
        .map_err(|err| snowflake_err(format!("failed to generate Snowflake JWT: {err}")))
}

pub(super) fn public_key_fingerprint(private_key_pem: &str) -> Result<String> {
    let rsa_private_der = rsa_private_key_der_from_pem(private_key_pem)?;
    let (modulus, exponent) = rsa_public_components_from_private_der(&rsa_private_der)?;
    let public_key_der = rsa_public_spki_der(modulus, exponent)?;
    let digest = Sha256::digest(&public_key_der);
    Ok(format!(
        "SHA256:{}",
        base64::engine::general_purpose::STANDARD.encode(digest)
    ))
}

pub(super) fn rsa_private_key_der_from_pem(private_key_pem: &str) -> Result<Vec<u8>> {
    let parsed = pem::parse(private_key_pem.as_bytes())
        .map_err(|err| snowflake_err(format!("failed to parse private key PEM: {err}")))?;
    match parsed.tag() {
        "RSA PRIVATE KEY" => Ok(parsed.into_contents()),
        "PRIVATE KEY" => extract_first_octet_string(&parsed.into_contents()),
        "ENCRYPTED PRIVATE KEY" => Err(DbtNovaError::InvalidParams(
            "Encrypted Snowflake private keys are not supported by dbt-nova yet; provide an unencrypted PKCS#8 or RSA PEM key".to_string(),
        )),
        tag => Err(DbtNovaError::InvalidParams(format!(
            "Unsupported Snowflake private key PEM tag '{tag}'"
        ))),
    }
}

pub(super) fn extract_first_octet_string(der: &[u8]) -> Result<Vec<u8>> {
    let blocks = simple_asn1::from_der(der)
        .map_err(|err| snowflake_err(format!("failed to decode PKCS#8 private key DER: {err}")))?;
    visit_first_octet_string(&blocks)
        .ok_or_else(|| snowflake_err("PKCS#8 private key did not contain an RSA key"))
}

pub(super) fn visit_first_octet_string(blocks: &[ASN1Block]) -> Option<Vec<u8>> {
    for block in blocks {
        match block {
            ASN1Block::OctetString(_, value) => return Some(value.clone()),
            ASN1Block::Sequence(_, children) => {
                if let Some(value) = visit_first_octet_string(children) {
                    return Some(value);
                }
            }
            _ => {}
        }
    }
    None
}

pub(super) fn rsa_public_components_from_private_der(
    private_der: &[u8],
) -> Result<(BigInt, BigInt)> {
    let blocks = simple_asn1::from_der(private_der)
        .map_err(|err| snowflake_err(format!("failed to decode RSA private key DER: {err}")))?;
    let [ASN1Block::Sequence(_, entries)] = blocks.as_slice() else {
        return Err(snowflake_err(
            "RSA private key DER must contain a single sequence",
        ));
    };
    let modulus = match entries.get(1) {
        Some(ASN1Block::Integer(_, value)) => value.clone(),
        _ => return Err(snowflake_err("RSA private key missing modulus")),
    };
    let exponent = match entries.get(2) {
        Some(ASN1Block::Integer(_, value)) => value.clone(),
        _ => return Err(snowflake_err("RSA private key missing public exponent")),
    };
    Ok((modulus, exponent))
}

pub(super) fn rsa_public_spki_der(modulus: BigInt, exponent: BigInt) -> Result<Vec<u8>> {
    let rsa_public_key = simple_asn1::to_der(&ASN1Block::Sequence(
        0,
        vec![
            ASN1Block::Integer(0, modulus),
            ASN1Block::Integer(0, exponent),
        ],
    ))
    .map_err(|err| snowflake_err(format!("failed to encode RSA public key DER: {err}")))?;

    let spki = ASN1Block::Sequence(
        0,
        vec![
            ASN1Block::Sequence(
                0,
                vec![
                    ASN1Block::ObjectIdentifier(0, oid!(1, 2, 840, 113_549, 1, 1, 1)),
                    ASN1Block::Null(0),
                ],
            ),
            ASN1Block::BitString(0, rsa_public_key.len().saturating_mul(8), rsa_public_key),
        ],
    );

    simple_asn1::to_der(&spki)
        .map_err(|err| snowflake_err(format!("failed to encode public key SPKI DER: {err}")))
}
