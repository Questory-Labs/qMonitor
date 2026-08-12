//! Discord detectable applications catalog: download, cache, and match.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::config::AppConfig;

use super::{Confidence, GameIdentity, ProcessSnapshot};

pub const DETECTABLE_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DetectableMeta {
    url: String,
    /// Unix seconds when the cache was written.
    fetched_at: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct DetectableSku {
    distributor: Option<String>,
    id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct DetectableExecutable {
    os: String,
    name: String,
    #[serde(default)]
    is_launcher: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct DetectableEntryRaw {
    id: String,
    name: String,
    #[serde(default)]
    executables: Vec<DetectableExecutable>,
    #[serde(default)]
    third_party_skus: Vec<DetectableSku>,
}

#[derive(Debug, Clone)]
struct IndexedPattern {
    /// Normalized path pattern (`\` → `/`, lowercase).
    pattern: String,
    /// Basename of the pattern (last path segment).
    basename: String,
    entry_idx: usize,
}

#[derive(Debug, Clone)]
struct IndexedEntry {
    id: String,
    title: String,
    steam_app_id: Option<u32>,
}

/// In-memory Discord detectable catalog indexed by exe basename.
#[derive(Debug, Clone, Default)]
pub struct DetectableCatalog {
    entries: Vec<IndexedEntry>,
    /// basename → patterns that use it
    by_basename: HashMap<String, Vec<IndexedPattern>>,
}

impl DetectableCatalog {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn load_from_disk() -> Self {
        let path = cache_json_path();
        match fs::read_to_string(&path) {
            Ok(raw) => Self::from_json(&raw).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn from_json(raw: &str) -> Result<Self, String> {
        let parsed: Vec<DetectableEntryRaw> =
            serde_json::from_str(raw).map_err(|e| e.to_string())?;
        Ok(Self::from_entries(parsed))
    }

    fn from_entries(raw: Vec<DetectableEntryRaw>) -> Self {
        let os = current_discord_os();
        let mut entries = Vec::with_capacity(raw.len());
        let mut by_basename: HashMap<String, Vec<IndexedPattern>> = HashMap::new();

        for item in raw {
            let steam_app_id = item
                .third_party_skus
                .iter()
                .find(|s| {
                    s.distributor
                        .as_deref()
                        .is_some_and(|d| d.eq_ignore_ascii_case("steam"))
                })
                .and_then(|s| s.id.as_ref())
                .and_then(|id| id.parse::<u32>().ok());

            let entry_idx = entries.len();
            entries.push(IndexedEntry {
                id: item.id,
                title: item.name,
                steam_app_id,
            });

            for exe in item.executables {
                if exe.is_launcher {
                    continue;
                }
                if exe.os != os {
                    continue;
                }
                let pattern = normalize_path(&exe.name);
                if pattern.is_empty() {
                    continue;
                }
                let basename = pattern_basename(&pattern);
                by_basename
                    .entry(basename.clone())
                    .or_default()
                    .push(IndexedPattern {
                        pattern,
                        basename,
                        entry_idx,
                    });
            }
        }

        Self {
            entries,
            by_basename,
        }
    }

    pub fn match_process(&self, proc: &ProcessSnapshot) -> Option<GameIdentity> {
        let pname = proc.name.to_ascii_lowercase();
        let path_n = proc
            .exe_path
            .as_deref()
            .map(normalize_path)
            .unwrap_or_default();

        let candidates = self.by_basename.get(&pname)?;
        let mut matched_idxs: Vec<usize> = Vec::new();

        for pat in candidates {
            if pattern_matches(pat, &pname, &path_n) && !matched_idxs.contains(&pat.entry_idx) {
                matched_idxs.push(pat.entry_idx);
            }
        }

        match matched_idxs.as_slice() {
            [] => None,
            [only] => {
                let entry = &self.entries[*only];
                Some(identity_from_entry(entry, proc, Confidence::Medium))
            }
            _ => {
                // Ambiguous shared exe — prefer first for suggested title, Low confidence.
                let entry = &self.entries[matched_idxs[0]];
                Some(identity_from_entry(entry, proc, Confidence::Low))
            }
        }
    }
}

fn identity_from_entry(
    entry: &IndexedEntry,
    proc: &ProcessSnapshot,
    confidence: Confidence,
) -> GameIdentity {
    GameIdentity {
        id: format!("discord:{}", entry.id),
        title: entry.title.clone(),
        steam_app_id: entry.steam_app_id,
        exe: Some(proc.name.clone()),
        confidence,
        source: "discord".into(),
        fingerprint: None,
    }
}

fn pattern_matches(pat: &IndexedPattern, proc_basename: &str, exe_path_norm: &str) -> bool {
    if pat.pattern.contains('/') {
        // Path-ish pattern: require exe_path suffix match.
        if exe_path_norm.is_empty() {
            return false;
        }
        return exe_path_norm == pat.pattern
            || exe_path_norm.ends_with(&format!("/{}", pat.pattern));
    }
    // Bare filename: basename equality is enough.
    proc_basename == pat.basename
}

fn normalize_path(s: &str) -> String {
    s.trim().replace('\\', "/").to_ascii_lowercase()
}

fn pattern_basename(pattern: &str) -> String {
    pattern
        .rsplit('/')
        .next()
        .unwrap_or(pattern)
        .to_string()
}

fn current_discord_os() -> &'static str {
    if cfg!(target_os = "windows") {
        "win32"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "other"
    }
}

fn cache_json_path() -> PathBuf {
    AppConfig::config_dir().join("detectable.json")
}

fn cache_meta_path() -> PathBuf {
    AppConfig::config_dir().join("detectable.meta.json")
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn meta_is_fresh(meta: &DetectableMeta, url: &str, max_age: Duration, now: u64) -> bool {
    if meta.url != url {
        return false;
    }
    now.saturating_sub(meta.fetched_at) < max_age.as_secs()
}

fn read_meta() -> Option<DetectableMeta> {
    let raw = fs::read_to_string(cache_meta_path()).ok()?;
    serde_json::from_str(&raw).ok()
}

fn write_cache(url: &str, body: &str) -> Result<(), String> {
    let dir = AppConfig::config_dir();
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let json_path = cache_json_path();
    let tmp_json = dir.join("detectable.json.tmp");
    fs::write(&tmp_json, body).map_err(|e| e.to_string())?;
    fs::rename(&tmp_json, &json_path).map_err(|e| e.to_string())?;

    let meta = DetectableMeta {
        url: url.to_string(),
        fetched_at: now_unix_secs(),
    };
    let meta_raw = serde_json::to_string_pretty(&meta).map_err(|e| e.to_string())?;
    let tmp_meta = dir.join("detectable.meta.json.tmp");
    fs::write(&tmp_meta, meta_raw).map_err(|e| e.to_string())?;
    fs::rename(&tmp_meta, cache_meta_path()).map_err(|e| e.to_string())?;
    Ok(())
}

/// Load catalog from disk if cache is fresh for `url`; otherwise fetch and rebuild.
/// On network failure, returns stale cache when present.
pub async fn ensure_fresh(url: &str, max_age: Duration) -> DetectableCatalog {
    let now = now_unix_secs();
    if let Some(meta) = read_meta() {
        if meta_is_fresh(&meta, url, max_age, now) {
            let catalog = DetectableCatalog::load_from_disk();
            if !catalog.is_empty() {
                return catalog;
            }
        }
    }

    match fetch_detectable(url).await {
        Ok(body) => {
            if let Err(e) = write_cache(url, &body) {
                tracing::warn!("failed to write detectable cache: {e}");
            }
            match DetectableCatalog::from_json(&body) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("failed to parse detectable JSON: {e}");
                    DetectableCatalog::load_from_disk()
                }
            }
        }
        Err(e) => {
            tracing::warn!("failed to fetch detectable catalog: {e}");
            DetectableCatalog::load_from_disk()
        }
    }
}

async fn fetch_detectable(url: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(url)
        .header("User-Agent", "qMonitor/0.1")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    resp.text().await.map_err(|e| e.to_string())
}

/// Force download regardless of age (e.g. URL changed in settings).
pub async fn force_refresh(url: &str) -> DetectableCatalog {
    ensure_fresh(url, Duration::ZERO).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win_os_entries() -> Vec<DetectableEntryRaw> {
        vec![
            DetectableEntryRaw {
                id: "111".into(),
                name: "Overwatch".into(),
                executables: vec![DetectableExecutable {
                    os: current_discord_os().into(),
                    name: "overwatch.exe".into(),
                    is_launcher: false,
                }],
                third_party_skus: vec![DetectableSku {
                    distributor: Some("steam".into()),
                    id: Some("2357570".into()),
                }],
            },
            DetectableEntryRaw {
                id: "222".into(),
                name: "PUBG".into(),
                executables: vec![DetectableExecutable {
                    os: current_discord_os().into(),
                    name: "win64/tslgame.exe".into(),
                    is_launcher: false,
                }],
                third_party_skus: vec![],
            },
            DetectableEntryRaw {
                id: "333".into(),
                name: "Launcher Only".into(),
                executables: vec![DetectableExecutable {
                    os: current_discord_os().into(),
                    name: "gamelauncher.exe".into(),
                    is_launcher: true,
                }],
                third_party_skus: vec![],
            },
            DetectableEntryRaw {
                id: "444".into(),
                name: "Wrong OS Game".into(),
                executables: vec![DetectableExecutable {
                    os: if current_discord_os() == "win32" {
                        "linux".into()
                    } else {
                        "win32".into()
                    },
                    name: "wrongos.exe".into(),
                    is_launcher: false,
                }],
                third_party_skus: vec![],
            },
            DetectableEntryRaw {
                id: "555".into(),
                name: "Shared A".into(),
                executables: vec![DetectableExecutable {
                    os: current_discord_os().into(),
                    name: "shared.exe".into(),
                    is_launcher: false,
                }],
                third_party_skus: vec![],
            },
            DetectableEntryRaw {
                id: "666".into(),
                name: "Shared B".into(),
                executables: vec![DetectableExecutable {
                    os: current_discord_os().into(),
                    name: "shared.exe".into(),
                    is_launcher: false,
                }],
                third_party_skus: vec![],
            },
        ]
    }

    #[test]
    fn bare_exe_unique_is_medium_with_steam_sku() {
        let catalog = DetectableCatalog::from_entries(win_os_entries());
        let proc = ProcessSnapshot {
            pid: 1,
            name: "Overwatch.exe".into(),
            exe_path: Some(r"C:\Games\Overwatch\Overwatch.exe".into()),
            cmdline: None,
        };
        let id = catalog.match_process(&proc).unwrap();
        assert_eq!(id.id, "discord:111");
        assert_eq!(id.title, "Overwatch");
        assert_eq!(id.confidence, Confidence::Medium);
        assert_eq!(id.source, "discord");
        assert_eq!(id.steam_app_id, Some(2357570));
    }

    #[test]
    fn path_pattern_requires_suffix() {
        let catalog = DetectableCatalog::from_entries(win_os_entries());
        let miss = ProcessSnapshot {
            pid: 1,
            name: "tslgame.exe".into(),
            exe_path: Some(r"C:\elsewhere\tslgame.exe".into()),
            cmdline: None,
        };
        assert!(catalog.match_process(&miss).is_none());

        let hit = ProcessSnapshot {
            pid: 2,
            name: "tslgame.exe".into(),
            exe_path: Some(r"C:\Games\PUBG\win64\tslgame.exe".into()),
            cmdline: None,
        };
        let id = catalog.match_process(&hit).unwrap();
        assert_eq!(id.id, "discord:222");
        assert_eq!(id.confidence, Confidence::Medium);
    }

    #[test]
    fn shared_basename_is_low() {
        let catalog = DetectableCatalog::from_entries(win_os_entries());
        let proc = ProcessSnapshot {
            pid: 1,
            name: "shared.exe".into(),
            exe_path: Some(r"C:\Games\shared.exe".into()),
            cmdline: None,
        };
        let id = catalog.match_process(&proc).unwrap();
        assert_eq!(id.confidence, Confidence::Low);
        assert!(id.id == "discord:555" || id.id == "discord:666");
    }

    #[test]
    fn launcher_and_wrong_os_skipped() {
        let catalog = DetectableCatalog::from_entries(win_os_entries());
        let launcher = ProcessSnapshot {
            pid: 1,
            name: "gamelauncher.exe".into(),
            exe_path: None,
            cmdline: None,
        };
        assert!(catalog.match_process(&launcher).is_none());

        let wrong = ProcessSnapshot {
            pid: 2,
            name: "wrongos.exe".into(),
            exe_path: None,
            cmdline: None,
        };
        assert!(catalog.match_process(&wrong).is_none());
    }

    #[test]
    fn meta_freshness() {
        let meta = DetectableMeta {
            url: "https://example.com/d".into(),
            fetched_at: 1_000,
        };
        assert!(meta_is_fresh(
            &meta,
            "https://example.com/d",
            Duration::from_secs(100),
            1_050
        ));
        assert!(!meta_is_fresh(
            &meta,
            "https://example.com/d",
            Duration::from_secs(100),
            1_200
        ));
        assert!(!meta_is_fresh(
            &meta,
            "https://other.example/d",
            Duration::from_secs(100),
            1_050
        ));
    }

    #[test]
    fn from_json_parses_minimal() {
        let raw = r#"[{
            "id": "99",
            "name": "Test Game",
            "executables": [{"os": "win32", "name": "testgame.exe", "is_launcher": false},
                            {"os": "linux", "name": "testgame", "is_launcher": false}],
            "third_party_skus": []
        }]"#;
        let catalog = DetectableCatalog::from_json(raw).unwrap();
        assert_eq!(catalog.len(), 1);
        let name = if cfg!(target_os = "windows") {
            "testgame.exe"
        } else if cfg!(target_os = "linux") {
            "testgame"
        } else {
            return;
        };
        let proc = ProcessSnapshot {
            pid: 1,
            name: name.into(),
            exe_path: None,
            cmdline: None,
        };
        let id = catalog.match_process(&proc).unwrap();
        assert_eq!(id.title, "Test Game");
    }
}
