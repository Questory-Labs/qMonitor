//! Steam reaper helpers (platform-agnostic parsing lives in identity::steam_library).

use crate::identity::deny::is_denied;
use crate::identity::steam_library::{cmdline_has_app_id, parse_reaper_app_ids, SteamLibraryIndex};
use crate::identity::{GameIdentity, ProcessSnapshot};

#[cfg(test)]
pub fn running_steam_app_ids(processes: &[ProcessSnapshot]) -> Vec<u32> {
    let mut ids = Vec::new();
    for proc in processes {
        let Some(cmd) = &proc.cmdline else {
            continue;
        };
        if !cmd.contains("SteamLaunch AppId=") && !cmd.contains("reaper") {
            if !cmd.contains("AppId=") {
                continue;
            }
        }
        for id in parse_reaper_app_ids(cmd) {
            if cmdline_has_app_id(cmd, id) && !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    ids
}

/// High-confidence Steam identities from `SteamLaunch AppId=` command lines.
pub fn steamlaunch_identities(
    processes: &[ProcessSnapshot],
    steam: &SteamLibraryIndex,
) -> Vec<GameIdentity> {
    let mut identities = Vec::new();
    for proc in processes {
        if is_denied(&proc.name) {
            continue;
        }
        let Some(cmd) = &proc.cmdline else {
            continue;
        };
        if !cmd.contains("SteamLaunch AppId=") {
            continue;
        }
        for app_id in parse_reaper_app_ids(cmd) {
            if !cmdline_has_app_id(cmd, app_id) {
                continue;
            }
            if let Some(mut id) = steam.resolve_app_id(app_id) {
                id.exe = Some(proc.name.clone());
                if !identities.iter().any(|e: &GameIdentity| e.id == id.id) {
                    identities.push(id);
                }
            }
        }
    }
    identities
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_reaper_ids() {
        let procs = vec![
            ProcessSnapshot {
                pid: 1,
                name: "reaper".into(),
                exe_path: None,
                cmdline: Some("reaper SteamLaunch AppId=730 --".into()),
            },
            ProcessSnapshot {
                pid: 2,
                name: "reaper".into(),
                exe_path: None,
                cmdline: Some("reaper SteamLaunch AppId=440 --".into()),
            },
        ];
        let mut ids = running_steam_app_ids(&procs);
        ids.sort();
        assert_eq!(ids, vec![440, 730]);
    }

    #[test]
    fn steamlaunch_identities_are_high() {
        let mut steam = crate::identity::steam_library::SteamLibraryIndex::default();
        steam.games.insert(
            570,
            crate::identity::steam_library::SteamGame {
                app_id: 570,
                title: "Dota 2".into(),
                install_path: std::path::PathBuf::from("/games/dota"),
            },
        );
        let procs = vec![ProcessSnapshot {
            pid: 9,
            name: "reaper".into(),
            exe_path: None,
            cmdline: Some("reaper SteamLaunch AppId=570 --".into()),
        }];
        let ids = steamlaunch_identities(&procs, &steam);
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0].steam_app_id, Some(570));
        assert_eq!(ids[0].confidence, crate::identity::Confidence::High);
    }
}
