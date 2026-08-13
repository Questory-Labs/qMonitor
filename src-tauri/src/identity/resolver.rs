//! Pluggable identity resolution: Steam → catalog → Discord detectable → user mappings → (later CrowdApi).

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::detect::platform;

use super::catalog::LocalCatalog;
use super::deny::is_denied;
use super::detectable::DetectableCatalog;
use super::fingerprint::fingerprint_process;
use super::steam_library::SteamLibraryIndex;
use super::{Confidence, GameIdentity, ManualGame, PendingDetection, ProcessSnapshot};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserMapping {
    pub fingerprint: String,
    pub title: String,
    pub identity_id: String,
}

pub trait CrowdResolver: Send + Sync {
    fn resolve(&self, fingerprint: &str) -> Option<GameIdentity>;
}

/// Placeholder for future qIdentity crowdsource API.
#[derive(Default)]
pub struct NullCrowdResolver;

impl CrowdResolver for NullCrowdResolver {
    fn resolve(&self, _fingerprint: &str) -> Option<GameIdentity> {
        None
    }
}

pub struct IdentityPipeline {
    pub steam: SteamLibraryIndex,
    pub catalog: LocalCatalog,
    pub detectable: DetectableCatalog,
    pub user_mappings: HashMap<String, UserMapping>,
    pub manual_games: Vec<ManualGame>,
    pub ignored_identities: HashSet<String>,
    pub crowd: Box<dyn CrowdResolver>,
}

impl IdentityPipeline {
    pub fn new(
        steam_override: Option<&Path>,
        catalog_path: Option<&Path>,
        user_mappings: HashMap<String, UserMapping>,
    ) -> Self {
        let catalog = catalog_path
            .map(LocalCatalog::load_from_path)
            .unwrap_or_default();
        Self {
            steam: SteamLibraryIndex::load(steam_override),
            catalog,
            detectable: DetectableCatalog::load_from_disk(),
            user_mappings,
            manual_games: Vec::new(),
            ignored_identities: HashSet::new(),
            crowd: Box::new(NullCrowdResolver),
        }
    }

    pub fn resolve_running(
        &self,
        processes: &[ProcessSnapshot],
    ) -> (Vec<GameIdentity>, Vec<PendingDetection>) {
        let mut identities = platform::detect_steam(processes, &self.steam);
        let mut pending: Vec<PendingDetection> = Vec::new();

        for proc in processes {
            if is_denied(&proc.name) {
                continue;
            }

            let fp = fingerprint_process(proc);

            // Manual / user mappings are ground truth — do not sidecar-skip them.
            if let Some(mapping) = self.user_mappings.get(&fp) {
                push_unique(
                    &mut identities,
                    GameIdentity {
                        id: mapping.identity_id.clone(),
                        title: mapping.title.clone(),
                        steam_app_id: None,
                        exe: Some(proc.name.clone()),
                        confidence: Confidence::Medium,
                        source: "user".into(),
                        fingerprint: Some(fp.clone()),
                    },
                );
                continue;
            }

            if let Some(id) = match_manual_game(&self.manual_games, proc) {
                push_unique(&mut identities, id);
                continue;
            }

            if let Some(id) = self.crowd.resolve(&fp) {
                push_unique(&mut identities, id);
                continue;
            }

            // Steam path helpers are not catalog/Discord games.
            if platform::windows::is_install_sidecar(proc)
                || platform::linux::is_proton_wrapper(&proc.name)
            {
                continue;
            }

            if let Some(id) = self.catalog.match_process(proc) {
                if id.confidence.allows_auto_track() {
                    push_unique(&mut identities, id);
                } else {
                    pending.push(PendingDetection {
                        process_name: proc.name.clone(),
                        exe_path: proc.exe_path.clone(),
                        fingerprint: fp,
                        suggested_title: id.title.clone(),
                        identity_id: Some(id.id),
                    });
                }
                continue;
            }

            if let Some(id) = self.detectable.match_process(proc) {
                if id.confidence.allows_auto_track() {
                    absorb_discord(&mut identities, id, proc);
                } else {
                    pending.push(PendingDetection {
                        process_name: proc.name.clone(),
                        exe_path: proc.exe_path.clone(),
                        fingerprint: fp,
                        suggested_title: id.title.clone(),
                        identity_id: Some(id.id),
                    });
                }
            }
        }

        identities.retain(|i| {
            i.confidence.allows_auto_track() && !self.ignored_identities.contains(&i.id)
        });
        pending.retain(|p| {
            p.identity_id
                .as_ref()
                .map(|id| !self.ignored_identities.contains(id))
                .unwrap_or(true)
        });
        (identities, pending)
    }
}

/// Promote a Low steam-path hit when Discord uniquely matches the same Steam SKU
/// (or the same process when Discord has no SKU). Never promote a coincidental
/// install-dir hit to a different Discord game.
fn absorb_discord(
    identities: &mut Vec<GameIdentity>,
    discord: GameIdentity,
    proc: &ProcessSnapshot,
) {
    if let Some(sid) = discord.steam_app_id {
        if let Some(existing) = identities.iter_mut().find(|i| i.steam_app_id == Some(sid)) {
            if existing.confidence == Confidence::Low {
                existing.confidence = Confidence::Medium;
            }
            return;
        }
        push_unique(identities, discord);
        return;
    }
    if let Some(existing) = identities.iter_mut().find(|i| {
        i.source == "steam-path"
            && i.exe
                .as_ref()
                .map(|e| e.eq_ignore_ascii_case(&proc.name))
                .unwrap_or(false)
    }) {
        existing.confidence = Confidence::Medium;
        return;
    }
    push_unique(identities, discord);
}

fn push_unique(list: &mut Vec<GameIdentity>, id: GameIdentity) {
    if !list.iter().any(|e| e.id == id.id) {
        list.push(id);
    }
}

fn match_manual_game(games: &[ManualGame], proc: &ProcessSnapshot) -> Option<GameIdentity> {
    let pname = proc.name.to_ascii_lowercase();
    let path_n = proc
        .exe_path
        .as_deref()
        .unwrap_or("")
        .replace('\\', "/")
        .to_ascii_lowercase();

    for g in games {
        if !exe_name_matches(&g.exe_name, &pname) {
            continue;
        }
        if let Some(hint) = &g.path_hint {
            let hint_n = hint.replace('\\', "/").to_ascii_lowercase();
            if !path_n.is_empty() && !path_n.contains(&hint_n) {
                continue;
            }
            // Bare process name with no path: still allow if no conflicting path required tightly
            if path_n.is_empty() && !hint_n.is_empty() {
                continue;
            }
        }
        let identity_id = g
            .steam_app_id
            .map(|id| format!("steam:{id}"))
            .unwrap_or_else(|| format!("manual:{}", g.id));
        return Some(GameIdentity {
            id: identity_id,
            title: g.title.clone(),
            steam_app_id: g.steam_app_id,
            exe: Some(proc.name.clone()),
            confidence: Confidence::Medium,
            source: "manual".into(),
            fingerprint: None,
        });
    }
    None
}

fn exe_name_matches(configured: &str, proc_name: &str) -> bool {
    let cfg = configured.trim().to_ascii_lowercase();
    let cfg = cfg.trim_end_matches(".exe");
    let proc = proc_name.trim_end_matches(".exe");
    cfg == proc || configured.eq_ignore_ascii_case(proc_name)
}

/// Parse user-entered exe / path into basename + optional path hint (parent dir).
///
/// Splits on both `/` and `\` so Windows-style paths parse correctly on Unix CI hosts
/// (and vice versa for forward-slash paths on Windows).
pub fn parse_exe_input(raw: &str) -> Result<(String, Option<String>), String> {
    let trimmed = raw.trim().trim_matches('"').trim();
    if trimmed.is_empty() {
        return Err("Exe / path is required".into());
    }
    let trimmed = trimmed.trim_end_matches(['\\', '/']);
    if trimmed.is_empty() {
        return Err("Could not parse exe name".into());
    }
    let (parent, exe_name) = match trimmed.rsplit_once(['\\', '/']) {
        Some((parent, name)) if !name.is_empty() => (Some(parent), name),
        _ => (None, trimmed),
    };
    if exe_name.is_empty() {
        return Err("Could not parse exe name".into());
    }
    let path_hint = parent.and_then(|p| {
        let s = p.trim().trim_end_matches(['\\', '/']);
        if s.is_empty() || s == "." {
            None
        } else {
            // Prefer last folder segment as a stable hint (e.g. "Hades" from D:\Games\Hades).
            s.rsplit_once(['\\', '/'])
                .map(|(_, last)| last.to_string())
                .or_else(|| Some(s.to_string()))
        }
    });
    Ok((exe_name.to_string(), path_hint))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn empty_pipeline() -> IdentityPipeline {
        IdentityPipeline {
            steam: SteamLibraryIndex::default(),
            catalog: LocalCatalog::default(),
            detectable: DetectableCatalog::default(),
            user_mappings: HashMap::new(),
            manual_games: Vec::new(),
            ignored_identities: HashSet::new(),
            crowd: Box::new(NullCrowdResolver),
        }
    }

    #[test]
    fn steam_reaper_wins() {
        let mut steam = SteamLibraryIndex::default();
        steam.games.insert(
            570,
            super::super::steam_library::SteamGame {
                app_id: 570,
                title: "Dota 2".into(),
                install_path: std::path::PathBuf::from("/games/dota"),
            },
        );
        let pipeline = IdentityPipeline {
            steam,
            catalog: LocalCatalog::default(),
            detectable: DetectableCatalog::default(),
            user_mappings: HashMap::new(),
            manual_games: Vec::new(),
            ignored_identities: HashSet::new(),
            crowd: Box::new(NullCrowdResolver),
        };
        let procs = vec![ProcessSnapshot {
            pid: 9,
            name: "reaper".into(),
            exe_path: None,
            cmdline: Some("reaper SteamLaunch AppId=570 --".into()),
        }];
        let (ids, _) = pipeline.resolve_running(&procs);
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0].steam_app_id, Some(570));
        assert_eq!(ids[0].confidence, Confidence::High);
    }

    #[test]
    fn unknown_exe_never_pending() {
        let pipeline = empty_pipeline();
        let procs = vec![
            ProcessSnapshot {
                pid: 1,
                name: "msedgewebview2.exe".into(),
                exe_path: Some(r"C:\Program Files\msedgewebview2.exe".into()),
                cmdline: None,
            },
            ProcessSnapshot {
                pid: 2,
                name: "NVIDIA Broadcast.exe".into(),
                exe_path: Some(r"C:\Program Files\NVIDIA Broadcast.exe".into()),
                cmdline: None,
            },
            ProcessSnapshot {
                pid: 3,
                name: "zen.exe".into(),
                exe_path: Some(r"C:\Users\me\zen.exe".into()),
                cmdline: None,
            },
        ];
        let (ids, pending) = pipeline.resolve_running(&procs);
        assert!(ids.is_empty());
        assert!(pending.is_empty());
    }

    #[test]
    fn discord_low_goes_pending() {
        let raw = r#"[{
            "id": "555",
            "name": "Shared A",
            "executables": [{"os": "win32", "name": "shared.exe", "is_launcher": false},
                            {"os": "linux", "name": "shared", "is_launcher": false}],
            "third_party_skus": []
        },{
            "id": "666",
            "name": "Shared B",
            "executables": [{"os": "win32", "name": "shared.exe", "is_launcher": false},
                            {"os": "linux", "name": "shared", "is_launcher": false}],
            "third_party_skus": []
        }]"#;
        let detectable = DetectableCatalog::from_json(raw).unwrap();
        let pipeline = IdentityPipeline {
            detectable,
            ..empty_pipeline()
        };
        let name = if cfg!(target_os = "windows") {
            "shared.exe"
        } else if cfg!(target_os = "linux") {
            "shared"
        } else {
            return;
        };
        let procs = vec![ProcessSnapshot {
            pid: 1,
            name: name.into(),
            exe_path: Some(format!("/games/{name}")),
            cmdline: None,
        }];
        let (ids, pending) = pipeline.resolve_running(&procs);
        assert!(ids.is_empty());
        assert_eq!(pending.len(), 1);
        assert!(pending[0].identity_id.is_some());
    }

    #[test]
    fn ignored_identity_filtered() {
        let mut steam = SteamLibraryIndex::default();
        steam.games.insert(
            570,
            super::super::steam_library::SteamGame {
                app_id: 570,
                title: "Dota 2".into(),
                install_path: std::path::PathBuf::from("/games/dota"),
            },
        );
        let mut ignored = HashSet::new();
        ignored.insert("steam:570".into());
        let pipeline = IdentityPipeline {
            steam,
            catalog: LocalCatalog::default(),
            detectable: DetectableCatalog::default(),
            user_mappings: HashMap::new(),
            manual_games: Vec::new(),
            ignored_identities: ignored,
            crowd: Box::new(NullCrowdResolver),
        };
        let procs = vec![ProcessSnapshot {
            pid: 9,
            name: "reaper".into(),
            exe_path: None,
            cmdline: Some("reaper SteamLaunch AppId=570 --".into()),
        }];
        let (ids, _) = pipeline.resolve_running(&procs);
        assert!(ids.is_empty());
    }

    #[test]
    fn manual_game_matches_path() {
        let (exe_name, path_hint) =
            parse_exe_input(r"D:\Games\Hades\Hades.exe").unwrap();
        assert_eq!(exe_name, "Hades.exe");
        assert_eq!(path_hint.as_deref(), Some("Hades"));

        let mut pipeline = empty_pipeline();
        pipeline.manual_games.push(ManualGame {
            id: "abc".into(),
            title: "Hades".into(),
            exe_name,
            path_hint,
            steam_app_id: Some(1145360),
        });
        let procs = vec![ProcessSnapshot {
            pid: 1,
            name: "Hades.exe".into(),
            exe_path: Some(r"D:\Games\Hades\Hades.exe".into()),
            cmdline: None,
        }];
        let (ids, pending) = pipeline.resolve_running(&procs);
        assert!(pending.is_empty());
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0].title, "Hades");
        assert_eq!(ids[0].id, "steam:1145360");
        assert_eq!(ids[0].source, "manual");
    }

    #[test]
    fn parse_bare_exe() {
        let (name, hint) = parse_exe_input("game.exe").unwrap();
        assert_eq!(name, "game.exe");
        assert!(hint.is_none());
    }

    #[test]
    fn parse_unix_style_path() {
        let (name, hint) = parse_exe_input("/opt/games/Hades/Hades.exe").unwrap();
        assert_eq!(name, "Hades.exe");
        assert_eq!(hint.as_deref(), Some("Hades"));
    }
}
