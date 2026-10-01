//! Steam install-dir path hits: title-like exe promotion and pending leftovers.

use super::deny::is_denied;
use super::fingerprint::fingerprint_process;
use super::steam_library::{path_is_under_install, SteamLibraryIndex};
use super::{Confidence, GameIdentity, PendingDetection, ProcessSnapshot};
use crate::detect::platform;

const MIN_SLUG: usize = 4;
const MIN_AFFIX: usize = 8;

const TITLE_TAILS: &[&str] = &[
    "definitiveedition",
    "completeedition",
    "gameoftheyear",
    "ultimateedition",
    "deluxeedition",
    "goldedition",
    "standardedition",
    "remastered",
    "enhanced",
    "complete",
    "definitive",
    "ultimate",
    "deluxe",
    "goty",
];

const EXE_TAILS: &[&str] = &[
    "win64shipping",
    "linuxsteamrt64",
    "shipping",
    "win64",
    "win32",
];

const GENERIC_EXE: &[&str] = &[
    "game", "games", "launcher", "launch", "client", "app", "unity", "unreal", "editor", "server",
    "helper", "service", "crash", "setup", "update", "play", "start", "engine",
];

/// Alphanumeric slug of a library title vs process name (e.g. Crimson Desert Enhanced / CrimsonDesert.exe).
pub fn exe_looks_like_title(title: &str, process_name: &str) -> bool {
    let title_slug = strip_tails(alnum_lower(title), TITLE_TAILS);
    let basename = process_basename(process_name);
    let stem = basename.strip_suffix(".exe").unwrap_or(&basename);
    let exe_slug = strip_tails(alnum_lower(stem), EXE_TAILS);

    if title_slug.len() < MIN_SLUG || exe_slug.len() < MIN_SLUG {
        return false;
    }
    if GENERIC_EXE.contains(&exe_slug.as_str()) {
        return false;
    }
    if title_slug == exe_slug {
        return true;
    }
    (exe_slug.len() >= MIN_AFFIX && title_slug.ends_with(&exe_slug))
        || (title_slug.len() >= MIN_AFFIX && exe_slug.ends_with(&title_slug))
}

/// Process under `steamapps/common` that did not unique-match any indexed install dir.
pub fn is_unindexed_steamapps_process(proc: &ProcessSnapshot, steam: &SteamLibraryIndex) -> bool {
    if is_path_noise(proc) {
        return false;
    }
    let Some(path) = proc.exe_path.as_deref() else {
        return false;
    };
    let n = path.replace('\\', "/").to_ascii_lowercase();
    if !n.contains("/steamapps/common/") {
        return false;
    }
    steam.match_path(proc).is_none()
}

/// Unique Steam-path Low hits: title-like exe → Medium auto-track; otherwise pending (not silent drop).
pub fn finalize_steam_path_hits(
    steam: &SteamLibraryIndex,
    identities: &mut Vec<GameIdentity>,
    pending: &mut Vec<PendingDetection>,
    processes: &[ProcessSnapshot],
) {
    let mut drop_idx: Vec<usize> = Vec::new();
    for (idx, id) in identities.iter_mut().enumerate() {
        if id.source != "steam-path" || id.confidence != Confidence::Low {
            continue;
        }
        let Some(app_id) = id.steam_app_id else {
            continue;
        };
        let Some(proc) = best_process_under_app(steam, app_id, processes) else {
            continue;
        };
        if exe_looks_like_title(&id.title, &proc.name) {
            id.confidence = Confidence::Medium;
            id.exe = Some(proc.name.clone());
            continue;
        }
        pending.push(PendingDetection {
            process_name: proc.name.clone(),
            exe_path: proc.exe_path.clone(),
            fingerprint: fingerprint_process(proc),
            suggested_title: id.title.clone(),
            identity_id: Some(id.id.clone()),
        });
        drop_idx.push(idx);
    }
    for idx in drop_idx.into_iter().rev() {
        identities.remove(idx);
    }
}

fn best_process_under_app<'a>(
    steam: &SteamLibraryIndex,
    app_id: u32,
    processes: &'a [ProcessSnapshot],
) -> Option<&'a ProcessSnapshot> {
    let game = steam.games.get(&app_id)?;
    let under: Vec<&ProcessSnapshot> = processes
        .iter()
        .filter(|p| {
            !is_path_noise(p)
                && p.exe_path
                    .as_deref()
                    .is_some_and(|path| path_is_under_install(path, &game.install_path))
        })
        .collect();
    if under.is_empty() {
        return None;
    }
    if let Some(i) = under
        .iter()
        .position(|p| exe_looks_like_title(&game.title, &p.name))
    {
        return Some(under[i]);
    }
    Some(under[0])
}

fn is_path_noise(proc: &ProcessSnapshot) -> bool {
    is_denied(&proc.name)
        || platform::linux::is_proton_wrapper(&proc.name)
        || platform::windows::is_install_sidecar(proc)
}

fn process_basename(process_name: &str) -> String {
    // Backslash is a separator only on the host's Windows path grammar; on Unix it is a
    // legal filename character and must be preserved.
    #[cfg(windows)]
    const SEPARATORS: &[char] = &['/', '\\'];
    #[cfg(not(windows))]
    const SEPARATORS: &[char] = &['/'];

    process_name
        .rsplit(SEPARATORS)
        .next()
        .unwrap_or(process_name)
        .to_ascii_lowercase()
}

fn alnum_lower(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

fn strip_tails(mut slug: String, tails: &[&str]) -> String {
    let mut changed = true;
    while changed {
        changed = false;
        for tail in tails {
            if slug.ends_with(tail) && slug.len() - tail.len() >= MIN_SLUG {
                slug.truncate(slug.len() - tail.len());
                changed = true;
                break;
            }
        }
    }
    slug
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::steam_library::SteamGame;
    use std::path::PathBuf;

    fn crimson_steam() -> SteamLibraryIndex {
        let mut steam = SteamLibraryIndex::default();
        steam.games.insert(
            3321460,
            SteamGame {
                app_id: 3321460,
                title: "Crimson Desert Enhanced".into(),
                install_path: PathBuf::from(r"D:\SteamLibrary\steamapps\common\Crimson Desert"),
            },
        );
        steam
    }

    fn proc(name: &str, exe: &str) -> ProcessSnapshot {
        ProcessSnapshot {
            pid: 1,
            name: name.into(),
            exe_path: Some(exe.into()),
            cmdline: None,
        }
    }

    #[test]
    fn crimson_desert_enhanced_matches_exe() {
        assert!(exe_looks_like_title(
            "Crimson Desert Enhanced",
            "CrimsonDesert.exe"
        ));
        assert!(exe_looks_like_title("Dota 2", "dota2.exe"));
        assert!(exe_looks_like_title("Hades", "/Games/Hades/Hades.exe"));
        #[cfg(windows)]
        assert!(exe_looks_like_title("Hades", r"D:\Games\Hades\Hades.exe"));
    }

    #[cfg(not(windows))]
    #[test]
    fn backslashes_are_not_separators_on_unix() {
        // A Unix process filename may legitimately contain backslashes; keep it intact.
        assert_eq!(process_basename(r"weird\name.exe"), r"weird\name.exe");
    }

    #[test]
    fn mismatched_and_generic_exes_do_not_match() {
        assert!(!exe_looks_like_title("Apex Legends", "r5apex.exe"));
        assert!(!exe_looks_like_title("Some Game Title", "game.exe"));
        assert!(!exe_looks_like_title(
            "Crimson Desert",
            "UnrealCEFSubProcess.exe"
        ));
        assert!(!exe_looks_like_title("IT", "it.exe"));
    }

    #[test]
    fn title_like_path_hit_is_medium() {
        let steam = crimson_steam();
        let proc = proc(
            "CrimsonDesert.exe",
            r"D:\SteamLibrary\steamapps\common\Crimson Desert\bin64\CrimsonDesert.exe",
        );
        let mut identities = vec![steam.match_path(&proc).unwrap()];
        assert_eq!(identities[0].confidence, Confidence::Low);
        let mut pending = Vec::new();
        finalize_steam_path_hits(&steam, &mut identities, &mut pending, &[proc]);
        assert!(pending.is_empty());
        assert_eq!(identities.len(), 1);
        assert_eq!(identities[0].confidence, Confidence::Medium);
        assert_eq!(identities[0].steam_app_id, Some(3321460));
        assert_eq!(identities[0].exe.as_deref(), Some("CrimsonDesert.exe"));
    }

    #[test]
    fn prefers_title_like_exe_over_launcher() {
        let steam = crimson_steam();
        let launcher = proc(
            "PALauncher.exe",
            r"D:\SteamLibrary\steamapps\common\Crimson Desert\PALauncher.exe",
        );
        let game = proc(
            "CrimsonDesert.exe",
            r"D:\SteamLibrary\steamapps\common\Crimson Desert\bin64\CrimsonDesert.exe",
        );
        let mut identities = vec![steam.match_path(&launcher).unwrap()];
        assert_eq!(identities[0].exe.as_deref(), Some("PALauncher.exe"));
        let mut pending = Vec::new();
        finalize_steam_path_hits(&steam, &mut identities, &mut pending, &[launcher, game]);
        assert!(pending.is_empty());
        assert_eq!(identities[0].confidence, Confidence::Medium);
        assert_eq!(identities[0].exe.as_deref(), Some("CrimsonDesert.exe"));
    }

    #[test]
    fn mismatched_path_hit_goes_pending() {
        let mut steam = SteamLibraryIndex::default();
        steam.games.insert(
            1172470,
            SteamGame {
                app_id: 1172470,
                title: "Apex Legends".into(),
                install_path: PathBuf::from(r"D:\Steam\steamapps\common\Apex Legends"),
            },
        );
        let proc = proc(
            "r5apex.exe",
            r"D:\Steam\steamapps\common\Apex Legends\r5apex.exe",
        );
        let mut identities = vec![steam.match_path(&proc).unwrap()];
        let mut pending = Vec::new();
        finalize_steam_path_hits(&steam, &mut identities, &mut pending, &[proc]);
        assert!(identities.is_empty());
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].suggested_title, "Apex Legends");
        assert_eq!(pending[0].identity_id.as_deref(), Some("steam:1172470"));
    }

    #[test]
    fn unindexed_steamapps_detects_missing_library_row() {
        let steam = SteamLibraryIndex::default();
        let proc = proc(
            "CrimsonDesert.exe",
            r"D:\SteamLibrary\steamapps\common\Crimson Desert\bin64\CrimsonDesert.exe",
        );
        assert!(is_unindexed_steamapps_process(&proc, &steam));
        assert!(!is_unindexed_steamapps_process(&proc, &crimson_steam()));
    }
}
