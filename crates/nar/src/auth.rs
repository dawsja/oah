//! Sign in with ChatGPT (ChatGPT plan usage for open-source clients) and API keys.
//!
//! Flow (see developers.openai.com/siwc/token-sharing-open-source):
//! * A stable, opaque `ext_agent_host_id` (`urn:uuid:…`) is created once per host
//!   before the first sign-in and reused forever.
//! * First sign-in registers a user-defined agent: `client_id=dynamic_agent_client`
//!   plus `agent_name_hint`; the callback returns the issued `client_id`, which is
//!   kept (even across sign-out) and reused for every later authorization.
//! * PKCE S256, fresh state + nonce, loopback redirect on 127.0.0.1 (port 1455
//!   for registration; any free port afterwards).
//! * The ID token is verified against OpenAI's JWKS (iss, aud, exp, nonce) and the
//!   granted scopes must include `chatgpt.tokens.use.direct` for plan usage.
//! * Access tokens last an hour; refreshes are serialized and the replacement
//!   refresh token is stored atomically with 0600 permissions.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;

use crate::config;

pub const ISSUER: &str = "https://auth.openai.com";
pub const AUTHORIZE_URL: &str = "https://auth.openai.com/api/accounts/authorize";
pub const TOKEN_URL: &str = "https://auth.openai.com/api/accounts/oauth/token";
pub const DISCOVERY_URL: &str = "https://auth.openai.com/.well-known/openid-configuration";
pub const RESOURCE: &str = "https://api.openai.com/v1";
pub const REGISTRATION_CLIENT_ID: &str = "dynamic_agent_client";
pub const PLAN_SCOPE: &str = "chatgpt.tokens.use.direct";
pub const SCOPES: &str = "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
pub const REGISTRATION_PORT: u16 = 1455;
pub const USAGE_URL: &str = "https://chatgpt.com/settings/usage";

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ChatGptCredentials {
    pub email: Option<String>,
    pub issuer: String,
    pub subject: String,
    pub client_id: String,
    pub ext_agent_host_id: String,
    pub id_token: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub token_type: String,
    pub expires_in: i64,
    pub scopes: Vec<String>,
    /// Unix seconds when the access token was received.
    pub saved_at: i64,
}

impl ChatGptCredentials {
    pub fn plan_usage_enabled(&self) -> bool {
        self.scopes.iter().any(|s| s == PLAN_SCOPE)
    }
    fn expires_at(&self) -> i64 {
        self.saved_at + self.expires_in
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AuthMethod {
    #[default]
    None,
    ChatGpt,
    ApiKey,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AuthStore {
    pub method: AuthMethod,
    pub chatgpt: Option<ChatGptCredentials>,
    pub api_key: Option<String>,
    /// Issued client id, retained across sign-out for reauthorization.
    pub client_id: Option<String>,
    /// Last ID token, used as `id_token_hint` to skip account selection.
    pub last_id_token: Option<String>,
}

impl AuthStore {
    pub fn load() -> AuthStore {
        std::fs::read(config::auth_file())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }
    pub fn save(&self) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(self)?;
        config::write_private(&config::auth_file(), &bytes)?;
        Ok(())
    }
}

/// Short, user-facing description of the active authentication.
#[derive(Clone, Debug, Default)]
pub struct AuthStatus {
    pub method: AuthMethod,
    pub label: String,
    pub email: Option<String>,
    pub plan_usage: bool,
}

pub fn host_id() -> String {
    let path = config::host_id_file();
    if let Ok(s) = std::fs::read_to_string(&path) {
        let s = s.trim().to_string();
        if s.starts_with("urn:uuid:") || s.starts_with("urn:ietf:params:oauth:jwk-thumbprint:") || s.starts_with("did:key:") {
            return s;
        }
    }
    let id = format!("urn:uuid:{}", uuid::Uuid::new_v4());
    let _ = config::write_private(&path, id.as_bytes());
    id
}

fn random_b64(n: usize) -> String {
    let mut b = vec![0u8; n];
    rand::thread_rng().fill_bytes(&mut b);
    URL_SAFE_NO_PAD.encode(b)
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    id_token: Option<String>,
    token_type: Option<String>,
    expires_in: Option<i64>,
    scope: Option<String>,
}

#[derive(Deserialize)]
struct OAuthError {
    error: Option<String>,
    error_description: Option<String>,
}

/// Shared, thread-safe authentication manager.
#[derive(Clone)]
pub struct Auth {
    inner: Arc<Mutex<AuthStore>>,
    http: reqwest::Client,
}

pub struct PendingLogin {
    /// Full authorize URL to open in the browser.
    pub url: String,
    /// The same URL with `id_token_hint` redacted, safe to display.
    pub display_url: String,
    pub task: tokio::task::JoinHandle<Result<AuthStatus>>,
}

impl Auth {
    pub fn new(http: reqwest::Client) -> Auth {
        Auth { inner: Arc::new(Mutex::new(AuthStore::load())), http }
    }

    pub async fn status(&self) -> AuthStatus {
        let store = self.inner.lock().await;
        status_of(&store)
    }

    /// Resolve a bearer token for the Responses API, refreshing if needed.
    pub async fn bearer(&self) -> Result<String> {
        let mut store = self.inner.lock().await;
        match store.method {
            AuthMethod::ChatGpt => {
                let creds = store.chatgpt.clone().ok_or_else(|| anyhow!("not signed in — run /auth"))?;
                if !creds.plan_usage_enabled() {
                    bail!("ChatGPT plan usage was not granted — run /auth to enable it");
                }
                if creds.expires_at() - now() > 120 {
                    return Ok(creds.access_token);
                }
                match self.refresh_locked(&mut store).await {
                    Ok(tok) => Ok(tok),
                    Err(e) => Err(e),
                }
            }
            AuthMethod::ApiKey => store
                .api_key
                .clone()
                .or_else(|| std::env::var("OPENAI_API_KEY").ok())
                .ok_or_else(|| anyhow!("no API key — run /auth")),
            AuthMethod::None => {
                if let Ok(k) = std::env::var("OPENAI_API_KEY") {
                    return Ok(k);
                }
                bail!("not signed in — run /auth to continue with ChatGPT or add an API key")
            }
        }
    }

    /// Force a refresh (e.g. after a 401).
    pub async fn force_refresh(&self) -> Result<String> {
        let mut store = self.inner.lock().await;
        if store.method != AuthMethod::ChatGpt {
            bail!("unauthorized");
        }
        self.refresh_locked(&mut store).await
    }

    async fn refresh_locked(&self, store: &mut AuthStore) -> Result<String> {
        let creds = store.chatgpt.clone().ok_or_else(|| anyhow!("not signed in"))?;
        let refresh = creds.refresh_token.clone().ok_or_else(|| anyhow!("session expired — run /auth"))?;
        let resp = self
            .http
            .post(TOKEN_URL)
            .form(&[
                ("grant_type", "refresh_token"),
                ("client_id", creds.client_id.as_str()),
                ("refresh_token", refresh.as_str()),
                ("resource", RESOURCE),
            ])
            .send()
            .await
            .context("refreshing ChatGPT session")?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            let err: OAuthError = serde_json::from_str(&body).unwrap_or(OAuthError { error: None, error_description: None });
            let code = err.error.unwrap_or_default();
            const TERMINAL: [&str; 6] = [
                "invalid_grant",
                "invalid_refresh_token",
                "token_expired",
                "refresh_token_expired",
                "refresh_token_invalidated",
                "refresh_token_reused",
            ];
            if TERMINAL.contains(&code.as_str()) {
                store.chatgpt = None;
                let _ = store.save();
                bail!("ChatGPT session ended ({code}) — run /auth to sign in again");
            }
            if code == "invalid_client" {
                bail!("ChatGPT client registration is invalid (invalid_client) — run /auth logout then /auth");
            }
            bail!("refresh failed ({status}): {}", err.error_description.unwrap_or(body));
        }
        let tok: TokenResponse = serde_json::from_str(&body).context("parsing refresh response")?;
        let mut updated = creds.clone();
        updated.access_token = tok.access_token.clone();
        if let Some(r) = tok.refresh_token {
            updated.refresh_token = Some(r);
        }
        if let Some(id) = tok.id_token {
            updated.id_token = id.clone();
            store.last_id_token = Some(id);
        }
        updated.expires_in = tok.expires_in.unwrap_or(3600);
        if let Some(scope) = tok.scope {
            updated.scopes = scope.split_whitespace().map(String::from).collect();
        }
        updated.saved_at = now();
        store.chatgpt = Some(updated);
        store.save()?;
        Ok(tok.access_token)
    }

    pub async fn set_api_key(&self, key: String) -> Result<AuthStatus> {
        let mut store = self.inner.lock().await;
        store.api_key = Some(key.trim().to_string());
        store.method = AuthMethod::ApiKey;
        store.save()?;
        Ok(status_of(&store))
    }

    pub async fn use_method(&self, m: AuthMethod) -> Result<AuthStatus> {
        let mut store = self.inner.lock().await;
        store.method = m;
        store.save()?;
        Ok(status_of(&store))
    }

    /// Sign out: revoke the refresh token (best effort) and clear credentials.
    /// The issued client id is kept for future reauthorization.
    pub async fn logout(&self) -> Result<String> {
        let mut store = self.inner.lock().await;
        let mut note = String::from("Signed out.");
        if let Some(creds) = store.chatgpt.take() {
            if let Some(rt) = creds.refresh_token.as_deref() {
                match self.revoke(rt, &creds.client_id).await {
                    Ok(()) => note = "Signed out and revoked the ChatGPT session.".into(),
                    Err(_) => {
                        note = "Signed out locally. Revocation could not be confirmed — disconnect nar in ChatGPT Settings if needed.".into()
                    }
                }
            }
            store.client_id = Some(creds.client_id);
        }
        store.last_id_token = None;
        if store.method == AuthMethod::ChatGpt {
            store.method = if store.api_key.is_some() { AuthMethod::ApiKey } else { AuthMethod::None };
        }
        store.save()?;
        Ok(note)
    }

    async fn revoke(&self, token: &str, client_id: &str) -> Result<()> {
        let disco: serde_json::Value = self.http.get(DISCOVERY_URL).send().await?.json().await?;
        let endpoint = disco["revocation_endpoint"].as_str().ok_or_else(|| anyhow!("no revocation endpoint"))?;
        let mut delay = Duration::from_millis(500);
        for _ in 0..3 {
            let r = self
                .http
                .post(endpoint)
                .form(&[("token", token), ("token_type_hint", "refresh_token"), ("client_id", client_id)])
                .send()
                .await;
            match r {
                Ok(r) if r.status().is_success() => return Ok(()),
                Ok(r) if r.status().is_client_error() => bail!("revoke rejected: {}", r.status()),
                _ => tokio::time::sleep(delay).await,
            }
            delay *= 2;
        }
        bail!("revoke failed")
    }

    /// Begin a browser sign-in. Returns the authorize URL (to open/show) and a
    /// task that completes when the loopback callback has been handled.
    pub async fn begin_chatgpt_login(&self) -> Result<PendingLogin> {
        let (client_id, id_token_hint) = {
            let store = self.inner.lock().await;
            let cid = store.chatgpt.as_ref().map(|c| c.client_id.clone()).or_else(|| store.client_id.clone());
            (cid, store.last_id_token.clone())
        };
        let registering = client_id.is_none();
        let host = host_id();

        let listener = match tokio::net::TcpListener::bind(("127.0.0.1", REGISTRATION_PORT)).await {
            Ok(l) => l,
            Err(e) if registering => {
                bail!("port {REGISTRATION_PORT} on 127.0.0.1 is busy ({e}); first-time ChatGPT registration requires it")
            }
            Err(_) => tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?,
        };
        let port = listener.local_addr()?.port();
        let redirect_uri = format!("http://127.0.0.1:{port}/auth/callback");

        let state = random_b64(32);
        let nonce = random_b64(32);
        let verifier = random_b64(64);
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));

        let mut url = url::Url::parse(AUTHORIZE_URL)?;
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("response_type", "code");
            q.append_pair("client_id", client_id.as_deref().unwrap_or(REGISTRATION_CLIENT_ID));
            if registering {
                q.append_pair("agent_name_hint", "nar");
            }
            q.append_pair("ext_agent_host_id", &host);
            q.append_pair("redirect_uri", &redirect_uri);
            q.append_pair("scope", SCOPES);
            q.append_pair("resource", RESOURCE);
            q.append_pair("state", &state);
            q.append_pair("nonce", &nonce);
            q.append_pair("code_challenge", &challenge);
            q.append_pair("code_challenge_method", "S256");
            if let Some(h) = id_token_hint.as_deref() {
                q.append_pair("id_token_hint", h);
            }
        }
        let full_url = url.to_string();
        let display_url = redact_url(&full_url);

        let me = self.clone();
        let task = tokio::spawn(async move {
            let params = tokio::time::timeout(Duration::from_secs(600), wait_for_callback(listener, &state))
                .await
                .map_err(|_| anyhow!("sign-in timed out"))??;
            if let Some(err) = params.get("error") {
                if err == "access_denied" {
                    bail!("sign-in was cancelled or consent was declined");
                }
                bail!("sign-in failed: {err} {}", params.get("error_description").cloned().unwrap_or_default());
            }
            let code = params.get("code").ok_or_else(|| anyhow!("callback missing code"))?.clone();
            let issued = params.get("client_id").cloned();
            let client_id = match (&client_id, issued) {
                (Some(saved), Some(got)) if saved != &got => bail!("callback returned an unexpected client_id"),
                (Some(saved), _) => saved.clone(),
                (None, Some(got)) if got != REGISTRATION_CLIENT_ID => got,
                (None, _) => bail!("registration incomplete: no issued client_id in callback"),
            };
            me.finish_login(&client_id, &code, &verifier, &redirect_uri, &nonce, &host).await
        });
        Ok(PendingLogin { url: full_url, display_url, task })
    }

    async fn finish_login(
        &self,
        client_id: &str,
        code: &str,
        verifier: &str,
        redirect_uri: &str,
        nonce: &str,
        host: &str,
    ) -> Result<AuthStatus> {
        let resp = self
            .http
            .post(TOKEN_URL)
            .form(&[
                ("grant_type", "authorization_code"),
                ("client_id", client_id),
                ("code", code),
                ("code_verifier", verifier),
                ("redirect_uri", redirect_uri),
                ("resource", RESOURCE),
            ])
            .send()
            .await
            .context("exchanging authorization code")?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            // Keep the issued client id so the retry reauthorizes instead of re-registering.
            let mut store = self.inner.lock().await;
            store.client_id = Some(client_id.to_string());
            let _ = store.save();
            bail!("token exchange failed ({status}): {body}");
        }
        let tok: TokenResponse = serde_json::from_str(&body).context("parsing token response")?;
        let id_token = tok.id_token.clone().ok_or_else(|| anyhow!("token response has no id_token"))?;
        let claims = verify_id_token(&self.http, &id_token, client_id, nonce).await?;
        let scopes: Vec<String> = tok.scope.unwrap_or_default().split_whitespace().map(String::from).collect();
        let creds = ChatGptCredentials {
            email: claims.get("email").and_then(|v| v.as_str()).map(String::from),
            issuer: ISSUER.into(),
            subject: claims.get("sub").and_then(|v| v.as_str()).unwrap_or_default().into(),
            client_id: client_id.into(),
            ext_agent_host_id: host.into(),
            id_token: id_token.clone(),
            access_token: tok.access_token,
            refresh_token: tok.refresh_token,
            token_type: tok.token_type.unwrap_or_else(|| "Bearer".into()),
            expires_in: tok.expires_in.unwrap_or(3600),
            scopes,
            saved_at: now(),
        };
        let mut store = self.inner.lock().await;
        store.client_id = Some(client_id.into());
        store.last_id_token = Some(id_token);
        store.chatgpt = Some(creds);
        store.method = AuthMethod::ChatGpt;
        store.save()?;
        Ok(status_of(&store))
    }
}

/// Redact `id_token_hint` from a URL for display/logging.
pub fn redact_url(u: &str) -> String {
    match url::Url::parse(u) {
        Ok(mut url) => {
            let pairs: Vec<(String, String)> =
                url.query_pairs().filter(|(k, _)| k != "id_token_hint").map(|(k, v)| (k.into(), v.into())).collect();
            url.query_pairs_mut().clear().extend_pairs(pairs);
            url.to_string()
        }
        Err(_) => u.to_string(),
    }
}

fn status_of(store: &AuthStore) -> AuthStatus {
    match store.method {
        AuthMethod::ChatGpt => match &store.chatgpt {
            Some(c) => AuthStatus {
                method: AuthMethod::ChatGpt,
                label: if c.plan_usage_enabled() { "ChatGPT plan".into() } else { "ChatGPT (plan usage off)".into() },
                email: c.email.clone(),
                plan_usage: c.plan_usage_enabled(),
            },
            None => AuthStatus { method: AuthMethod::None, label: "signed out".into(), ..Default::default() },
        },
        AuthMethod::ApiKey => AuthStatus { method: AuthMethod::ApiKey, label: "API key".into(), ..Default::default() },
        AuthMethod::None => {
            if std::env::var_os("OPENAI_API_KEY").is_some() {
                AuthStatus { method: AuthMethod::ApiKey, label: "OPENAI_API_KEY".into(), ..Default::default() }
            } else {
                AuthStatus { method: AuthMethod::None, label: "signed out".into(), ..Default::default() }
            }
        }
    }
}

async fn wait_for_callback(listener: tokio::net::TcpListener, state: &str) -> Result<HashMap<String, String>> {
    loop {
        let (mut sock, _) = listener.accept().await?;
        let mut buf = Vec::with_capacity(4096);
        let mut tmp = [0u8; 2048];
        loop {
            let n = sock.read(&mut tmp).await?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
            if buf.windows(4).any(|w| w == b"\r\n\r\n") || buf.len() > 64 * 1024 {
                break;
            }
        }
        let req = String::from_utf8_lossy(&buf);
        let line = req.lines().next().unwrap_or("");
        let target = line.split_whitespace().nth(1).unwrap_or("/");
        let parsed = url::Url::parse(&format!("http://127.0.0.1{target}"))?;
        if parsed.path() != "/auth/callback" {
            let _ = sock.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
            continue;
        }
        let params: HashMap<String, String> = parsed.query_pairs().map(|(k, v)| (k.into(), v.into())).collect();
        let ok_state = params.get("state").map(|s| s == state).unwrap_or(false);
        let (title, msg) = if !ok_state {
            ("Sign-in failed", "The sign-in response did not match this request. Return to nar and try again.")
        } else if params.contains_key("error") {
            ("Sign-in cancelled", "No changes were made. You can close this tab and return to nar.")
        } else {
            ("Signed in to nar", "You can close this tab and return to your terminal.")
        };
        let html = format!(
            "<!doctype html><meta charset=utf-8><title>{title}</title>\
             <body style=\"font-family:system-ui;background:#0f0f12;color:#e8e8ee;display:grid;place-items:center;height:100vh;margin:0\">\
             <div style=\"text-align:center\"><h1 style=\"font-weight:600\">{title}</h1><p style=\"color:#9a9aa8\">{msg}</p></div>"
        );
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            html.len(),
            html
        );
        let _ = sock.write_all(resp.as_bytes()).await;
        let _ = sock.shutdown().await;
        if !ok_state {
            bail!("state mismatch in OAuth callback");
        }
        return Ok(params);
    }
}

async fn verify_id_token(
    http: &reqwest::Client,
    token: &str,
    client_id: &str,
    nonce: &str,
) -> Result<serde_json::Map<String, serde_json::Value>> {
    use jsonwebtoken::{decode, decode_header, jwk::JwkSet, DecodingKey, Validation};
    let header = decode_header(token).context("malformed id_token")?;
    let disco: serde_json::Value = http.get(DISCOVERY_URL).send().await?.json().await?;
    let jwks_uri = disco["jwks_uri"].as_str().unwrap_or("https://auth.openai.com/.well-known/jwks.json");
    let jwks: JwkSet = http.get(jwks_uri).send().await?.json().await.context("fetching JWKS")?;
    let jwk = match header.kid.as_deref() {
        Some(kid) => jwks.find(kid).ok_or_else(|| anyhow!("unknown id_token key id"))?,
        None => jwks.keys.first().ok_or_else(|| anyhow!("empty JWKS"))?,
    };
    let key = DecodingKey::from_jwk(jwk)?;
    let mut v = Validation::new(header.alg);
    v.set_issuer(&[ISSUER]);
    v.set_audience(&[client_id]);
    v.leeway = 60;
    let data = decode::<serde_json::Map<String, serde_json::Value>>(token, &key, &v).context("id_token validation failed")?;
    let claims = data.claims;
    if claims.get("nonce").and_then(|n| n.as_str()) != Some(nonce) {
        bail!("id_token nonce mismatch");
    }
    Ok(claims)
}

pub fn open_browser(url: &str) -> bool {
    let cmds: &[&str] = if cfg!(target_os = "macos") { &["open"] } else { &["xdg-open", "wslview", "sensible-browser"] };
    for c in cmds {
        if std::process::Command::new(c)
            .arg(url)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .is_ok()
        {
            return true;
        }
    }
    false
}
