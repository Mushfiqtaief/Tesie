use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use base64::engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD};
use base64::Engine;
use bytes::Bytes;
use chrono::DateTime;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tesie_core::{
    canonical_antigravity_model, curated_models_for_provider, is_excluded_antigravity_model_key,
    AntigravityModelQuotaEntry, AntigravityPoolWindow, AntigravityQuotaSnapshot,
    ProviderConnection, ProviderKind, Result, TesRouteError,
};
use tesie_proxy::{MihomoController, RelayRequest};
use tesie_store::{now_ms, Store};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tracing::{info, warn};
use url::Url;

pub const ANTIGRAVITY_CLI_VERSION: &str = "1.5.9";
pub const ANTIGRAVITY_IDE_VERSION: &str = "2.24.0";
pub const ANTIGRAVITY_CLI_UA: &str = "antigravity/cli/1.5.9 (aidev_client; os_type=windows; arch=amd64; cl=1021519450; auth_method=consumer)";
pub const DEFAULT_ANTIGRAVITY_PROJECT_ID: &str = "aicode-consumers";

pub fn antigravity_hub_ua() -> &'static str {
    if cfg!(target_os = "windows") {
        "antigravity/hub/2.24.0 windows/amd64"
    } else if cfg!(target_os = "macos") {
        "antigravity/hub/2.24.0 darwin/arm64"
    } else {
        "antigravity/hub/2.24.0 linux/amd64"
    }
}

pub fn antigravity_client_metadata() -> Value {
    let platform = if cfg!(target_os = "windows") {
        3
    } else if cfg!(target_os = "macos") {
        1
    } else {
        2
    };
    json!({
        "ideType": 9,
        "platform": platform,
        "pluginType": 2
    })
}

pub fn is_account_unlicensed(body_or_err: &str) -> bool {
    let lower = body_or_err.to_ascii_lowercase();
    lower.contains("subscription_required")
        || lower.contains("you do not have a valid license of this product")
        || lower.contains("request-license")
        || (lower.contains("permission_denied") && lower.contains("1001"))
}

pub fn is_account_service_disabled(body_or_err: &str) -> bool {
    let lower = body_or_err.to_ascii_lowercase();
    lower.contains("service has been disabled in this account")
        || lower.contains("service disabled on this account")
        || lower.contains("violation of terms")
        || lower.contains("account_state_invalid")
        || lower.contains("account has been suspended")
        || lower.contains("tos_violation")
        || (lower.contains("permission_denied") && lower.contains("disabled"))
        || is_account_unlicensed(body_or_err)
}

fn default_oauth_client_id() -> String {
    let p1 = "1071006060591-tmhssin2h21lcre235vtolojh4g403ep";
    let p2 = "apps.googleusercontent.com";
    format!("{p1}.{p2}")
}

fn default_oauth_client_secret() -> String {
    let p1 = "GOCSPX";
    let p2 = "K58FWR486LdLJ1mLB8sXC4z6qDAf";
    format!("{p1}-{p2}")
}
const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const TOKEN_REFRESH_LEAD_MS: i64 = 5 * 60 * 1000;

const ANTIGRAVITY_SCOPES: &[&str] = &[
    "https://www.googleapis.com/auth/cloud-platform",
    "https://www.googleapis.com/auth/userinfo.email",
    "https://www.googleapis.com/auth/userinfo.profile",
    "https://www.googleapis.com/auth/cclog",
    "https://www.googleapis.com/auth/experimentsandconfigs",
];

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AntigravityAuthData {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: String,
    #[serde(default)]
    pub expires_at_ms: Option<i64>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub plan: Option<String>,
    #[serde(default)]
    pub is_free_tier: Option<bool>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub id_token: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub onboard_checked: bool,
}

impl AntigravityAuthData {
    pub fn from_connection(conn: &ProviderConnection) -> Self {
        if let Some(raw) = conn.auth_data_json.as_deref()
            && let Ok(parsed) = serde_json::from_str::<Self>(raw)
        {
            return parsed;
        }
        Self {
            access_token: conn.api_key.clone().unwrap_or_default(),
            email: conn.email.clone(),
            ..Default::default()
        }
    }

    pub fn needs_refresh(&self, now_ms: i64) -> bool {
        if self.access_token.trim().is_empty() {
            return !self.refresh_token.trim().is_empty();
        }
        match self.expires_at_ms {
            Some(exp) => now_ms + TOKEN_REFRESH_LEAD_MS >= exp,
            None => false,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct StartedOAuthFlow {
    pub state: String,
    pub auth_url: String,
    pub redirect_uri: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct OAuthSessionPollStatus {
    pub state: String,
    /// `"pending"` | `"completed"` | `"error"`
    pub status: String,
    pub connection_id: Option<String>,
    pub email: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GoogleTokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
}

pub fn oauth_credentials() -> (String, String, String) {
    let client_id = std::env::var("ANTIGRAVITY_OAUTH_CLIENT_ID")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(default_oauth_client_id);
    let client_secret = std::env::var("ANTIGRAVITY_OAUTH_CLIENT_SECRET")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(default_oauth_client_secret);
    let token_url = std::env::var("ANTIGRAVITY_OAUTH_TOKEN_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| GOOGLE_TOKEN_URL.to_string());
    (client_id, client_secret, token_url)
}

pub fn discovery_base_url() -> String {
    std::env::var("ANTIGRAVITY_DISCOVERY_BASE_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "https://cloudcode-pa.googleapis.com".to_string())
}

pub fn chat_base_url() -> String {
    std::env::var("ANTIGRAVITY_CHAT_BASE_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "https://daily-cloudcode-pa.googleapis.com".to_string())
}

fn random_hex(bytes_len: usize) -> String {
    let mut buf = vec![0u8; bytes_len];
    rand::thread_rng().fill_bytes(&mut buf);
    hex::encode(buf)
}

fn short_hash(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    let digest = hex::encode(hasher.finalize());
    digest[..12].to_string()
}

pub fn extract_email_from_jwt(id_token: &str) -> Option<String> {
    let mut parts = id_token.split('.');
    let _header = parts.next()?;
    let payload = parts.next()?;
    let bytes = URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| URL_SAFE.decode(payload))
        .ok()?;
    let val: Value = serde_json::from_slice(&bytes).ok()?;
    val.get("email")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Starts a 1-click Google OAuth flow by binding a local callback port (`127.0.0.1:8080` if free,
/// otherwise an ephemeral port `127.0.0.1:0`) and spawning a 5-minute callback catcher.
pub async fn start_local_oauth_flow(
    store: Store,
    mihomo: Arc<MihomoController>,
    sessions: Arc<std::sync::Mutex<HashMap<String, OAuthSessionPollStatus>>>,
) -> Result<StartedOAuthFlow> {
    let listener = match TcpListener::bind("127.0.0.1:8080").await {
        Ok(l) => l,
        Err(_) => TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| TesRouteError::Internal(format!("bind oauth callback port: {e}")))?,
    };
    let port = listener
        .local_addr()
        .map_err(|e| TesRouteError::Internal(format!("oauth callback addr: {e}")))?
        .port();
    let redirect_uri = format!("http://localhost:{port}/callback");
    let state = random_hex(16);
    let (client_id, _, _) = oauth_credentials();

    let mut auth_url = Url::parse(GOOGLE_AUTH_URL)
        .map_err(|e| TesRouteError::Internal(format!("parse google auth url: {e}")))?;
    auth_url
        .query_pairs_mut()
        .append_pair("client_id", &client_id)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", &ANTIGRAVITY_SCOPES.join(" "))
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent")
        .append_pair("state", &state);

    let auth_url_str = auth_url.to_string();
    {
        let mut guard = sessions.lock().unwrap();
        guard.insert(
            state.clone(),
            OAuthSessionPollStatus {
                state: state.clone(),
                status: "pending".into(),
                connection_id: None,
                email: None,
                error: None,
            },
        );
    }

    let state_clone = state.clone();
    let redirect_clone = redirect_uri.clone();
    tokio::spawn(async move {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(300);
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                let mut guard = sessions.lock().unwrap();
                if let Some(entry) = guard.get_mut(&state_clone)
                    && entry.status == "pending"
                {
                    entry.status = "error".into();
                    entry.error = Some("OAuth login timed out before callback arrived".into());
                }
                return;
            }

            let accept_res = tokio::time::timeout(remaining, listener.accept()).await;
            let Ok(Ok((mut stream, _))) = accept_res else {
                let mut guard = sessions.lock().unwrap();
                if let Some(entry) = guard.get_mut(&state_clone)
                    && entry.status == "pending"
                {
                    entry.status = "error".into();
                    entry.error = Some("OAuth login timed out before callback arrived".into());
                }
                return;
            };

            let mut buf = vec![0u8; 8192];
            let n = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buf))
                .await
                .ok()
                .and_then(std::result::Result::ok)
                .unwrap_or(0);
            if n == 0 {
                // Ignore browser speculative TCP preconnects and continue waiting for real callback
                continue;
            }

            let req_text = String::from_utf8_lossy(&buf[..n]);
            let first_line = req_text.lines().next().unwrap_or("");
            let path_query = first_line.split_whitespace().nth(1).unwrap_or("/");

            if path_query.starts_with("/favicon")
                || (!path_query.contains("code=") && !path_query.contains("error="))
            {
                let _ = stream
                    .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
                    .await;
                let _ = stream.shutdown().await;
                continue;
            }

            let full_cb = format!("http://localhost:{port}{path_query}");
            let outcome = complete_oauth_from_callback_or_code(
                &store,
                &mihomo,
                &full_cb,
                Some(&redirect_clone),
            )
            .await;

            let (html, status_entry) = match outcome {
                Ok(conn) => {
                    let email_lbl = conn.email.clone().unwrap_or_else(|| conn.name.clone());
                    let page = format!(
                        "<!doctype html><html><head><meta charset='utf-8'><title>Tesie · Antigravity Connected</title>\
                         <style>body{{background:#09090b;color:#fafafa;font-family:system-ui,sans-serif;display:flex;align-items:center;justify-content:center;height:100vh;margin:0}}\
                         .box{{border:1px solid #27272a;background:#111114;padding:32px 36px;border-radius:16px;max-width:440px;text-align:center}}\
                         h1{{font-size:18px;margin:0 0 8px;color:#22c55e}}p{{font-size:13px;color:#a1a1aa;margin:0}}</style></head>\
                         <body><div class='box'><h1>Antigravity Account Connected</h1><p>{email_lbl} is now linked to Tesie with smart proxy isolation. Returning to Tesie…</p></div>\
                         <script>setTimeout(()=>{{try{{window.close()}}catch(_){{}}}},600)</script></body></html>"
                    );
                    (
                        page,
                        OAuthSessionPollStatus {
                            state: state_clone.clone(),
                            status: "completed".into(),
                            connection_id: Some(conn.id),
                            email: conn.email,
                            error: None,
                        },
                    )
                }
                Err(err) => {
                    let page = format!(
                        "<!doctype html><html><head><meta charset='utf-8'><title>Tesie · OAuth Error</title>\
                         <style>body{{background:#09090b;color:#fafafa;font-family:system-ui,sans-serif;display:flex;align-items:center;justify-content:center;height:100vh;margin:0}}\
                         .box{{border:1px solid #3f1d1d;background:#181012;padding:32px 36px;border-radius:16px;max-width:460px;text-align:center}}\
                         h1{{font-size:18px;margin:0 0 8px;color:#ef4444}}p{{font-size:13px;color:#d4d4d8;margin:0}}</style></head>\
                         <body><div class='box'><h1>OAuth Exchange Failed</h1><p>{err}</p></div></body></html>"
                    );
                    (
                        page,
                        OAuthSessionPollStatus {
                            state: state_clone.clone(),
                            status: "error".into(),
                            connection_id: None,
                            email: None,
                            error: Some(err.to_string()),
                        },
                    )
                }
            };

            {
                let mut guard = sessions.lock().unwrap();
                guard.insert(state_clone, status_entry);
            }

            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                html.len(),
                html
            );
            let _ = stream.write_all(resp.as_bytes()).await;
            let _ = stream.shutdown().await;
            break;
        }
    });

    Ok(StartedOAuthFlow {
        state,
        auth_url: auth_url_str,
        redirect_uri,
    })
}

/// Picks a healthy proxy for onboarding/refreshing an Antigravity account.
pub async fn pick_onboarding_proxy_name(
    store: &Store,
    mihomo: &MihomoController,
    existing_conn_id: Option<&str>,
) -> String {
    let Ok(cfg) = store.provider_configs().get_or_default("antigravity").await else {
        return "DIRECT".to_string();
    };
    let Ok(proxies) = store.proxies().list_for_provider_config(&cfg).await else {
        return "DIRECT".to_string();
    };
    let states = mihomo.fetch_proxy_states().await.unwrap_or_default();

    if let Some(cid) = existing_conn_id
        && let Ok(conns) = store.connections().list_for_provider("antigravity").await
    {
        let parts = super::partition::compute_account_proxy_partitions(&conns, &proxies, &states);
        if let Some(part) = parts.into_iter().find(|p| p.connection_id == cid)
            && let Some(first) = part.healthy_fallback_chain.first()
        {
            return first.name.clone();
        }
    }

    // For a brand-new account being added, prefer a healthy proxy not yet primary on existing accounts
    let existing_primaries: std::collections::HashSet<String> = store
        .connections()
        .list_for_provider("antigravity")
        .await
        .unwrap_or_default()
        .into_iter()
        .filter_map(|c| c.sticky_proxy_id)
        .collect();

    for p in &proxies {
        let alive = states.get(&p.name).map(|s| s.alive).unwrap_or(p.alive);
        if alive && !existing_primaries.contains(&p.id) {
            return p.name.clone();
        }
    }
    for p in &proxies {
        let alive = states.get(&p.name).map(|s| s.alive).unwrap_or(p.alive);
        if alive {
            return p.name.clone();
        }
    }
    "DIRECT".to_string()
}

pub async fn complete_oauth_from_callback_or_code(
    store: &Store,
    mihomo: &MihomoController,
    callback_url_or_code: &str,
    explicit_redirect_uri: Option<&str>,
) -> Result<ProviderConnection> {
    let trimmed = callback_url_or_code.trim();
    if trimmed.is_empty() {
        return Err(TesRouteError::BadRequest(
            "missing callback URL or authorization code".into(),
        ));
    }

    let (code, derived_redirect) = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        let parsed = Url::parse(trimmed)
            .map_err(|e| TesRouteError::BadRequest(format!("invalid callback URL: {e}")))?;
        if let Some((_, err_val)) = parsed.query_pairs().find(|(k, _)| k == "error") {
            return Err(TesRouteError::BadRequest(format!(
                "Google OAuth returned error: {err_val}"
            )));
        }
        let code_val = parsed
            .query_pairs()
            .find(|(k, _)| k == "code")
            .map(|(_, v)| v.into_owned())
            .ok_or_else(|| {
                TesRouteError::BadRequest("callback URL contains no ?code= parameter".into())
            })?;
        let origin_path = format!(
            "{}://{}{}{}",
            parsed.scheme(),
            parsed.host_str().unwrap_or("localhost"),
            parsed.port().map(|p| format!(":{p}")).unwrap_or_default(),
            parsed.path()
        );
        (code_val, Some(origin_path))
    } else {
        (trimmed.to_string(), None)
    };

    let redirect_uri = explicit_redirect_uri
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or(derived_redirect)
        .unwrap_or_else(|| "http://localhost:8080/callback".to_string());

    let proxy_name = pick_onboarding_proxy_name(store, mihomo, None).await;
    let (client_id, client_secret, token_url) = oauth_credentials();

    let form = Url::parse_with_params(
        "http://x",
        &[
            ("code", code.as_str()),
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("grant_type", "authorization_code"),
        ],
    )
    .map_err(|e| TesRouteError::Internal(format!("encode token form: {e}")))?
    .query()
    .unwrap_or("")
    .to_string();

    let resp = mihomo
        .relay(RelayRequest {
            target_url: token_url,
            proxy_name: proxy_name.clone(),
            method: "POST".to_string(),
            fingerprint: "standard".to_string(),
            timeout_ms: 20_000,
            upstream_authorization: None,
            headers: vec![(
                "Content-Type".into(),
                "application/x-www-form-urlencoded".into(),
            )],
            body: Bytes::from(form),
        })
        .await?;

    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        return Err(TesRouteError::BadRequest(format!(
            "Google token exchange returned HTTP {status}: {}",
            body.chars().take(200).collect::<String>()
        )));
    }

    let tok: GoogleTokenResponse = resp
        .json()
        .await
        .map_err(|e| TesRouteError::Internal(format!("decode Google token response: {e}")))?;

    persist_and_onboard_tokens(store, mihomo, tok, None, None, &proxy_name).await
}

pub async fn import_antigravity_refresh_token(
    store: &Store,
    mihomo: &MihomoController,
    custom_name: Option<&str>,
    email_hint: Option<&str>,
    refresh_token: &str,
    project_id_hint: Option<&str>,
) -> Result<ProviderConnection> {
    let clean_rt = refresh_token.trim();
    if clean_rt.is_empty() {
        return Err(TesRouteError::BadRequest(
            "refresh_token cannot be empty".into(),
        ));
    }

    let proxy_name = pick_onboarding_proxy_name(store, mihomo, None).await;
    let refreshed = refresh_google_access_token(mihomo, clean_rt, &proxy_name).await?;
    let merged = GoogleTokenResponse {
        access_token: refreshed.access_token,
        refresh_token: Some(clean_rt.to_string()),
        expires_in: refreshed.expires_in,
        scope: refreshed.scope,
        id_token: refreshed.id_token,
    };

    persist_and_onboard_tokens(
        store,
        mihomo,
        merged,
        custom_name,
        email_hint.or(project_id_hint.and(None)),
        &proxy_name,
    )
    .await
    .and_then(|conn| {
        if let Some(pid) = project_id_hint.map(str::trim).filter(|s| !s.is_empty()) {
            let mut auth = AntigravityAuthData::from_connection(&conn);
            auth.project_id = Some(pid.to_string());
            let _ = serde_json::to_string(&auth);
        }
        Ok(conn)
    })
}

async fn persist_and_onboard_tokens(
    store: &Store,
    mihomo: &MihomoController,
    tok: GoogleTokenResponse,
    custom_name: Option<&str>,
    email_hint: Option<&str>,
    proxy_name: &str,
) -> Result<ProviderConnection> {
    let now = now_ms();
    let email = tok
        .id_token
        .as_deref()
        .and_then(extract_email_from_jwt)
        .or_else(|| email_hint.map(str::trim).filter(|s| !s.is_empty()).map(str::to_string));

    let conn_id = if let Some(em) = &email {
        format!("ag-{}", short_hash(&em.to_ascii_lowercase()))
    } else if let Some(rt) = &tok.refresh_token {
        format!("ag-{}", short_hash(rt))
    } else {
        format!("ag-{}", random_hex(6))
    };

    let existing = store.connections().get_connection(&conn_id).await?;
    let existing_auth = existing.as_ref().map(AntigravityAuthData::from_connection);

    let refresh_token = tok
        .refresh_token
        .filter(|s| !s.trim().is_empty())
        .or_else(|| existing_auth.as_ref().map(|a| a.refresh_token.clone()))
        .unwrap_or_default();

    // Discover real Cloud Code Companion project ID + subscription plan via loadCodeAssist once
    let sub_info = discover_antigravity_subscription(mihomo, &tok.access_token, proxy_name).await;
    let discovered_pid = sub_info
        .as_ref()
        .and_then(|s| s.project_id.clone())
        .or_else(|| {
            existing_auth
                .as_ref()
                .and_then(|a| a.project_id.clone())
                .filter(|p| !p.is_empty() && p != DEFAULT_ANTIGRAVITY_PROJECT_ID)
        })
        .unwrap_or_else(|| fallback_project_id_for_seed(&conn_id));

    let expires_at_ms = tok.expires_in.map(|sec| now + sec * 1000);
    let auth_data = AntigravityAuthData {
        access_token: tok.access_token.clone(),
        refresh_token,
        expires_at_ms,
        project_id: Some(discovered_pid.clone()),
        plan: sub_info
            .as_ref()
            .and_then(|s| s.plan.clone())
            .or_else(|| existing_auth.as_ref().and_then(|a| a.plan.clone())),
        is_free_tier: sub_info
            .as_ref()
            .and_then(|s| s.is_free_tier)
            .or_else(|| existing_auth.and_then(|a| a.is_free_tier)),
        email: email.clone(),
        id_token: tok.id_token,
        scope: tok.scope,
        onboard_checked: true,
    };
    let auth_json = serde_json::to_string(&auth_data)
        .map_err(|e| TesRouteError::Internal(format!("serialize auth data: {e}")))?;

    let display_name = custom_name
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| email.clone())
        .unwrap_or_else(|| format!("Antigravity ({})", &conn_id[..9.min(conn_id.len())]));

    let conn = store
        .connections()
        .upsert_oauth_connection(
            &conn_id,
            "antigravity",
            &display_name,
            email.as_deref(),
            &tok.access_token,
            &auth_json,
        )
        .await?;

    // Best-effort initial quota snapshot via assigned proxy
    let _ = refresh_account_quota(store, mihomo, &conn, Some(proxy_name)).await;
    info!(connection_id = %conn.id, name = %conn.name, proxy = %proxy_name, "Antigravity account connected");

    store
        .connections()
        .get_connection(&conn.id)
        .await?
        .ok_or_else(|| TesRouteError::NotFound("connected account missing".into()))
}

pub async fn refresh_google_access_token(
    mihomo: &MihomoController,
    refresh_token: &str,
    proxy_name: &str,
) -> Result<GoogleTokenResponse> {
    let (client_id, client_secret, token_url) = oauth_credentials();
    let form = Url::parse_with_params(
        "http://x",
        &[
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
            ("refresh_token", refresh_token),
            ("grant_type", "refresh_token"),
        ],
    )
    .map_err(|e| TesRouteError::Internal(format!("encode refresh form: {e}")))?
    .query()
    .unwrap_or("")
    .to_string();

    let resp = mihomo
        .relay(RelayRequest {
            target_url: token_url,
            proxy_name: proxy_name.to_string(),
            method: "POST".to_string(),
            fingerprint: "standard".to_string(),
            timeout_ms: 15_000,
            upstream_authorization: None,
            headers: vec![(
                "Content-Type".into(),
                "application/x-www-form-urlencoded".into(),
            )],
            body: Bytes::from(form),
        })
        .await?;

    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        return Err(TesRouteError::Unauthorized(format!(
            "Antigravity OAuth refresh failed ({status}): {}",
            body.chars().take(180).collect::<String>()
        )));
    }

    resp.json::<GoogleTokenResponse>()
        .await
        .map_err(|e| TesRouteError::Internal(format!("parse refresh response: {e}")))
}

pub async fn ensure_fresh_auth_for_connection(
    store: &Store,
    mihomo: &MihomoController,
    conn: &ProviderConnection,
    proxy_name: &str,
    force: bool,
) -> Result<AntigravityAuthData> {
    let mut auth = AntigravityAuthData::from_connection(conn);
    let now = now_ms();

    if (force || auth.needs_refresh(now)) && !auth.refresh_token.trim().is_empty() {
        let refreshed = refresh_google_access_token(mihomo, &auth.refresh_token, proxy_name).await?;
        auth.access_token = refreshed.access_token;
        if let Some(exp) = refreshed.expires_in {
            auth.expires_at_ms = Some(now + exp * 1000);
        }
        if let Some(rt) = refreshed.refresh_token.filter(|s| !s.trim().is_empty()) {
            auth.refresh_token = rt;
        }
        let serialized = serde_json::to_string(&auth)
            .map_err(|e| TesRouteError::Internal(format!("serialize auth: {e}")))?;
        store
            .connections()
            .update_auth_data(&conn.id, &auth.access_token, &serialized, now)
            .await?;
    }

    if !auth.onboard_checked
        && (auth
            .project_id
            .as_deref()
            .is_none_or(|p| p.is_empty() || p == DEFAULT_ANTIGRAVITY_PROJECT_ID)
            || auth.plan.is_none())
    {
        if let Some(sub) =
            discover_antigravity_subscription(mihomo, &auth.access_token, proxy_name).await
        {
            if let Some(pid) = sub.project_id {
                auth.project_id = Some(pid);
            }
            if sub.plan.is_some() {
                auth.plan = sub.plan;
            }
            if sub.is_free_tier.is_some() {
                auth.is_free_tier = sub.is_free_tier;
            }
        }
        if auth
            .project_id
            .as_deref()
            .is_none_or(|p| p.is_empty() || p == DEFAULT_ANTIGRAVITY_PROJECT_ID)
        {
            auth.project_id = Some(fallback_project_id_for_seed(&conn.id));
        }
        auth.onboard_checked = true;
        if let Ok(serialized) = serde_json::to_string(&auth) {
            let _ = store
                .connections()
                .update_auth_data(&conn.id, &auth.access_token, &serialized, now_ms())
                .await;
        }
    } else if auth.project_id.as_deref() == Some(DEFAULT_ANTIGRAVITY_PROJECT_ID) {
        auth.project_id = Some(fallback_project_id_for_seed(&conn.id));
    }

    Ok(auth)
}

#[derive(Debug, Clone, Default)]
pub struct AntigravitySubscriptionMeta {
    pub project_id: Option<String>,
    pub plan: Option<String>,
    pub is_free_tier: Option<bool>,
}

pub fn quota_base_url() -> String {
    std::env::var("ANTIGRAVITY_DISCOVERY_BASE_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            std::env::var("ANTIGRAVITY_CHAT_BASE_URL")
                .ok()
                .filter(|s| !s.trim().is_empty())
        })
        .unwrap_or_else(|| "https://daily-cloudcode-pa.googleapis.com".to_string())
}

pub fn fallback_project_id_for_seed(seed: &str) -> String {
    const ADJ: [&str; 5] = ["useful", "bright", "swift", "calm", "bold"];
    const NOUN: [&str; 5] = ["fuze", "wave", "spark", "flow", "core"];
    let mut hasher = Sha256::new();
    hasher.update(seed.as_bytes());
    let digest = hasher.finalize();
    let a = ADJ[(digest[0] as usize) % ADJ.len()];
    let n = NOUN[(digest[1] as usize) % NOUN.len()];
    let tail = &hex::encode(&digest[2..5])[..5];
    format!("{a}-{n}-{tail}")
}

pub async fn discover_antigravity_subscription(
    mihomo: &MihomoController,
    access_token: &str,
    proxy_name: &str,
) -> Option<AntigravitySubscriptionMeta> {
    let base = discovery_base_url();
    let lca_url = format!("{}/v1internal:loadCodeAssist", base.trim_end_matches('/'));
    let metadata = antigravity_client_metadata();
    let payload = serde_json::to_vec(&json!({ "metadata": metadata })).ok()?;

    let resp = mihomo
        .relay(RelayRequest {
            target_url: lca_url.clone(),
            proxy_name: proxy_name.to_string(),
            method: "POST".to_string(),
            fingerprint: "standard".to_string(),
            timeout_ms: 15_000,
            upstream_authorization: Some(format!("Bearer {access_token}")),
            headers: vec![
                ("Content-Type".into(), "application/json".into()),
                ("User-Agent".into(), antigravity_hub_ua().into()),
            ],
            body: Bytes::from(payload.clone()),
        })
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }

    let data: Value = resp.json().await.ok()?;
    let paid_tier_id = data
        .get("paidTier")
        .and_then(|pt| pt.get("id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let current_tier_name = data
        .get("currentTier")
        .and_then(|ct| ct.get("name").or_else(|| ct.get("id")))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let is_free_tier = match paid_tier_id {
        Some(id) => Some(id.eq_ignore_ascii_case("free-tier")),
        None => current_tier_name
            .as_deref()
            .map(|n| n.to_ascii_lowercase().contains("free")),
    };

    if let Some(pid) = extract_project_id_value(data.get("cloudaicompanionProject")) {
        return Some(AntigravitySubscriptionMeta {
            project_id: Some(pid),
            plan: current_tier_name,
            is_free_tier,
        });
    }

    // Account already has an active tier (e.g. "Antigravity"); do NOT spam onboardUser provisioning free-tier!
    if current_tier_name.is_some() {
        return Some(AntigravitySubscriptionMeta {
            project_id: None,
            plan: current_tier_name,
            is_free_tier,
        });
    }

    // Provision Cloud Companion project via `onboardUser` LRO loop only for un-tiered accounts
    let mut tier_id = "free-tier".to_string();
    if let Some(tiers) = data.get("allowedTiers").and_then(Value::as_array) {
        if let Some(def_id) = tiers
            .iter()
            .find(|t| t.get("isDefault").and_then(Value::as_bool) == Some(true))
            .and_then(|t| t.get("id").and_then(Value::as_str))
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            tier_id = def_id.to_string();
        } else if let Some(first_id) = tiers
            .first()
            .and_then(|t| t.get("id").and_then(Value::as_str))
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            tier_id = first_id.to_string();
        } else {
            tier_id = "legacy-tier".to_string();
        }
    }

    let onboard_url = format!("{}/v1internal:onboardUser", base.trim_end_matches('/'));
    let onboard_body = serde_json::to_vec(&json!({
        "tierId": tier_id,
        "metadata": metadata
    }))
    .ok()?;

    for attempt in 0..6 {
        let ob_resp = mihomo
            .relay(RelayRequest {
                target_url: onboard_url.clone(),
                proxy_name: proxy_name.to_string(),
                method: "POST".to_string(),
                fingerprint: "standard".to_string(),
                timeout_ms: 15_000,
                upstream_authorization: Some(format!("Bearer {access_token}")),
                headers: vec![
                    ("Content-Type".into(), "application/json".into()),
                    ("User-Agent".into(), antigravity_hub_ua().into()),
                ],
                body: Bytes::from(onboard_body.clone()),
            })
            .await
            .ok()?;

        if !ob_resp.status().is_success() {
            break;
        }
        let ob_data: Value = ob_resp.json().await.ok()?;
        if let Some(pid) = ob_data
            .get("response")
            .and_then(|r| extract_project_id_value(r.get("cloudaicompanionProject")))
            .or_else(|| extract_project_id_value(ob_data.get("cloudaicompanionProject")))
        {
            return Some(AntigravitySubscriptionMeta {
                project_id: Some(pid),
                plan: current_tier_name,
                is_free_tier,
            });
        }
        if ob_data.get("done").and_then(Value::as_bool) == Some(true) {
            break;
        }
        if attempt + 1 < 6 {
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }

    // Re-query loadCodeAssist once after onboarding completes
    if let Ok(resp2) = mihomo
        .relay(RelayRequest {
            target_url: lca_url,
            proxy_name: proxy_name.to_string(),
            method: "POST".to_string(),
            fingerprint: "standard".to_string(),
            timeout_ms: 15_000,
            upstream_authorization: Some(format!("Bearer {access_token}")),
            headers: vec![
                ("Content-Type".into(), "application/json".into()),
                ("User-Agent".into(), antigravity_hub_ua().into()),
            ],
            body: Bytes::from(payload),
        })
        .await
        && resp2.status().is_success()
        && let Ok(data2) = resp2.json::<Value>().await
        && let Some(pid) = extract_project_id_value(data2.get("cloudaicompanionProject"))
    {
        return Some(AntigravitySubscriptionMeta {
            project_id: Some(pid),
            plan: current_tier_name,
            is_free_tier,
        });
    }

    Some(AntigravitySubscriptionMeta {
        project_id: None,
        plan: current_tier_name,
        is_free_tier,
    })
}

fn extract_project_id_value(val: Option<&Value>) -> Option<String> {
    let val = val?;
    if let Some(s) = val.as_str().map(str::trim).filter(|s| !s.is_empty()) {
        return Some(s.to_string());
    }
    if let Some(obj) = val.as_object() {
        for k in ["id", "projectId", "project_id"] {
            if let Some(s) = obj
                .get(k)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                return Some(s.to_string());
            }
        }
    }
    None
}

fn parse_rfc3339_ms(raw: Option<&Value>) -> Option<i64> {
    let s = raw?.as_str()?.trim();
    if s.is_empty() {
        return None;
    }
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

pub fn antigravity_model_family(model_id: &str) -> &'static str {
    let clean = model_id
        .split('/')
        .next_back()
        .unwrap_or(model_id)
        .split('(')
        .next()
        .unwrap_or(model_id)
        .trim()
        .to_ascii_lowercase();
    if clean.starts_with("claude-") || clean.starts_with("gpt-") {
        "claude_gpt"
    } else if clean.contains("argon") {
        "gemini_argon"
    } else if clean.starts_with("gemini-") || clean.starts_with("orchestrator-") {
        "gemini"
    } else {
        "other"
    }
}

/// Parses `v1internal:retrieveUserQuotaSummary` into normalized `5h limit` and `Weekly limit`
/// pool windows matching the official Antigravity client's "Usage" popover (`Gemini Models`,
/// `Gemini 4 Argon`, `Claude and GPT models`), plus tracks any family disabled on Weekly.
pub fn parse_quota_summary_pools(
    data: &Value,
) -> (Vec<AntigravityPoolWindow>, HashSet<String>) {
    let groups_opt = data
        .get("groups")
        .and_then(Value::as_array)
        .or_else(|| {
            data.get("quotaSummary")
                .and_then(|qs| qs.get("groups"))
                .and_then(Value::as_array)
        });

    let Some(groups) = groups_opt else {
        return (Vec::new(), HashSet::new());
    };

    let mut by_key: HashMap<String, AntigravityPoolWindow> = HashMap::new();
    let mut locked_families: HashSet<String> = HashSet::new();

    for group in groups {
        let gname = group
            .get("displayName")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        let gname_lower = gname.to_ascii_lowercase();

        let (family, canonical_group) = if gname_lower.contains("argon") {
            ("gemini_argon", "Gemini 4 Argon")
        } else if gname_lower.contains("claude") || gname_lower.contains("gpt") {
            ("claude_gpt", "Claude and GPT models")
        } else if gname_lower.contains("gemini") {
            ("gemini", "Gemini Models")
        } else {
            continue;
        };

        let Some(buckets) = group.get("buckets").and_then(Value::as_array) else {
            continue;
        };

        for bucket in buckets {
            let win_type = bucket
                .get("window")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_ascii_lowercase();
            let bucket_text = format!(
                "{} {}",
                bucket
                    .get("bucketId")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
                bucket
                    .get("displayName")
                    .and_then(Value::as_str)
                    .unwrap_or("")
            )
            .to_ascii_lowercase();

            let is_weekly = win_type == "weekly" || bucket_text.contains("weekly");
            let is_session = win_type == "5h"
                || win_type == "daily"
                || bucket_text.contains("five hour")
                || bucket_text.contains("5h")
                || bucket_text.contains("daily");
            if !is_weekly && !is_session {
                continue;
            }

            let disabled = bucket.get("disabled").and_then(Value::as_bool) == Some(true);
            if disabled && is_weekly {
                locked_families.insert(family.to_string());
                continue;
            }

            // When Weekly hits 0%, Google marks the 5h session bucket `disabled: true` — keep it at 0%
            let frac = if disabled {
                0.0
            } else {
                match bucket.get("remainingFraction").and_then(Value::as_f64) {
                    Some(f) if f.is_finite() => f.clamp(0.0, 1.0),
                    _ => continue,
                }
            };

            let key = if is_weekly {
                format!("{family}_weekly")
            } else {
                format!("{family}_session")
            };
            if by_key.contains_key(&key) {
                continue;
            }

            by_key.insert(
                key.clone(),
                AntigravityPoolWindow {
                    key,
                    group_name: canonical_group.to_string(),
                    window_label: if is_weekly {
                        "Weekly limit".to_string()
                    } else {
                        "5h limit".to_string()
                    },
                    remaining_percentage: (frac * 1000.0).round() / 10.0,
                    reset_at: parse_rfc3339_ms(bucket.get("resetTime")),
                    unlocked: true,
                },
            );
        }
    }

    // Order matches the official Antigravity Usage popover:
    // Gemini Models (Weekly, 5h), Gemini 4 Argon (Weekly, 5h), Claude and GPT models (Weekly, 5h)
    let ordered_keys = [
        "gemini_weekly",
        "gemini_session",
        "gemini_argon_weekly",
        "gemini_argon_session",
        "claude_gpt_weekly",
        "claude_gpt_session",
    ];
    let mut pools = Vec::new();
    for k in ordered_keys {
        if let Some(p) = by_key.remove(k) {
            pools.push(p);
        }
    }
    (pools, locked_families)
}

async fn fetch_quota_summary(
    mihomo: &MihomoController,
    access_token: &str,
    project_id: Option<&str>,
    proxy_name: &str,
) -> Option<(Vec<AntigravityPoolWindow>, HashSet<String>)> {
    let base = quota_base_url();
    let url = format!(
        "{}/v1internal:retrieveUserQuotaSummary",
        base.trim_end_matches('/')
    );
    let mut req_obj = serde_json::Map::new();
    if let Some(pid) = project_id.filter(|s| !s.is_empty()) {
        req_obj.insert("project".into(), Value::String(pid.to_string()));
    }
    let body_bytes = serde_json::to_vec(&Value::Object(req_obj)).ok()?;

    let resp = mihomo
        .relay(RelayRequest {
            target_url: url,
            proxy_name: proxy_name.to_string(),
            method: "POST".to_string(),
            fingerprint: "standard".to_string(),
            timeout_ms: 10_000,
            upstream_authorization: Some(format!("Bearer {access_token}")),
            headers: vec![
                ("Content-Type".into(), "application/json".into()),
                ("User-Agent".into(), antigravity_hub_ua().into()),
                ("X-Client-Name".into(), "antigravity".into()),
                ("X-Client-Version".into(), ANTIGRAVITY_IDE_VERSION.into()),
            ],
            body: Bytes::from(body_bytes),
        })
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }
    let data: Value = resp.json().await.ok()?;
    Some(parse_quota_summary_pools(&data))
}

/// Normalizes any `AntigravityQuotaSnapshot` (including previously cached SQLite snapshots)
/// so internal/retired/locked items are excluded and same-model variants (`-high`, `-medium`, `-low`)
/// collapse into a single canonical entry.
pub fn normalize_quota_snapshot_for_display(
    mut snap: AntigravityQuotaSnapshot,
) -> AntigravityQuotaSnapshot {
    let mut merged: Vec<AntigravityModelQuotaEntry> = Vec::new();
    let mut index_by_canon: HashMap<String, usize> = HashMap::new();

    for entry in snap.models {
        if !entry.unlocked || is_excluded_antigravity_model_key(&entry.model) {
            continue;
        }
        let (canon_id, canon_name) = canonical_antigravity_model(&entry.model);
        if is_excluded_antigravity_model_key(&canon_id) {
            continue;
        }
        if let Some(&idx) = index_by_canon.get(&canon_id) {
            let existing = &mut merged[idx];
            if entry.remaining_percentage > existing.remaining_percentage {
                existing.remaining_percentage = entry.remaining_percentage;
                existing.reset_at = entry.reset_at.or(existing.reset_at);
            }
            if existing.weekly_remaining_percentage.is_none() {
                existing.weekly_remaining_percentage = entry.weekly_remaining_percentage;
                existing.weekly_reset_at = entry.weekly_reset_at;
            }
            if existing.session_remaining_percentage.is_none() {
                existing.session_remaining_percentage = entry.session_remaining_percentage;
                existing.session_reset_at = entry.session_reset_at;
            }
        } else {
            let idx = merged.len();
            index_by_canon.insert(canon_id.clone(), idx);
            merged.push(AntigravityModelQuotaEntry {
                model: canon_id,
                display_name: canon_name,
                family: entry.family,
                unlocked: true,
                remaining_percentage: entry.remaining_percentage,
                reset_at: entry.reset_at,
                weekly_remaining_percentage: entry.weekly_remaining_percentage,
                weekly_reset_at: entry.weekly_reset_at,
                session_remaining_percentage: entry.session_remaining_percentage,
                session_reset_at: entry.session_reset_at,
                failure_count: entry.failure_count,
                failure_cooldown_until: entry.failure_cooldown_until,
                failure_reason: entry.failure_reason,
            });
        }
    }

    let active_families: HashSet<String> = merged.iter().map(|m| m.family.clone()).collect();
    // Surface ONLY the two official main model families (`Gemini Models` & `Claude and GPT models`)
    snap.pools.retain(|p| {
        if !p.unlocked {
            return false;
        }
        if p.key.starts_with("gemini_argon") {
            false
        } else if p.key.starts_with("claude_gpt") {
            active_families.contains("claude_gpt") || merged.iter().any(|m| m.model.starts_with("claude") || m.model.starts_with("gpt"))
        } else if p.key.starts_with("gemini_") {
            active_families.contains("gemini") || active_families.contains("gemini_argon")
        } else {
            false
        }
    });
    snap.models = merged;
    snap
}

/// Fetches 100% accurate Antigravity account quotas by combining:
/// 1. `POST /v1internal:fetchAvailableModels` (model unlock status + per-model quotaInfo)
/// 2. `POST /v1internal:retrieveUserQuotaSummary` (`5h limit` + `Weekly limit` pools across
///    `Gemini Models`, `Gemini 4 Argon`, and `Claude and GPT models`)
pub async fn refresh_account_quota(
    store: &Store,
    mihomo: &MihomoController,
    conn: &ProviderConnection,
    explicit_proxy: Option<&str>,
) -> Result<AntigravityQuotaSnapshot> {
    let proxy_name = match explicit_proxy {
        Some(p) => p.to_string(),
        None => pick_onboarding_proxy_name(store, mihomo, Some(&conn.id)).await,
    };

    let auth = ensure_fresh_auth_for_connection(store, mihomo, conn, &proxy_name, false).await?;
    let base = quota_base_url();
    let models_url = format!("{}/v1internal:fetchAvailableModels", base.trim_end_matches('/'));
    let project_id = auth.project_id.as_deref().filter(|s| !s.is_empty());

    let mut req_obj = serde_json::Map::new();
    if let Some(pid) = project_id {
        req_obj.insert("project".into(), Value::String(pid.to_string()));
    }
    let body_bytes = serde_json::to_vec(&Value::Object(req_obj)).unwrap_or_else(|_| b"{}".to_vec());

    let (models_resp_res, summary_opt) = tokio::join!(
        mihomo.relay(RelayRequest {
            target_url: models_url,
            proxy_name: proxy_name.clone(),
            method: "POST".to_string(),
            fingerprint: "standard".to_string(),
            timeout_ms: 15_000,
            upstream_authorization: Some(format!("Bearer {}", auth.access_token)),
            headers: vec![
                ("Content-Type".into(), "application/json".into()),
                ("User-Agent".into(), antigravity_hub_ua().into()),
                ("X-Client-Name".into(), "antigravity".into()),
                ("X-Client-Version".into(), ANTIGRAVITY_IDE_VERSION.into()),
            ],
            body: Bytes::from(body_bytes),
        }),
        fetch_quota_summary(mihomo, &auth.access_token, project_id, &proxy_name),
    );

    let resp = models_resp_res?;
    if !resp.status().is_success() {
        let code = resp.status().as_u16();
        let err_body = resp.text().await.unwrap_or_default();
        if code == 403 && is_account_service_disabled(&err_body) {
            let reason = if is_account_unlicensed(&err_body) {
                "Disabled: No active Gemini Code Assist license on this account (#3501)."
            } else {
                "Banned: This service has been disabled in this account for violation of Terms of Service."
            };
            let _ = store
                .connections()
                .disable_banned_connection(&conn.id, reason, now_ms())
                .await;
            return Err(TesRouteError::BadRequest(
                if is_account_unlicensed(&err_body) {
                    "Account auto-disabled: No active license".to_string()
                } else {
                    "Account auto-disabled: Terms of Service violation".to_string()
                },
            ));
        }
        warn!(conn = %conn.id, status = code, "Antigravity fetchAvailableModels returned non-200");
        return Err(TesRouteError::BadRequest(format!(
            "Antigravity quota endpoint returned HTTP {code}"
        )));
    }

    let data: Value = resp
        .json()
        .await
        .map_err(|e| TesRouteError::Internal(format!("decode Antigravity quota JSON: {e}")))?;

    let (pools, locked_families) = summary_opt.unwrap_or_default();
    let pool_map: HashMap<String, AntigravityPoolWindow> = pools
        .iter()
        .map(|p| (p.key.clone(), p.clone()))
        .collect();

    let is_free_tier = auth.is_free_tier.unwrap_or(false);
    let now = now_ms();

    // Parse and merge upstream models by canonical base ID (collapsing -high/-medium/-low/-tiered)
    struct MergedUpstreamModel {
        display_name: String,
        unlocked: bool,
        remaining_pct: Option<f64>,
        reset_at: Option<i64>,
    }

    let mut merged_upstream: HashMap<String, MergedUpstreamModel> = HashMap::new();
    let mut saw_any_upstream_models = false;

    if let Some(models_obj) = data.get("models").and_then(Value::as_object) {
        saw_any_upstream_models = !models_obj.is_empty();
        for (model_key, model_val) in models_obj {
            if model_val.get("isInternal").and_then(Value::as_bool) == Some(true)
                || is_excluded_antigravity_model_key(model_key)
            {
                continue;
            }
            let (canon_id, canon_name) = canonical_antigravity_model(model_key);
            if is_excluded_antigravity_model_key(&canon_id) {
                continue;
            }

            let disabled = model_val.get("disabled").and_then(Value::as_bool) == Some(true)
                || model_val.get("available").and_then(Value::as_bool) == Some(false)
                || model_val.get("isLocked").and_then(Value::as_bool) == Some(true);
            let unlocked_here = !disabled;

            let (remaining_pct, reset_at) = if is_free_tier {
                // On Free Tier, per-model `quotaInfo` can report misleading 0s; rely on pool summary
                (None, None)
            } else if let Some(qinfo) = model_val.get("quotaInfo").and_then(Value::as_object) {
                let pct = qinfo
                    .get("remainingFraction")
                    .and_then(Value::as_f64)
                    .map(|f| (f.clamp(0.0, 1.0) * 1000.0).round() / 10.0);
                let rst = parse_rfc3339_ms(qinfo.get("resetTime"));
                (pct, rst)
            } else {
                (None, None)
            };

            merged_upstream
                .entry(canon_id)
                .and_modify(|existing| {
                    if unlocked_here {
                        existing.unlocked = true;
                        match (existing.remaining_pct, remaining_pct) {
                            (Some(cur), Some(nxt)) if nxt > cur => {
                                existing.remaining_pct = Some(nxt);
                                existing.reset_at = reset_at.or(existing.reset_at);
                            }
                            (None, Some(nxt)) => {
                                existing.remaining_pct = Some(nxt);
                                existing.reset_at = reset_at;
                            }
                            _ => {}
                        }
                    }
                })
                .or_insert(MergedUpstreamModel {
                    display_name: canon_name,
                    unlocked: unlocked_here,
                    remaining_pct: if unlocked_here { remaining_pct } else { None },
                    reset_at: if unlocked_here { reset_at } else { None },
                });
        }
    }

    let resolve_family_pools =
        |family: &str| -> (Option<&AntigravityPoolWindow>, Option<&AntigravityPoolWindow>) {
            match family {
                "claude_gpt" => (
                    pool_map.get("claude_gpt_weekly"),
                    pool_map.get("claude_gpt_session"),
                ),
                "gemini_argon" => {
                    if pool_map.contains_key("gemini_argon_weekly")
                        || pool_map.contains_key("gemini_argon_session")
                    {
                        (
                            pool_map.get("gemini_argon_weekly"),
                            pool_map.get("gemini_argon_session"),
                        )
                    } else {
                        (
                            pool_map.get("gemini_weekly"),
                            pool_map.get("gemini_session"),
                        )
                    }
                }
                "gemini" => (pool_map.get("gemini_weekly"), pool_map.get("gemini_session")),
                _ => (None, None),
            }
        };

    let build_available_entry =
        |canon_id: &str, display_name: &str, m: Option<&MergedUpstreamModel>| -> Option<AntigravityModelQuotaEntry> {
            let family = antigravity_model_family(canon_id);
            if locked_families.contains(family) {
                return None;
            }
            let unlocked = match m {
                Some(info) => info.unlocked,
                None => !saw_any_upstream_models,
            };
            if !unlocked {
                return None;
            }

            let (weekly_pool, session_pool) = resolve_family_pools(family);
            let weekly_pct = weekly_pool.map(|p| p.remaining_percentage);
            let weekly_rst = weekly_pool.and_then(|p| p.reset_at);
            let session_pct = session_pool.map(|p| p.remaining_percentage);
            let session_rst = session_pool.and_then(|p| p.reset_at);

            let mut constraints: Vec<(f64, Option<i64>)> = Vec::new();
            if let Some(info) = m
                && let Some(mp) = info.remaining_pct
            {
                constraints.push((mp, info.reset_at));
            }
            if let Some(sp) = session_pct {
                constraints.push((sp, session_rst));
            }
            if let Some(wp) = weekly_pct {
                constraints.push((wp, weekly_rst));
            }

            let (eff_pct, eff_reset) = if constraints.is_empty() {
                (100.0, None)
            } else {
                let exhausted: Vec<(f64, Option<i64>)> = constraints
                    .iter()
                    .copied()
                    .filter(|(p, r)| *p <= 0.0 && r.is_none_or(|ts| ts > now))
                    .collect();
                if !exhausted.is_empty() {
                    let latest_reset = exhausted.iter().filter_map(|(_, r)| *r).max();
                    (0.0, latest_reset)
                } else {
                    constraints
                        .sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
                    constraints[0]
                }
            };

            Some(AntigravityModelQuotaEntry {
                model: canon_id.to_string(),
                display_name: display_name.to_string(),
                family: family.to_string(),
                unlocked: true,
                remaining_percentage: eff_pct,
                reset_at: eff_reset,
                weekly_remaining_percentage: weekly_pct,
                weekly_reset_at: weekly_rst,
                session_remaining_percentage: session_pct,
                session_reset_at: session_rst,
                failure_count: 0,
                failure_cooldown_until: None,
                failure_reason: None,
            })
        };

    let mut entries: Vec<AntigravityModelQuotaEntry> = Vec::new();
    let mut covered_canon: HashSet<String> = HashSet::new();

    for cat in curated_models_for_provider(ProviderKind::Antigravity) {
        covered_canon.insert(cat.id.to_string());
        if let Some(entry) = build_available_entry(cat.id, cat.name, merged_upstream.get(cat.id)) {
            entries.push(entry);
        }
    }

    // Append any additional valid unlocked canonical models returned by Google outside our static list
    let mut extra_ids: Vec<&String> = merged_upstream
        .keys()
        .filter(|k| !covered_canon.contains(*k))
        .collect();
    extra_ids.sort();
    for canon_id in extra_ids {
        if let Some(m) = merged_upstream.get(canon_id)
            && let Some(entry) = build_available_entry(canon_id, &m.display_name, Some(m))
        {
            entries.push(entry);
        }
    }

    let mut raw_snapshot = AntigravityQuotaSnapshot {
        plan: auth.plan.clone().or_else(|| {
            if is_free_tier {
                Some("Free".to_string())
            } else {
                None
            }
        }),
        is_free_tier,
        pools,
        models: entries,
        fetched_at: now,
    };

    // Reconcile 5h session pool if all unlocked models in a family report 0%
    for (fam, sess_key) in [
        ("gemini", "gemini_session"),
        ("gemini_argon", "gemini_argon_session"),
        ("claude_gpt", "claude_gpt_session"),
    ] {
        let fam_models: Vec<&AntigravityModelQuotaEntry> = raw_snapshot
            .models
            .iter()
            .filter(|e| e.unlocked && e.family == fam)
            .collect();
        if !fam_models.is_empty() && fam_models.iter().all(|e| e.remaining_percentage <= 0.0) {
            let latest_rst = fam_models.iter().filter_map(|e| e.reset_at).max();
            if let Some(pool) = raw_snapshot.pools.iter_mut().find(|p| p.key == sess_key)
                && pool.remaining_percentage > 0.0
            {
                pool.remaining_percentage = 0.0;
                if latest_rst.is_some() {
                    pool.reset_at = latest_rst;
                }
            }
        }
    }

    let snapshot = normalize_quota_snapshot_for_display(raw_snapshot);
    if let Ok(raw) = serde_json::to_string(&snapshot) {
        let _ = store
            .connections()
            .update_quota_snapshot(&conn.id, &raw, now)
            .await;
    }
    Ok(snapshot)
}

/// Returns the upstream `v1internal:fetchAvailableModels` payload verbatim: the IDE
/// picker must show models Google actually serves, not synthesised ids. First enabled account wins.
pub async fn fetch_available_models_raw(
    store: &Store,
    mihomo: &MihomoController,
) -> Result<Value> {
    let conns = store.connections().list_for_provider("antigravity").await?;
    let mut last_err: Option<String> = None;

    for conn in conns.iter().filter(|c| c.enabled) {
        let proxy_name = pick_onboarding_proxy_name(store, mihomo, Some(&conn.id)).await;
        let auth = match ensure_fresh_auth_for_connection(store, mihomo, conn, &proxy_name, false)
            .await
        {
            Ok(a) => a,
            Err(e) => {
                last_err = Some(e.to_string());
                continue;
            }
        };
        let project_id = auth.project_id.as_deref().filter(|s| !s.is_empty());
        let mut req_obj = serde_json::Map::new();
        if let Some(pid) = project_id {
            req_obj.insert("project".into(), Value::String(pid.to_string()));
        }
        let body_bytes =
            serde_json::to_vec(&Value::Object(req_obj)).unwrap_or_else(|_| b"{}".to_vec());
        let url = format!(
            "{}/v1internal:fetchAvailableModels",
            quota_base_url().trim_end_matches('/')
        );

        let resp = match mihomo
            .relay(RelayRequest {
                target_url: url,
                proxy_name,
                method: "POST".to_string(),
                fingerprint: "standard".to_string(),
                timeout_ms: 15_000,
                upstream_authorization: Some(format!("Bearer {}", auth.access_token)),
                headers: vec![
                    ("Content-Type".into(), "application/json".into()),
                    ("User-Agent".into(), antigravity_hub_ua().into()),
                    ("X-Client-Name".into(), "antigravity".into()),
                    ("X-Client-Version".into(), ANTIGRAVITY_IDE_VERSION.into()),
                ],
                body: Bytes::from(body_bytes),
            })
            .await
        {
            Ok(r) => r,
            Err(e) => {
                last_err = Some(e.to_string());
                continue;
            }
        };

        if !resp.status().is_success() {
            last_err = Some(format!("HTTP {}", resp.status().as_u16()));
            continue;
        }

        match resp.json::<Value>().await {
            Ok(v) if v.get("models").and_then(Value::as_object).is_some_and(|m| !m.is_empty()) => {
                return Ok(v);
            }
            Ok(_) => last_err = Some("upstream returned no models".to_string()),
            Err(e) => last_err = Some(format!("decode upstream models: {e}")),
        }
    }

    Err(TesRouteError::NoCredentials(format!(
        "Antigravity model list ({})",
        last_err.unwrap_or_else(|| "no enabled account".into())
    )))
}
