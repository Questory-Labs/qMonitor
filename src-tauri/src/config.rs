//! Application configuration persisted under the OS config directory.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

pub const WEBHOOK_PATH: &str = "/webhooks/qmonitor";
pub const OAUTH_AUTHORIZE_PATH: &str = "/oauth/qmonitor/authorize";
pub const OAUTH_TOKEN_PATH: &str = "/oauth/qmonitor/token";
pub const OAUTH_REVOKE_PATH: &str = "/oauth/qmonitor/revoke";
pub const KEYRING_SERVICE: &str = "qmonitor";
pub const KEYRING_ACCESS: &str = "access_token";
pub const KEYRING_SESSION: &str = "session_token";
pub const DEFAULT_DETECTABLE_URL: &str =
    "https://discord.com/api/v10/applications/detectable";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DetectedService {
    Fe,
    Be,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum UpdateChannel {
    #[default]
    Stable,
    Canary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub base_url: Option<String>,
    /// Resolved API root for token/webhook/revoke (with `/api` when FE).
    pub api_root: Option<String>,
    /// Web origin for consent (equals base_url on FE; from health on BE).
    pub web_origin: Option<String>,
    pub service: Option<DetectedService>,
    /// Optional override for the local Turso DB file (default: config_dir/qmonitor.db).
    pub db_path: Option<String>,
    pub poll_interval_secs: u64,
    pub retention_acked_days: u32,
    pub catalog_path: Option<String>,
    /// Override for Discord detectable catalog URL (empty/null → default Discord v10).
    pub detectable_url: Option<String>,
    pub steam_path_override: Option<String>,
    pub start_at_login: bool,
    /// When true, minimizing the window hides it to the system tray.
    #[serde(default)]
    pub minimize_to_tray: bool,
    /// When true, closing the window hides it to the system tray (Quit via tray).
    #[serde(default)]
    pub close_to_tray: bool,
    /// GitHub release channel to poll for updates (stable = Latest, canary = newest prerelease).
    #[serde(default)]
    pub update_channel: UpdateChannel,
    /// Dev fallback when device login is unavailable.
    pub dev_access_token: Option<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            base_url: None,
            api_root: None,
            web_origin: None,
            service: None,
            db_path: None,
            poll_interval_secs: 3,
            retention_acked_days: 30,
            catalog_path: None,
            detectable_url: None,
            steam_path_override: None,
            start_at_login: false,
            minimize_to_tray: false,
            close_to_tray: false,
            update_channel: UpdateChannel::Stable,
            dev_access_token: None,
        }
    }
}

impl AppConfig {
    pub fn config_dir() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("qMonitor")
    }

    pub fn config_path() -> PathBuf {
        Self::config_dir().join("config.json")
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        match fs::read_to_string(&path) {
            Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let dir = Self::config_dir();
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let raw = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        fs::write(Self::config_path(), raw).map_err(|e| e.to_string())
    }

    pub fn webhook_url(&self) -> Option<String> {
        let root = self.api_root.as_ref()?.trim_end_matches('/');
        Some(format!("{root}{WEBHOOK_PATH}"))
    }

    pub fn token_url(&self) -> Option<String> {
        let root = self.api_root.as_ref()?.trim_end_matches('/');
        Some(format!("{root}{OAUTH_TOKEN_PATH}"))
    }

    pub fn revoke_url(&self) -> Option<String> {
        let root = self.api_root.as_ref()?.trim_end_matches('/');
        Some(format!("{root}{OAUTH_REVOKE_PATH}"))
    }

    pub fn authorize_url(
        &self,
        redirect_uri: &str,
        state: &str,
        code_challenge: &str,
        device_id: &str,
    ) -> Option<String> {
        let web = self.web_origin.as_ref()?.trim_end_matches('/');
        let mut url = url::Url::parse(&format!("{web}{OAUTH_AUTHORIZE_PATH}")).ok()?;
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", "qmonitor")
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("state", state)
            .append_pair("scope", "qmonitor")
            .append_pair("code_challenge", code_challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("device_id", device_id);
        Some(url.to_string())
    }

    pub fn is_onboarded(&self) -> bool {
        self.base_url.as_ref().is_some_and(|u| !u.is_empty())
            && self.api_root.as_ref().is_some_and(|u| !u.is_empty())
            && self.web_origin.as_ref().is_some_and(|u| !u.is_empty())
    }

    pub fn has_base_url(&self) -> bool {
        self.is_onboarded()
    }

    pub fn resolved_db_path(&self) -> PathBuf {
        self.db_path
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| Self::config_dir().join("qmonitor.db"))
    }

    /// Resolved Discord detectable catalog URL (override or default).
    pub fn resolved_detectable_url(&self) -> String {
        self.detectable_url
            .as_ref()
            .map(|u| u.trim().to_string())
            .filter(|u| !u.is_empty())
            .unwrap_or_else(|| DEFAULT_DETECTABLE_URL.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_update_channel_defaults_to_stable() {
        let cfg: AppConfig = serde_json::from_str(
            r#"{"pollIntervalSecs":3,"retentionAckedDays":30,"startAtLogin":false}"#,
        )
        .expect("parse");
        assert_eq!(cfg.update_channel, UpdateChannel::Stable);
        assert!(!cfg.minimize_to_tray);
        assert!(!cfg.close_to_tray);
    }

    #[test]
    fn update_channel_canary_roundtrip() {
        let mut cfg = AppConfig::default();
        cfg.update_channel = UpdateChannel::Canary;
        let raw = serde_json::to_string(&cfg).expect("ser");
        let back: AppConfig = serde_json::from_str(&raw).expect("de");
        assert_eq!(back.update_channel, UpdateChannel::Canary);
        assert!(raw.contains("\"updateChannel\":\"canary\""));
    }
}
