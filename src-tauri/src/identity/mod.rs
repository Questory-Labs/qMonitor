pub mod catalog;
pub mod deny;
pub mod detectable;
pub mod fingerprint;
pub mod resolver;
pub mod steam_library;

#[cfg(test)]
pub mod fixtures;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Confidence {
    High,
    Medium,
    Low,
    None,
}

impl Confidence {
    pub fn allows_auto_track(self) -> bool {
        matches!(self, Confidence::High | Confidence::Medium)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameIdentity {
    /// `steam:<appid>`, `catalog:<id>`, `discord:<snowflake>`, or `user:<fingerprint>`
    pub id: String,
    pub title: String,
    pub steam_app_id: Option<u32>,
    pub exe: Option<String>,
    pub confidence: Confidence,
    pub source: String,
    pub fingerprint: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ProcessSnapshot {
    pub pid: u32,
    pub name: String,
    pub exe_path: Option<String>,
    pub cmdline: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingDetection {
    pub process_name: String,
    pub exe_path: Option<String>,
    pub fingerprint: String,
    pub suggested_title: String,
    /// Catalog / Discord identity when known (for Confirm / Don't track).
    pub identity_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IgnoredIdentity {
    pub identity_id: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackableGame {
    pub id: String,
    pub title: String,
    pub steam_app_id: Option<u32>,
    pub source: String,
    pub tracking_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualGame {
    pub id: String,
    pub title: String,
    pub exe_name: String,
    pub path_hint: Option<String>,
    pub steam_app_id: Option<u32>,
}
