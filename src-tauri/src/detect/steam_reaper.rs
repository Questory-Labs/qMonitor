//! Steam reaper helpers (platform-agnostic parsing lives in identity::steam_library).

use crate::identity::steam_library::{cmdline_has_app_id, parse_reaper_app_ids};
use crate::identity::ProcessSnapshot;

pub fn running_steam_app_ids(processes: &[ProcessSnapshot]) -> Vec<u32> {
    let mut ids = Vec::new();
    for proc in processes {
        let Some(cmd) = &proc.cmdline else {
            continue;
        };
        if !cmd.contains("SteamLaunch AppId=") && !cmd.contains("reaper") {
            // Still parse if marker present without reaper name (Windows may differ)
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
}
