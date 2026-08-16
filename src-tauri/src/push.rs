//! Webhook push client for completed sessions.

use std::time::Duration;

use serde::Serialize;

use crate::auth;
use crate::config::AppConfig;
use crate::db::SessionRow;

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
pub const POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Serialize)]
pub struct SessionPayload {
    pub schema_version: u32,
    pub session_id: String,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steam_app_id: Option<u32>,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exe: Option<String>,
    pub started_at: String,
    pub ended_at: String,
    pub duration_secs: i64,
    pub host: HostInfo,
}

#[derive(Debug, Serialize)]
pub struct HostInfo {
    pub os: String,
    pub hostname: String,
}

impl SessionPayload {
    pub fn from_row(row: &SessionRow) -> Option<Self> {
        Some(Self {
            schema_version: 1,
            session_id: row.id.clone(),
            source: row.source.clone(),
            steam_app_id: row.steam_app_id,
            title: row.title.clone(),
            exe: row.exe.clone(),
            started_at: row.started_at.to_rfc3339(),
            ended_at: row.ended_at?.to_rfc3339(),
            duration_secs: row.duration_secs.unwrap_or(0),
            host: HostInfo {
                os: std::env::consts::OS.to_string(),
                hostname: hostname::get()
                    .ok()
                    .and_then(|h| h.into_string().ok())
                    .unwrap_or_else(|| "unknown".into()),
            },
        })
    }
}

pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .pool_idle_timeout(POOL_IDLE_TIMEOUT)
        .build()
        .expect("reqwest client")
}

pub struct WebhookClient {
    http: reqwest::Client,
}

impl Default for WebhookClient {
    fn default() -> Self {
        Self::new()
    }
}

impl WebhookClient {
    pub fn new() -> Self {
        Self { http: http_client() }
    }

    pub async fn push(
        &self,
        cfg: &AppConfig,
        webhook_url: &str,
        access_token: &str,
        row: &SessionRow,
    ) -> Result<(), String> {
        let payload = SessionPayload::from_row(row).ok_or("session not ended")?;
        let res = self
            .http
            .post(webhook_url)
            .header("Authorization", format!("Bearer {access_token}"))
            .json(&payload)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if res.status().is_success() {
            return Ok(());
        }

        if res.status().as_u16() == 401 {
            let fresh = auth::refresh_access_token(cfg).await?;
            let retry = self
                .http
                .post(webhook_url)
                .header("Authorization", format!("Bearer {fresh}"))
                .json(&payload)
                .send()
                .await
                .map_err(|e| e.to_string())?;
            if retry.status().is_success() {
                return Ok(());
            }
            return Err(format!("webhook HTTP {}", retry.status()));
        }

        Err(format!("webhook HTTP {}", res.status()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{PushStatus, SessionRow};
    use chrono::Utc;

    #[test]
    fn payload_from_ended_row() {
        let row = SessionRow {
            id: "abc".into(),
            identity_id: "steam:570".into(),
            title: "Dota 2".into(),
            steam_app_id: Some(570),
            exe: Some("dota2.exe".into()),
            source: "steam".into(),
            started_at: Utc::now(),
            ended_at: Some(Utc::now()),
            duration_secs: Some(120),
            push_status: PushStatus::Pending,
            acked_at: None,
            retry_count: 0,
            next_retry_at: None,
            last_error: None,
        };
        let p = SessionPayload::from_row(&row).unwrap();
        assert_eq!(p.schema_version, 1);
        assert_eq!(p.steam_app_id, Some(570));
        assert_eq!(p.duration_secs, 120);
    }

    #[test]
    fn http_client_builds_with_timeouts() {
        assert_eq!(CONNECT_TIMEOUT, Duration::from_secs(8));
        assert_eq!(REQUEST_TIMEOUT, Duration::from_secs(15));
        assert_eq!(POOL_IDLE_TIMEOUT, Duration::from_secs(60));
        let _ = http_client();
        let _ = WebhookClient::new();
    }
}
