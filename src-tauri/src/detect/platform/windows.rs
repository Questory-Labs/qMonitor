//! Windows Steam detection: sidecar path segments skipped; SteamLaunch High only if cmdline present.

use crate::detect::steam_reaper::steamlaunch_identities;
use crate::identity::deny::is_denied;
use crate::identity::steam_library::SteamLibraryIndex;
use crate::identity::{GameIdentity, ProcessSnapshot};

pub fn is_install_sidecar(proc: &ProcessSnapshot) -> bool {
    let name = proc.name.to_ascii_lowercase();
    let stem = name.trim_end_matches(".exe");
    if stem == "crashreportclient"
        || stem.starts_with("eaanticheat")
        || stem.starts_with("easyanticheat")
        || stem.contains("eosoverlay")
    {
        return true;
    }
    let path = proc
        .exe_path
        .as_deref()
        .unwrap_or("")
        .replace('\\', "/")
        .to_ascii_lowercase();
    const SEGMENTS: &[&str] = &[
        "/easyanticheat/",
        "/battleye/",
        "/eossdk/",
        "/eosoverlay",
    ];
    SEGMENTS.iter().any(|s| path.contains(s))
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub fn detect_steam(
    processes: &[ProcessSnapshot],
    steam: &SteamLibraryIndex,
) -> Vec<GameIdentity> {
    let mut identities = steamlaunch_identities(processes, steam);
    for proc in processes {
        if is_denied(&proc.name) || is_install_sidecar(proc) {
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

#[cfg(target_os = "windows")]
pub fn foreground_pid() -> Option<u32> {
    windows_foreground_pid()
}

#[cfg(target_os = "windows")]
fn windows_foreground_pid() -> Option<u32> {
    use std::mem::MaybeUninit;
    #[link(name = "user32")]
    extern "system" {
        fn GetForegroundWindow() -> isize;
        fn GetWindowThreadProcessId(hwnd: isize, pid: *mut u32) -> u32;
    }
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd == 0 {
            return None;
        }
        let mut pid = MaybeUninit::<u32>::uninit();
        GetWindowThreadProcessId(hwnd, pid.as_mut_ptr());
        Some(pid.assume_init())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::steam_library::SteamGame;
    use crate::identity::{Confidence, ProcessSnapshot};
    use std::path::PathBuf;

    fn apex_index() -> SteamLibraryIndex {
        let mut steam = SteamLibraryIndex::default();
        steam.games.insert(
            1172470,
            SteamGame {
                app_id: 1172470,
                title: "Apex Legends".into(),
                install_path: PathBuf::from(r"D:\Steam\steamapps\common\Apex Legends"),
            },
        );
        steam
    }

    #[test]
    fn crash_report_client_under_apex_is_not_steam() {
        let procs = vec![ProcessSnapshot {
            pid: 1,
            name: "CrashReportClient.exe".into(),
            exe_path: Some(
                r"D:\Steam\steamapps\common\Apex Legends\Engine\Binaries\Win64\CrashReportClient.exe"
                    .into(),
            ),
            cmdline: None,
        }];
        assert!(detect_steam(&procs, &apex_index()).is_empty());
    }

    #[test]
    fn r5apex_path_is_low_without_corroboration() {
        let procs = vec![ProcessSnapshot {
            pid: 2,
            name: "r5apex.exe".into(),
            exe_path: Some(r"D:\Steam\steamapps\common\Apex Legends\r5apex.exe".into()),
            cmdline: None,
        }];
        let ids = detect_steam(&procs, &apex_index());
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0].steam_app_id, Some(1172470));
        assert_eq!(ids[0].confidence, Confidence::Low);
    }

    #[test]
    fn easyanticheat_folder_is_sidecar() {
        let proc = ProcessSnapshot {
            pid: 3,
            name: "EasyAntiCheat_EOS.exe".into(),
            exe_path: Some(
                r"D:\Steam\steamapps\common\Apex Legends\EasyAntiCheat\EasyAntiCheat_EOS.exe"
                    .into(),
            ),
            cmdline: None,
        };
        assert!(is_install_sidecar(&proc));
        assert!(detect_steam(&[proc], &apex_index()).is_empty());
    }

    #[test]
    fn unreal_game_exe_in_engine_binaries_is_not_a_sidecar() {
        let mut steam = SteamLibraryIndex::default();
        steam.games.insert(
            42,
            SteamGame {
                app_id: 42,
                title: "MyGame".into(),
                install_path: PathBuf::from(r"D:\Steam\steamapps\common\MyGame"),
            },
        );
        let proc = ProcessSnapshot {
            pid: 4,
            name: "MyGame.exe".into(),
            exe_path: Some(
                r"D:\Steam\steamapps\common\MyGame\Engine\Binaries\Win64\MyGame.exe".into(),
            ),
            cmdline: None,
        };
        assert!(!is_install_sidecar(&proc));
        let ids = detect_steam(&[proc], &steam);
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0].steam_app_id, Some(42));
        assert_eq!(ids[0].confidence, Confidence::Low);
    }
}
