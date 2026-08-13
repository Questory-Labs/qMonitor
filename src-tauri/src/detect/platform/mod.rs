//! OS-specific Steam process detection. Shared merge lives in identity::resolver.

pub mod linux;
pub mod windows;

use crate::identity::{GameIdentity, ProcessSnapshot};
use crate::identity::steam_library::SteamLibraryIndex;

/// Dispatch to the host OS detector.
pub fn detect_steam(
    processes: &[ProcessSnapshot],
    steam: &SteamLibraryIndex,
) -> Vec<GameIdentity> {
    #[cfg(target_os = "windows")]
    {
        windows::detect_steam(processes, steam)
    }
    #[cfg(target_os = "linux")]
    {
        linux::detect_steam(processes, steam)
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        let _ = (processes, steam);
        Vec::new()
    }
}

pub fn foreground_pid() -> Option<u32> {
    #[cfg(target_os = "windows")]
    {
        windows::foreground_pid()
    }
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}
