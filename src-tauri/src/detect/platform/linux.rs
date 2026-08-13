//! Linux Steam detection: reaper cmdline is High; Proton wrappers never path-match.

use crate::detect::steam_reaper::steamlaunch_identities;
use crate::identity::deny::is_denied;
use crate::identity::steam_library::SteamLibraryIndex;
use crate::identity::{GameIdentity, ProcessSnapshot};

pub fn is_proton_wrapper(process_name: &str) -> bool {
    let stem = process_name
        .to_ascii_lowercase()
        .trim_end_matches(".exe")
        .to_string();
    matches!(
        stem.as_str(),
        "reaper"
            | "wineserver"
            | "wine64-preloader"
            | "wine64"
            | "wine"
            | "proton"
            | "pv-adverb"
            | "pressure-vessel-wrap"
    ) || stem.starts_with("pressure-vessel")
        || stem.starts_with("steam-runtime")
        || stem.starts_with("proton")
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn detect_steam(
    processes: &[ProcessSnapshot],
    steam: &SteamLibraryIndex,
) -> Vec<GameIdentity> {
    let mut identities = steamlaunch_identities(processes, steam);
    for proc in processes {
        if is_denied(&proc.name) || is_proton_wrapper(&proc.name) {
            continue;
        }
        if let Some(id) = steam.match_path(proc) {
            if !identities.iter().any(|e| e.id == id.id) {
                identities.push(id);
            }
        }
    }
    identities
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::steam_library::SteamGame;
    use crate::identity::{Confidence, ProcessSnapshot};
    use std::path::PathBuf;

    fn dota_index() -> SteamLibraryIndex {
        let mut steam = SteamLibraryIndex::default();
        steam.games.insert(
            570,
            SteamGame {
                app_id: 570,
                title: "Dota 2".into(),
                install_path: PathBuf::from("/games/steamapps/common/dota 2 beta"),
            },
        );
        steam
    }

    #[test]
    fn reaper_cmdline_is_high() {
        let procs = vec![ProcessSnapshot {
            pid: 9,
            name: "reaper".into(),
            exe_path: None,
            cmdline: Some("reaper SteamLaunch AppId=570 --".into()),
        }];
        let ids = detect_steam(&procs, &dota_index());
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0].steam_app_id, Some(570));
        assert_eq!(ids[0].confidence, Confidence::High);
        assert_eq!(ids[0].source, "steam");
    }

    #[test]
    fn other_app_reaper_does_not_keep_570() {
        let mut steam = dota_index();
        steam.games.insert(
            730,
            SteamGame {
                app_id: 730,
                title: "CS2".into(),
                install_path: PathBuf::from("/games/steamapps/common/Counter-Strike Global Offensive"),
            },
        );
        let procs = vec![ProcessSnapshot {
            pid: 2,
            name: "reaper".into(),
            exe_path: None,
            cmdline: Some("reaper SteamLaunch AppId=730 --".into()),
        }];
        let ids = detect_steam(&procs, &steam);
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0].steam_app_id, Some(730));
    }

    #[test]
    fn wineserver_under_install_dir_is_not_path_matched() {
        let procs = vec![ProcessSnapshot {
            pid: 4,
            name: "wineserver".into(),
            exe_path: Some("/games/steamapps/common/dota 2 beta/wineserver".into()),
            cmdline: None,
        }];
        let ids = detect_steam(&procs, &dota_index());
        assert!(ids.is_empty());
    }

    #[test]
    fn native_dota_path_is_low() {
        let procs = vec![ProcessSnapshot {
            pid: 1,
            name: "dota2".into(),
            exe_path: Some("/games/steamapps/common/dota 2 beta/game/bin/linuxsteamrt64/dota2".into()),
            cmdline: None,
        }];
        let ids = detect_steam(&procs, &dota_index());
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0].confidence, Confidence::Low);
        assert_eq!(ids[0].source, "steam-path");
    }
}
