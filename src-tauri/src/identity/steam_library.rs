//! Local Steam library index from libraryfolders.vdf + appmanifest_*.acf.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::{Confidence, GameIdentity, ProcessSnapshot};

#[derive(Debug, Clone)]
pub struct SteamGame {
    pub app_id: u32,
    pub title: String,
    pub install_path: PathBuf,
}

#[derive(Debug, Clone, Default)]
pub struct SteamLibraryIndex {
    pub games: HashMap<u32, SteamGame>,
}

impl SteamLibraryIndex {
    pub fn load(steam_root: Option<&Path>) -> Self {
        let Some(root) = steam_root.map(PathBuf::from).or_else(detect_steam_root) else {
            return Self::default();
        };
        let mut games = HashMap::new();
        for lib in library_folders(&root) {
            let steamapps = lib.join("steamapps");
            let Ok(entries) = fs::read_dir(&steamapps) else {
                continue;
            };
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if !name.starts_with("appmanifest_") || !name.ends_with(".acf") {
                    continue;
                }
                if let Some(game) = parse_appmanifest(&entry.path(), &steamapps) {
                    games.insert(game.app_id, game);
                }
            }
        }
        Self { games }
    }

    pub fn resolve_app_id(&self, app_id: u32) -> Option<GameIdentity> {
        let game = self.games.get(&app_id);
        Some(GameIdentity {
            id: format!("steam:{app_id}"),
            title: game
                .map(|g| g.title.clone())
                .unwrap_or_else(|| format!("Steam App {app_id}")),
            steam_app_id: Some(app_id),
            exe: None,
            confidence: Confidence::High,
            source: "steam".into(),
            fingerprint: None,
        })
    }

    /// Path under a unique install dir. Uncorroborated hits are **Low** (not auto-track).
    pub fn match_path(&self, proc: &ProcessSnapshot) -> Option<GameIdentity> {
        let path = proc.exe_path.as_deref()?;
        let mut hits: Vec<&SteamGame> = self
            .games
            .values()
            .filter(|g| path_is_under_install(path, &g.install_path))
            .collect();
        if hits.len() != 1 {
            return None;
        }
        let g = hits.pop()?;
        Some(GameIdentity {
            id: format!("steam:{}", g.app_id),
            title: g.title.clone(),
            steam_app_id: Some(g.app_id),
            exe: Some(proc.name.clone()),
            confidence: Confidence::Low,
            source: "steam-path".into(),
            fingerprint: None,
        })
    }
}

/// Directory-boundary prefix: `install` or `install/...`, not `install-other`.
pub fn path_is_under_install(exe_path: &str, install_path: &Path) -> bool {
    let path = exe_path.replace('\\', "/").to_ascii_lowercase();
    let install = install_path
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();
    let install = install.trim_end_matches('/');
    if install.is_empty() {
        return false;
    }
    path == install || path.starts_with(&format!("{install}/"))
}

pub fn detect_steam_root() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let candidates = [
            PathBuf::from(r"C:\Program Files (x86)\Steam"),
            PathBuf::from(r"C:\Program Files\Steam"),
        ];
        for c in candidates {
            if c.join("steam.exe").exists() {
                return Some(c);
            }
        }
        if let Ok(home) = std::env::var("PROGRAMFILES(X86)") {
            let p = PathBuf::from(home).join("Steam");
            if p.exists() {
                return Some(p);
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(home) = dirs::home_dir() {
            for rel in [
                ".steam/steam",
                ".local/share/Steam",
                ".var/app/com.valvesoftware.Steam/data/Steam",
            ] {
                let p = home.join(rel);
                if p.exists() {
                    return Some(p);
                }
            }
        }
    }
    None
}

fn library_folders(steam_root: &Path) -> Vec<PathBuf> {
    let mut libs = vec![steam_root.to_path_buf()];
    let vdf = steam_root.join("steamapps").join("libraryfolders.vdf");
    let Ok(raw) = fs::read_to_string(vdf) else {
        return libs;
    };
    for line in raw.lines() {
        let line = line.trim();
        // "path"		"D:\\SteamLibrary"
        if let Some(rest) = line.strip_prefix("\"path\"") {
            let path = rest.trim().trim_matches('"').replace("\\\\", "\\");
            let pb = PathBuf::from(path);
            if pb.exists() && !libs.contains(&pb) {
                libs.push(pb);
            }
        }
    }
    libs
}

fn parse_appmanifest(path: &Path, steamapps: &Path) -> Option<SteamGame> {
    let raw = fs::read_to_string(path).ok()?;
    let app_id = vdf_string(&raw, "appid")?.parse().ok()?;
    let title = vdf_string(&raw, "name").unwrap_or_else(|| format!("App {app_id}"));
    let installdir = vdf_string(&raw, "installdir")?;
    let install_path = steamapps.join("common").join(installdir);
    Some(SteamGame {
        app_id,
        title,
        install_path,
    })
}

fn vdf_string(raw: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    for line in raw.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix(&needle) {
            let val = rest.trim().trim_matches('"').to_string();
            if !val.is_empty() {
                return Some(val);
            }
        }
    }
    None
}

/// Extract Steam AppIds from process command lines (`SteamLaunch AppId=440`).
pub fn parse_reaper_app_ids(cmdline: &str) -> Vec<u32> {
    let mut ids = Vec::new();
    let marker = "SteamLaunch AppId=";
    let mut search = cmdline;
    while let Some(idx) = search.find(marker) {
        let after = &search[idx + marker.len()..];
        let id_str: String = after
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if let Ok(id) = id_str.parse::<u32>() {
            // Boundary: next char must be non-digit (space/end) — already ensured by take_while.
            // Avoid substring: "440" matching inside "4400" — take_while gets full number so 4400 is fine as distinct.
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        search = &after[id_str.len().max(1)..];
    }
    ids
}

/// True if cmdline contains exact AppId with boundary (not a prefix of a longer id).
pub fn cmdline_has_app_id(cmdline: &str, app_id: u32) -> bool {
    let needle = format!("SteamLaunch AppId={app_id}");
    if let Some(idx) = cmdline.find(&needle) {
        let after = &cmdline[idx + needle.len()..];
        return after.is_empty()
            || after.starts_with(' ')
            || after.starts_with('\0')
            || after.starts_with('\t');
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reaper_app_id_boundary() {
        assert!(cmdline_has_app_id("reaper SteamLaunch AppId=440 -- game", 440));
        assert!(!cmdline_has_app_id(
            "reaper SteamLaunch AppId=4400 -- game",
            440
        ));
        assert!(cmdline_has_app_id(
            "reaper SteamLaunch AppId=4400 -- game",
            4400
        ));
        assert_eq!(
            parse_reaper_app_ids("reaper SteamLaunch AppId=570 SteamLaunch AppId=440"),
            vec![570, 440]
        );
    }

    #[test]
    fn path_under_installdir() {
        let mut index = SteamLibraryIndex::default();
        index.games.insert(
            570,
            SteamGame {
                app_id: 570,
                title: "Dota 2".into(),
                install_path: PathBuf::from(r"D:\Steam\steamapps\common\dota 2 beta"),
            },
        );
        let proc = ProcessSnapshot {
            pid: 1,
            name: "dota2.exe".into(),
            exe_path: Some(r"D:\Steam\steamapps\common\dota 2 beta\game\bin\win64\dota2.exe".into()),
            cmdline: None,
        };
        let id = index.match_path(&proc).unwrap();
        assert_eq!(id.steam_app_id, Some(570));
        assert_eq!(id.confidence, Confidence::Low);
        assert_eq!(id.source, "steam-path");
    }

    #[test]
    fn path_boundary_portal_vs_portal_2() {
        let portal = PathBuf::from(r"D:\Steam\steamapps\common\Portal");
        let portal2_exe =
            r"D:\Steam\steamapps\common\Portal 2\bin\portal2.exe";
        assert!(!path_is_under_install(portal2_exe, &portal));
        assert!(path_is_under_install(
            r"D:\Steam\steamapps\common\Portal\portal.exe",
            &portal
        ));
    }

    #[test]
    fn overlapping_installs_yield_no_path_match() {
        let mut index = SteamLibraryIndex::default();
        index.games.insert(
            1,
            SteamGame {
                app_id: 1,
                title: "Nested".into(),
                install_path: PathBuf::from(r"D:\Steam\steamapps\common\Game"),
            },
        );
        index.games.insert(
            2,
            SteamGame {
                app_id: 2,
                title: "Nested Too".into(),
                install_path: PathBuf::from(r"D:\Steam\steamapps\common\Game"),
            },
        );
        let proc = ProcessSnapshot {
            pid: 1,
            name: "game.exe".into(),
            exe_path: Some(r"D:\Steam\steamapps\common\Game\game.exe".into()),
            cmdline: None,
        };
        assert!(index.match_path(&proc).is_none());
    }
}
