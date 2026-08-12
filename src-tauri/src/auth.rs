//! Device login token storage, health detect, and OAuth token calls.

use keyring::Entry;
use serde::Deserialize;

use crate::config::{
    AppConfig, DetectedService, KEYRING_ACCESS, KEYRING_SERVICE, KEYRING_SESSION,
};
use crate::device;
use crate::oauth_loopback::REDIRECT_URI;
use crate::pkce;

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

pub fn store_tokens(access: &str, refresh: Option<&str>) -> Result<(), String> {
    Entry::new(KEYRING_SERVICE, KEYRING_ACCESS)
        .map_err(|e| e.to_string())?
        .set_password(access)
        .map_err(|e| e.to_string())?;
    if let Some(s) = refresh {
        Entry::new(KEYRING_SERVICE, KEYRING_SESSION)
            .map_err(|e| e.to_string())?
            .set_password(s)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn clear_tokens() -> Result<(), String> {
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
    Entry::new(KEYRING_SERVICE, KEYRING_ACCESS)
        .ok()
        .and_then(|e| e.get_password().ok())
}

pub fn get_refresh_token() -> Option<String> {
    Entry::new(KEYRING_SERVICE, KEYRING_SESSION)
        .ok()
        .and_then(|e| e.get_password().ok())
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
    let client = reqwest::Client::new();
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
    let refresh = get_refresh_token().ok_or("no refresh token")?;
    let device_id = device::device_id()?;
    let token_url = cfg.token_url().ok_or("api root not configured")?;
    let client = reqwest::Client::new();
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
    let client = reqwest::Client::new();
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
}
