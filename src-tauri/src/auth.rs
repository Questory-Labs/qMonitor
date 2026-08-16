//! Device login token storage, health detect, and OAuth token calls.

use std::sync::{Mutex, OnceLock};

use keyring::Entry;
use serde::Deserialize;
use tokio::sync::Mutex as AsyncMutex;

use crate::config::{
    AppConfig, DetectedService, KEYRING_ACCESS, KEYRING_SERVICE, KEYRING_SESSION,
};
use crate::device;
use crate::oauth_loopback::REDIRECT_URI;
use crate::pkce;
use crate::push::http_client;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthState {
    pub base_url: String,
    pub webhook_url: String,
    pub has_access_token: bool,
    pub has_session_token: bool,
}

#[derive(Debug, Clone)]
pub struct LoginAttempt {
    pub state: String,
    pub verifier: String,
    pub device_id: String,
}

#[derive(Debug, Deserialize)]
struct HealthBody {
    ok: bool,
    service: String,
    #[serde(rename = "webOrigin")]
    web_origin: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
}

#[derive(Default)]
struct TokenCache {
    access: Option<String>,
    refresh: Option<String>,
    keyring_hydrated: bool,
}

fn token_cache() -> &'static Mutex<TokenCache> {
    static CACHE: OnceLock<Mutex<TokenCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(TokenCache::default()))
}

fn refresh_lock() -> &'static AsyncMutex<()> {
    static LOCK: OnceLock<AsyncMutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| AsyncMutex::new(()))
}

fn cache_put(access: Option<&str>, refresh: Option<&str>) {
    if let Ok(mut c) = token_cache().lock() {
        if let Some(a) = access {
            c.access = Some(a.to_string());
        }
        if let Some(r) = refresh {
            c.refresh = Some(r.to_string());
        }
    }
}

fn cache_clear() {
    if let Ok(mut c) = token_cache().lock() {
        c.access = None;
        c.refresh = None;
        c.keyring_hydrated = true;
    }
}

fn keyring_get(key: &str) -> Result<Option<String>, String> {
    let entry = Entry::new(KEYRING_SERVICE, key).map_err(|e| e.to_string())?;
    match entry.get_password() {
        Ok(p) if !p.is_empty() => Ok(Some(p)),
        Ok(_) => Ok(None),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

fn hydrate_keyring_once(cache: &mut TokenCache) {
    if cache.keyring_hydrated {
        return;
    }
    cache.keyring_hydrated = true;
    match keyring_get(KEYRING_ACCESS) {
        Ok(v) => {
            if cache.access.is_none() {
                cache.access = v;
            }
        }
        Err(e) => tracing::warn!(%e, "keyring access read failed; keeping cache"),
    }
    match keyring_get(KEYRING_SESSION) {
        Ok(v) => {
            if cache.refresh.is_none() {
                cache.refresh = v;
            }
        }
        Err(e) => tracing::warn!(%e, "keyring refresh read failed; keeping cache"),
    }
}

fn keyring_set(key: &str, value: &str, what: &str) {
    match Entry::new(KEYRING_SERVICE, key) {
        Ok(entry) => {
            if let Err(e) = entry.set_password(value) {
                tracing::warn!(%e, "{what} write failed; token cached in memory");
            }
        }
        Err(e) => tracing::warn!(%e, "{what} write failed; token cached in memory"),
    }
}

pub fn store_tokens(access: &str, refresh: Option<&str>) -> Result<(), String> {
    cache_put(Some(access), refresh);
    keyring_set(KEYRING_ACCESS, access, "keyring access");
    if let Some(s) = refresh {
        keyring_set(KEYRING_SESSION, s, "keyring refresh");
    }
    Ok(())
}

pub fn clear_tokens() -> Result<(), String> {
    cache_clear();
    let _ = Entry::new(KEYRING_SERVICE, KEYRING_ACCESS)
        .ok()
        .and_then(|e| e.delete_credential().ok());
    let _ = Entry::new(KEYRING_SERVICE, KEYRING_SESSION)
        .ok()
        .and_then(|e| e.delete_credential().ok());
    Ok(())
}

pub fn get_access_token(cfg: &AppConfig) -> Option<String> {
    if let Some(dev) = &cfg.dev_access_token {
        if !dev.is_empty() {
            return Some(dev.clone());
        }
    }
    let mut cache = token_cache().lock().ok()?;
    hydrate_keyring_once(&mut cache);
    cache.access.clone()
}

pub fn get_refresh_token() -> Option<String> {
    let mut cache = token_cache().lock().ok()?;
    hydrate_keyring_once(&mut cache);
    cache.refresh.clone()
}

pub fn auth_state(cfg: &AppConfig) -> Option<AuthState> {
    let base = cfg.base_url.clone()?;
    let webhook = cfg.webhook_url()?;
    Some(AuthState {
        base_url: base,
        webhook_url: webhook,
        has_access_token: get_access_token(cfg).is_some(),
        has_session_token: get_refresh_token().is_some(),
    })
}

/// Probe `{base}/api/health` and fill api_root / web_origin / service on cfg.
pub async fn detect_and_apply(cfg: &mut AppConfig) -> Result<String, String> {
    let base = cfg
        .base_url
        .as_ref()
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "URL is empty".to_string())?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .map_err(|e| e.to_string())?;
    let health_url = format!("{base}/api/health");
    let res = client
        .get(&health_url)
        .send()
        .await
        .map_err(|e| format!("unreachable: {e}"))?;
    if !res.status().is_success() {
        return Err(format!("health HTTP {}", res.status()));
    }
    let body: HealthBody = res
        .json()
        .await
        .map_err(|e| format!("health JSON: {e}"))?;
    if !body.ok {
        return Err("health ok=false".into());
    }

    match body.service.as_str() {
        "fe" => {
            cfg.service = Some(DetectedService::Fe);
            cfg.api_root = Some(format!("{base}/api"));
            cfg.web_origin = Some(base.clone());
            cfg.base_url = Some(base);
            Ok("fe".into())
        }
        "be" => {
            let web = body
                .web_origin
                .as_ref()
                .map(|s| s.trim().trim_end_matches('/').to_string())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "BE health missing webOrigin".to_string())?;
            cfg.service = Some(DetectedService::Be);
            cfg.api_root = Some(base.clone());
            cfg.web_origin = Some(web);
            cfg.base_url = Some(base);
            Ok("be".into())
        }
        other => Err(format!("unknown service `{other}`")),
    }
}

pub async fn exchange_authorization_code(
    cfg: &AppConfig,
    attempt: &LoginAttempt,
    code: &str,
    state: &str,
) -> Result<(), String> {
    if state != attempt.state {
        return Err("state mismatch".into());
    }
    let token_url = cfg.token_url().ok_or("api root not configured")?;
    let client = http_client();
    let res = client
        .post(&token_url)
        .json(&serde_json::json!({
            "grant_type": "authorization_code",
            "code": code,
            "redirect_uri": REDIRECT_URI,
            "client_id": "qmonitor",
            "code_verifier": attempt.verifier,
            "device_id": attempt.device_id,
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        let status = res.status();
        let text = res.text().await.unwrap_or_default();
        return Err(format!("token HTTP {status}: {text}"));
    }
    let body: TokenResponse = res.json().await.map_err(|e| e.to_string())?;
    store_tokens(&body.access_token, body.refresh_token.as_deref())?;
    Ok(())
}

pub async fn refresh_access_token(cfg: &AppConfig) -> Result<String, String> {
    let _guard = refresh_lock().lock().await;
    let refresh = get_refresh_token().ok_or("no refresh token")?;
    let device_id = device::device_id()?;
    let token_url = cfg.token_url().ok_or("api root not configured")?;
    let client = http_client();
    let res = client
        .post(&token_url)
        .json(&serde_json::json!({
            "grant_type": "refresh_token",
            "refresh_token": refresh,
            "device_id": device_id,
            "client_id": "qmonitor",
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!("refresh HTTP {}", res.status()));
    }
    let body: TokenResponse = res.json().await.map_err(|e| e.to_string())?;
    store_tokens(&body.access_token, None)?;
    Ok(body.access_token)
}

pub async fn revoke_remote(cfg: &AppConfig) -> Result<(), String> {
    let Some(revoke_url) = cfg.revoke_url() else {
        return Ok(());
    };
    let device_id = device::device_id().ok();
    let token = get_refresh_token().or_else(|| get_access_token(cfg));
    let Some(token) = token else {
        return Ok(());
    };
    let client = http_client();
    let mut body = serde_json::json!({
        "token": token,
        "client_id": "qmonitor",
    });
    if let Some(d) = device_id {
        body["device_id"] = serde_json::json!(d);
        body["token_type_hint"] = serde_json::json!("refresh_token");
    }
    let _ = client.post(&revoke_url).json(&body).send().await;
    Ok(())
}

/// Parse `code` (+ optional `state`) from a callback URL.
pub fn parse_callback_code(url: &str) -> Result<(String, String), String> {
    let parsed = url::Url::parse(url).map_err(|e| e.to_string())?;
    let mut code = None;
    let mut state = None;
    for (k, v) in parsed.query_pairs() {
        match k.as_ref() {
            "code" => code = Some(v.to_string()),
            "state" => state = Some(v.to_string()),
            "error" => return Err(format!("oauth error: {v}")),
            _ => {}
        }
    }
    let code = code.ok_or_else(|| "code missing in callback".to_string())?;
    let state = state.ok_or_else(|| "state missing in callback".to_string())?;
    Ok((code, state))
}

pub fn begin_login_attempt() -> Result<(LoginAttempt, String, String), String> {
    let device_id = device::device_id()?;
    let state = uuid::Uuid::new_v4().to_string();
    let verifier = pkce::generate_verifier();
    let challenge = pkce::challenge_s256(&verifier);
    Ok((
        LoginAttempt {
            state: state.clone(),
            verifier,
            device_id: device_id.clone(),
        },
        challenge,
        device_id,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_code_callback() {
        let (c, s) = parse_callback_code(
            "http://127.0.0.1:58473/callback?code=abc&state=xyz",
        )
        .unwrap();
        assert_eq!(c, "abc");
        assert_eq!(s, "xyz");
    }

    #[test]
    fn cache_survives_without_keyring_and_clears() {
        cache_put(Some("access-1"), Some("refresh-1"));
        let cfg = AppConfig::default();
        assert_eq!(get_access_token(&cfg).as_deref(), Some("access-1"));
        assert_eq!(get_refresh_token().as_deref(), Some("refresh-1"));
        cache_clear();
        // Hydrated empty cache: no keyring token in CI.
        let mut c = token_cache().lock().unwrap();
        c.keyring_hydrated = true;
        c.access = None;
        c.refresh = None;
        drop(c);
        assert!(get_access_token(&cfg).is_none());
    }
}
