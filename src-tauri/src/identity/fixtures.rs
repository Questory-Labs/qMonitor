//! Shared ProcessSnapshot tables for Win+Linux detection tests (every runner).

use std::path::PathBuf;

use super::catalog::LocalCatalog;
use super::detectable::DetectableCatalog;
use super::resolver::{IdentityPipeline, NullCrowdResolver};
use super::steam_library::{SteamGame, SteamLibraryIndex};
use super::{ManualGame, ProcessSnapshot};

use crate::detect::platform;
use std::collections::{HashMap, HashSet};

pub fn proc(pid: u32, name: &str, exe: Option<&str>, cmd: Option<&str>) -> ProcessSnapshot {
    ProcessSnapshot {
        pid,
        name: name.into(),
        exe_path: exe.map(str::to_string),
        cmdline: cmd.map(str::to_string),
    }
}

pub fn apex_install() -> PathBuf {
    PathBuf::from(r"D:\Steam\steamapps\common\Apex Legends")
}

pub fn dota_install() -> PathBuf {
    PathBuf::from("/games/steamapps/common/dota 2 beta")
}

pub fn apex_steam() -> SteamLibraryIndex {
    let mut steam = SteamLibraryIndex::default();
    steam.games.insert(
        1172470,
        SteamGame {
            app_id: 1172470,
            title: "Apex Legends".into(),
            install_path: apex_install(),
        },
    );
    steam
}

pub fn dota_steam() -> SteamLibraryIndex {
    let mut steam = SteamLibraryIndex::default();
    steam.games.insert(
        570,
        SteamGame {
            app_id: 570,
            title: "Dota 2".into(),
            install_path: dota_install(),
        },
    );
    steam
}

pub fn portal_and_portal2_steam() -> SteamLibraryIndex {
    let mut steam = SteamLibraryIndex::default();
    steam.games.insert(
        400,
        SteamGame {
            app_id: 400,
            title: "Portal".into(),
            install_path: PathBuf::from(r"D:\Steam\steamapps\common\Portal"),
        },
    );
    steam.games.insert(
        620,
        SteamGame {
            app_id: 620,
            title: "Portal 2".into(),
            install_path: PathBuf::from(r"D:\Steam\steamapps\common\Portal 2"),
        },
    );
    steam
}

pub fn overlapping_nested_steam() -> SteamLibraryIndex {
    let mut steam = SteamLibraryIndex::default();
    steam.games.insert(
        1,
        SteamGame {
            app_id: 1,
            title: "Game".into(),
            install_path: PathBuf::from(r"D:\Steam\steamapps\common\Game"),
        },
    );
    steam.games.insert(
        2,
        SteamGame {
            app_id: 2,
            title: "Game DLC".into(),
            install_path: PathBuf::from(r"D:\Steam\steamapps\common\Game\DLC"),
        },
    );
    steam
}

/// Discord catalog includes both Win32 and Linux exe names so either host can unique-match.
pub fn apex_discord() -> DetectableCatalog {
    let raw = r#"[{
        "id": "apex-flake",
        "name": "Apex Legends",
        "executables": [
            {"os": "win32", "name": "r5apex.exe", "is_launcher": false},
            {"os": "linux", "name": "r5apex", "is_launcher": false}
        ],
        "third_party_skus": [{"distributor": "steam", "id": "1172470"}]
    }]"#;
    DetectableCatalog::from_json(raw).unwrap()
}

pub fn crash_report_client() -> ProcessSnapshot {
    proc(
        1,
        "CrashReportClient.exe",
        Some(
            r"D:\Steam\steamapps\common\Apex Legends\Engine\Binaries\Win64\CrashReportClient.exe",
        ),
        None,
    )
}

/// Host-appropriate Apex game process under the install dir (Discord OS filter).
pub fn r5apex_under_install() -> ProcessSnapshot {
    let name = if cfg!(target_os = "windows") {
        "r5apex.exe"
    } else {
        "r5apex"
    };
    proc(
        1,
        name,
        Some(&format!(
            r"D:\Steam\steamapps\common\Apex Legends\{name}"
        )),
        None,
    )
}

pub fn reaper_dota() -> ProcessSnapshot {
    proc(
        9,
        "reaper",
        None,
        Some("reaper SteamLaunch AppId=570 --"),
    )
}

pub fn wineserver_under_dota() -> ProcessSnapshot {
    proc(
        4,
        "wineserver",
        Some("/games/steamapps/common/dota 2 beta/wineserver"),
        None,
    )
}

pub fn hades_under_eac_folder() -> ProcessSnapshot {
    proc(
        1,
        "Hades.exe",
        Some(r"D:\Steam\steamapps\common\Apex Legends\EasyAntiCheat\Hades.exe"),
        None,
    )
}

pub fn portal2_exe() -> ProcessSnapshot {
    proc(
        1,
        "portal2.exe",
        Some(r"D:\Steam\steamapps\common\Portal 2\bin\portal2.exe"),
        None,
    )
}

pub fn nested_dlc_exe() -> ProcessSnapshot {
    proc(
        1,
        "game.exe",
        Some(r"D:\Steam\steamapps\common\Game\DLC\game.exe"),
        None,
    )
}

pub fn pipeline_with(
    steam: SteamLibraryIndex,
    detectable: DetectableCatalog,
    manual: Vec<ManualGame>,
) -> IdentityPipeline {
    IdentityPipeline {
        steam,
        catalog: LocalCatalog::default(),
        detectable,
        user_mappings: HashMap::new(),
        manual_games: manual,
        ignored_identities: HashSet::new(),
        crowd: Box::new(NullCrowdResolver),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Confidence;

    #[test]
    fn portal2_is_not_a_prefix_of_portal() {
        let steam = portal_and_portal2_steam();
        let id = steam.match_path(&portal2_exe()).unwrap();
        assert_eq!(id.steam_app_id, Some(620));
        assert_eq!(id.confidence, Confidence::Low);
    }

    #[test]
    fn overlapping_installs_yield_no_path_match() {
        let steam = overlapping_nested_steam();
        assert!(steam.match_path(&nested_dlc_exe()).is_none());
    }

    #[test]
    fn leftover_crash_reporter_is_not_auto_tracked() {
        let pipeline = pipeline_with(apex_steam(), apex_discord(), Vec::new());
        let (ids, pending) = pipeline.resolve_running(&[crash_report_client()]);
        assert!(ids.is_empty());
        assert!(pending.is_empty());
    }

    #[test]
    fn path_only_game_exe_is_not_auto_tracked_without_discord() {
        let pipeline = pipeline_with(apex_steam(), DetectableCatalog::default(), Vec::new());
        let (ids, pending) = pipeline.resolve_running(&[r5apex_under_install()]);
        assert!(ids.is_empty(), "path-only Low must not auto-track: {ids:?}");
        assert!(pending.is_empty());
    }

    #[test]
    fn discord_unique_promotes_steam_path_to_medium() {
        let pipeline = pipeline_with(apex_steam(), apex_discord(), Vec::new());
        let (ids, pending) = pipeline.resolve_running(&[r5apex_under_install()]);
        assert!(pending.is_empty());
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0].id, "steam:1172470");
        assert_eq!(ids[0].confidence, Confidence::Medium);
        assert_eq!(ids[0].source, "steam-path");
    }

    #[test]
    fn linux_reaper_cmdline_is_high() {
        let ids = platform::linux::detect_steam(&[reaper_dota()], &dota_steam());
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0].steam_app_id, Some(570));
        assert_eq!(ids[0].confidence, Confidence::High);
    }

    #[test]
    fn linux_wineserver_under_install_is_not_path_matched() {
        let ids = platform::linux::detect_steam(&[wineserver_under_dota()], &dota_steam());
        assert!(ids.is_empty());
    }

    #[test]
    fn wineserver_leftover_is_not_auto_tracked() {
        let pipeline = pipeline_with(dota_steam(), DetectableCatalog::default(), Vec::new());
        let (ids, pending) = pipeline.resolve_running(&[wineserver_under_dota()]);
        assert!(ids.is_empty());
        assert!(pending.is_empty());
    }

    #[test]
    fn manual_hades_bypasses_sidecar_skip() {
        let pipeline = pipeline_with(
            apex_steam(),
            DetectableCatalog::default(),
            vec![ManualGame {
                id: "abc".into(),
                title: "Hades".into(),
                exe_name: "Hades.exe".into(),
                path_hint: Some("Apex Legends".into()),
                steam_app_id: Some(1145360),
            }],
        );
        let (ids, pending) = pipeline.resolve_running(&[hades_under_eac_folder()]);
        assert!(pending.is_empty());
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0].source, "manual");
        assert_eq!(ids[0].title, "Hades");
        assert_eq!(ids[0].id, "steam:1145360");
    }
}
