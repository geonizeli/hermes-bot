//! Talking to a Hermes dashboard's auth endpoints: discovery, native (RFC
//! 8252) sign-in through the system browser, password sign-in without a
//! browser, and token refresh.

use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::RngExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::store::Tokens;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Provider {
    pub name: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub supports_password: bool,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct ServerInfo {
    pub url: String,
    pub version: String,
    pub auth_required: bool,
    pub native_sign_in: bool,
    pub providers: Vec<Provider>,
}

pub fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("hermes-bot/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(20))
        .build()
        .expect("HTTP client")
}

/// The most useful message out of an error response: FastAPI's `detail`,
/// Hermes' structured `error`, or the status line.
async fn error_text(response: reqwest::Response) -> String {
    let status = response.status();
    let body: Value = response.json().await.unwrap_or(Value::Null);
    let detail = body
        .get("detail")
        .and_then(Value::as_str)
        .filter(|d| !d.is_empty() && *d != "Unauthorized")
        .or_else(|| body.get("error").and_then(Value::as_str));
    match detail {
        Some(detail) => format!("{detail} (HTTP {})", status.as_u16()),
        None => format!("The server answered HTTP {status}"),
    }
}

fn network_error(base: &str, e: reqwest::Error) -> String {
    if e.is_timeout() {
        format!("{base} didn't answer in time.")
    } else if e.is_connect() {
        format!("Can't reach {base}. Check the address and your connection.")
    } else {
        format!("Request to {base} failed: {e}")
    }
}

pub async fn discover(base: &str) -> Result<ServerInfo, String> {
    let client = http();
    let health = client.get(format!("{base}/api/health")).send().await.map_err(|e| network_error(base, e))?;
    if !health.status().is_success() {
        return Err(format!("{base} doesn't look like a Hermes dashboard ({}).", error_text(health).await));
    }
    let health: Value = health
        .json()
        .await
        .map_err(|_| format!("{base} doesn't look like a Hermes dashboard (unexpected /api/health)."))?;
    let mut info = ServerInfo {
        url: base.to_string(),
        version: health
            .get("displayVersion")
            .or_else(|| health.get("version"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        auth_required: health.get("auth_required").and_then(Value::as_bool).unwrap_or(true),
        ..Default::default()
    };
    if let Ok(status) = client.get(format!("{base}/api/status")).send().await {
        if let Ok(status) = status.json::<Value>().await {
            info.native_sign_in = status
                .get("auth_flows")
                .and_then(Value::as_array)
                .is_some_and(|flows| flows.iter().any(|f| f == "native_pkce"));
        }
    }
    if info.auth_required {
        let response = client
            .get(format!("{base}/api/auth/providers"))
            .send()
            .await
            .map_err(|e| network_error(base, e))?;
        if !response.status().is_success() {
            return Err(format!("The server has no sign-in configured: {}", error_text(response).await));
        }
        let body: Value = response.json().await.map_err(|e| e.to_string())?;
        info.providers = serde_json::from_value(body.get("providers").cloned().unwrap_or(json!([]))).unwrap_or_default();
    }
    if info.auth_required && !info.native_sign_in {
        return Err(format!(
            "{base} runs Hermes {} which doesn't support app sign-in yet. Update Hermes on the server.",
            info.version
        ));
    }
    Ok(info)
}

struct Pkce {
    verifier: String,
    challenge: String,
    state: String,
}

fn random_token() -> String {
    let bytes: [u8; 32] = rand::rng().random();
    URL_SAFE_NO_PAD.encode(bytes)
}

fn pkce() -> Pkce {
    let verifier = random_token();
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    Pkce { verifier, challenge, state: random_token() }
}

fn authorize_url(base: &str, pkce: &Pkce, redirect_uri: &str, provider: &str) -> String {
    let mut url = url::Url::parse(&format!("{base}/auth/native/authorize")).expect("valid base");
    url.query_pairs_mut()
        .append_pair("code_challenge", &pkce.challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("state", &pkce.state);
    if !provider.is_empty() {
        url.query_pairs_mut().append_pair("provider", provider);
    }
    url.into()
}

fn parse_tokens(body: &Value, fallback_provider: &str) -> Result<Tokens, String> {
    let text = |key: &str| body.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    let tokens = Tokens {
        access_token: text("access_token"),
        refresh_token: text("refresh_token"),
        expires_at: body.get("expires_at").and_then(Value::as_f64).unwrap_or(0.0) as i64,
        provider: Some(text("provider")).filter(|p| !p.is_empty()).unwrap_or_else(|| fallback_provider.into()),
    };
    if tokens.access_token.is_empty() {
        return Err("The server's token response had no access token.".into());
    }
    Ok(tokens)
}

async fn exchange_code(base: &str, code: &str, verifier: &str, provider: &str) -> Result<Tokens, String> {
    let response = http()
        .post(format!("{base}/auth/native/token"))
        .json(&json!({ "code": code, "code_verifier": verifier }))
        .send()
        .await
        .map_err(|e| network_error(base, e))?;
    if !response.status().is_success() {
        return Err(format!("Sign-in couldn't be completed: {}", error_text(response).await));
    }
    parse_tokens(&response.json().await.map_err(|e| e.to_string())?, provider)
}

/// Pulls `code` out of a loopback redirect after checking `state`.
fn code_from_redirect(target: &str, expected_state: &str) -> Result<String, String> {
    let url = url::Url::parse(target)
        .or_else(|_| url::Url::parse(&format!("http://127.0.0.1{target}")))
        .map_err(|_| "The server returned an invalid sign-in redirect.".to_string())?;
    let param = |name: &str| url.query_pairs().find(|(k, _)| k == name).map(|(_, v)| v.into_owned());
    if let Some(error) = param("error") {
        let description = param("error_description").unwrap_or_default();
        return Err(format!("Sign-in was refused: {error} {description}").trim().to_string());
    }
    if param("state").as_deref() != Some(expected_state) {
        return Err("Sign-in response didn't match this request; try again.".into());
    }
    param("code").ok_or_else(|| "The sign-in redirect had no code.".into())
}

/// Sign-in through the system browser: OAuth providers, or the dashboard's
/// own login page. `open_browser` is injected so tests can follow the URL.
pub async fn browser_sign_in(
    base: &str,
    provider: &str,
    open_browser: impl FnOnce(&str) -> Result<(), String>,
) -> Result<Tokens, String> {
    let listener = TcpListener::bind("127.0.0.1:0").await.map_err(|e| format!("Can't listen for the sign-in reply: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");
    let pkce = pkce();
    open_browser(&authorize_url(base, &pkce, &redirect_uri, provider))?;

    let wait = async {
        loop {
            let (mut socket, _) = listener.accept().await.map_err(|e| e.to_string())?;
            let mut buffer = vec![0u8; 8192];
            let read = socket.read(&mut buffer).await.unwrap_or(0);
            let request = String::from_utf8_lossy(&buffer[..read]).to_string();
            let target = request.split_whitespace().nth(1).unwrap_or("").to_string();
            if !target.starts_with("/callback") {
                let _ = socket.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                continue;
            }
            let result = code_from_redirect(&target, &pkce.state);
            let message = match &result {
                Ok(_) => "Signed in. You can close this tab and return to Hermes Bot.",
                Err(_) => "Sign-in failed. Return to Hermes Bot for details.",
            };
            let page = format!(
                "<!doctype html><meta charset=utf-8><title>Hermes Bot</title>\
                 <body style=\"font-family:sans-serif;padding:3em\"><p>{message}</p>"
            );
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}",
                page.len()
            );
            let _ = socket.write_all(reply.as_bytes()).await;
            return result;
        }
    };
    let code = tokio::time::timeout(Duration::from_secs(600), wait)
        .await
        .map_err(|_| "Sign-in timed out. Try again.".to_string())??;
    exchange_code(base, &code, &pkce.verifier, provider).await
}

/// Password sign-in without a browser: start the native flow, post the
/// credentials with its cookie, and read the code from the redirect target
/// the server hands back instead of following it.
pub async fn password_sign_in(base: &str, provider: &str, username: &str, password: &str) -> Result<Tokens, String> {
    let client = reqwest::Client::builder()
        .user_agent(concat!("hermes-bot/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(20))
        .cookie_store(true)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())?;
    let pkce = pkce();
    // Nothing listens here: the code is read from the JSON reply, never
    // delivered to this address.
    let redirect_uri = "http://127.0.0.1:9/callback";
    let start = client
        .get(authorize_url(base, &pkce, redirect_uri, provider))
        .send()
        .await
        .map_err(|e| network_error(base, e))?;
    if !(start.status().is_redirection() || start.status().is_success()) {
        return Err(format!("Couldn't start sign-in: {}", error_text(start).await));
    }
    let response = client
        .post(format!("{base}/auth/password-login"))
        .json(&json!({ "provider": provider, "username": username, "password": password, "next": "" }))
        .send()
        .await
        .map_err(|e| network_error(base, e))?;
    match response.status().as_u16() {
        200..=299 => {}
        401 => return Err("Wrong username or password.".into()),
        429 => return Err("Too many attempts. Wait a minute and try again.".into()),
        _ => return Err(format!("Sign-in failed: {}", error_text(response).await)),
    }
    let body: Value = response.json().await.map_err(|e| e.to_string())?;
    let next = body.get("next").and_then(Value::as_str).unwrap_or_default();
    let code = code_from_redirect(next, &pkce.state)?;
    exchange_code(base, &code, &pkce.verifier, provider).await
}

#[derive(Debug)]
pub enum RefreshError {
    /// The refresh token is no longer valid; the user must sign in again.
    Expired,
    Other(String),
}

pub async fn refresh(base: &str, tokens: &Tokens) -> Result<Tokens, RefreshError> {
    let mut body = json!({ "refresh_token": tokens.refresh_token });
    if !tokens.provider.is_empty() {
        body["provider"] = json!(tokens.provider);
    }
    let response = http()
        .post(format!("{base}/auth/native/refresh"))
        .json(&body)
        .send()
        .await
        .map_err(|e| RefreshError::Other(network_error(base, e)))?;
    match response.status().as_u16() {
        200..=299 => {
            let body = response.json().await.map_err(|e| RefreshError::Other(e.to_string()))?;
            parse_tokens(&body, &tokens.provider).map_err(RefreshError::Other)
        }
        400 | 401 | 403 => Err(RefreshError::Expired),
        _ => Err(RefreshError::Other(error_text(response).await)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_is_s256_of_verifier() {
        let p = pkce();
        assert_eq!(p.challenge, URL_SAFE_NO_PAD.encode(Sha256::digest(p.verifier.as_bytes())));
        assert_eq!(p.verifier.len(), 43);
        assert_ne!(p.state, p.verifier);
    }

    #[test]
    fn authorize_url_carries_the_flow() {
        let p = pkce();
        let url = authorize_url("https://h.example/pre", &p, "http://127.0.0.1:5/callback", "basic");
        assert!(url.starts_with("https://h.example/pre/auth/native/authorize?"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A5%2Fcallback"));
        assert!(url.contains("provider=basic"));
    }

    #[test]
    fn reads_codes_from_redirects() {
        assert_eq!(code_from_redirect("http://127.0.0.1:9/callback?code=abc&state=s", "s").unwrap(), "abc");
        assert_eq!(code_from_redirect("/callback?code=abc&state=s", "s").unwrap(), "abc");
        assert!(code_from_redirect("/callback?code=abc&state=other", "s").is_err());
        assert!(code_from_redirect("/callback?error=access_denied&state=s", "s").is_err());
    }

    #[test]
    fn parses_token_responses() {
        let tokens = parse_tokens(
            &json!({"access_token":"a","refresh_token":"r","expires_at":1700000000,"provider":"nous"}),
            "x",
        )
        .unwrap();
        assert_eq!(tokens.provider, "nous");
        assert_eq!(tokens.expires_at, 1_700_000_000);
        assert!(parse_tokens(&json!({}), "x").is_err());
    }
}

/// Against a real server: `HERMES_BOT_TEST_URL=https://host cargo test -- --ignored`.
#[cfg(test)]
mod live {
    #[tokio::test]
    #[ignore]
    async fn discovers_a_real_server() {
        let Ok(url) = std::env::var("HERMES_BOT_TEST_URL") else { return };
        let info = super::discover(&url).await.unwrap();
        println!("{info:?}");
        assert!(!info.version.is_empty());
    }
}
