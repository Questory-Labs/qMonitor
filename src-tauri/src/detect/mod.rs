pub mod process;
pub mod steam_reaper;

use crate::identity::ProcessSnapshot;

pub fn snapshot_processes() -> Vec<ProcessSnapshot> {
    process::list_processes()
}

pub fn primary_identity<'a>(
    identities: &'a [crate::identity::GameIdentity],
    foreground_pid: Option<u32>,
    processes: &[ProcessSnapshot],
) -> Option<&'a crate::identity::GameIdentity> {
    if identities.is_empty() {
        return None;
    }
    if let Some(pid) = foreground_pid {
        if let Some(proc) = processes.iter().find(|p| p.pid == pid) {
            // Prefer identity whose exe matches foreground process name
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
    // Prefer Steam high-confidence
    identities
        .iter()
        .find(|i| i.source == "steam")
        .or_else(|| identities.first())
}

#[cfg(target_os = "windows")]
pub fn foreground_pid() -> Option<u32> {
    windows_foreground_pid()
}

#[cfg(not(target_os = "windows"))]
pub fn foreground_pid() -> Option<u32> {
    None
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
