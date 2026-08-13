use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

use crate::identity::ProcessSnapshot;

/// Basenames whose command line may contain `SteamLaunch AppId=` (Linux reaper / wrappers).
pub fn needs_launch_cmdline(process_name: &str) -> bool {
    let stem = process_name
        .to_ascii_lowercase()
        .trim_end_matches(".exe")
        .to_string();
    stem == "reaper"
        || stem == "steam-launch-wrapper"
        || stem == "steamlaunch"
        || stem.starts_with("steam-launch")
}

pub fn list_processes() -> Vec<ProcessSnapshot> {
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_exe(UpdateKind::Always),
    );

    let cmd_pids: Vec<Pid> = sys
        .processes()
        .iter()
        .filter(|(_, p)| needs_launch_cmdline(&p.name().to_string_lossy()))
        .map(|(pid, _)| *pid)
        .collect();
    if !cmd_pids.is_empty() {
        sys.refresh_processes_specifics(
            ProcessesToUpdate::Some(&cmd_pids),
            false,
            ProcessRefreshKind::nothing().with_cmd(UpdateKind::Always),
        );
    }

    sys.processes()
        .iter()
        .map(|(pid, p)| {
            let name = p.name().to_string_lossy().into_owned();
            let exe_path = p.exe().map(|x| x.to_string_lossy().into_owned());
            let cmdline = if needs_launch_cmdline(&name) {
                let args: Vec<String> = p
                    .cmd()
                    .iter()
                    .map(|a| a.to_string_lossy().into_owned())
                    .collect();
                if args.is_empty() {
                    None
                } else {
                    Some(args.join(" "))
                }
            } else {
                None
            };
            ProcessSnapshot {
                pid: pid.as_u32(),
                name,
                exe_path,
                cmdline,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_cmdline_only_for_reaper_like_names() {
        assert!(needs_launch_cmdline("reaper"));
        assert!(needs_launch_cmdline("reaper.exe"));
        assert!(needs_launch_cmdline("steam-launch-wrapper"));
        assert!(!needs_launch_cmdline("dota2.exe"));
        assert!(!needs_launch_cmdline("r5apex.exe"));
        assert!(!needs_launch_cmdline("chrome.exe"));
        assert!(!needs_launch_cmdline("steam.exe"));
    }
}
