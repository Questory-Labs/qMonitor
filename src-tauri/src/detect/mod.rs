pub mod platform;
pub mod process;
pub mod steam_reaper;

use crate::identity::{Confidence, GameIdentity, ProcessSnapshot};

pub fn snapshot_processes() -> Vec<ProcessSnapshot> {
    process::list_processes()
}

pub fn foreground_pid() -> Option<u32> {
    platform::foreground_pid()
}

pub fn primary_identity<'a>(
    identities: &'a [GameIdentity],
    foreground_pid: Option<u32>,
    processes: &[ProcessSnapshot],
) -> Option<&'a GameIdentity> {
    if identities.is_empty() {
        return None;
    }
    if let Some(high) = identities.iter().find(|i| i.confidence == Confidence::High) {
        return Some(high);
    }
    if let Some(pid) = foreground_pid {
        if let Some(proc) = processes.iter().find(|p| p.pid == pid) {
            if let Some(id) = identities.iter().find(|i| {
                i.exe
                    .as_ref()
                    .map(|e| e.eq_ignore_ascii_case(&proc.name))
                    .unwrap_or(false)
            }) {
                return Some(id);
            }
        }
    }
    identities.first()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{Confidence, GameIdentity, ProcessSnapshot};

    fn id(key: &str, exe: &str, confidence: Confidence) -> GameIdentity {
        GameIdentity {
            id: key.into(),
            title: key.into(),
            steam_app_id: None,
            exe: Some(exe.into()),
            confidence,
            source: "test".into(),
            fingerprint: None,
        }
    }

    #[test]
    fn high_beats_foreground_medium() {
        let identities = vec![
            id("steam:1", "game.exe", Confidence::Medium),
            id("steam:2", "reaper", Confidence::High),
        ];
        let procs = vec![ProcessSnapshot {
            pid: 10,
            name: "game.exe".into(),
            exe_path: None,
            cmdline: None,
        }];
        let primary = primary_identity(&identities, Some(10), &procs).unwrap();
        assert_eq!(primary.id, "steam:2");
    }
}
