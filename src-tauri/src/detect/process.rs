use sysinfo::{ProcessesToUpdate, System};

use crate::identity::ProcessSnapshot;

pub fn list_processes() -> Vec<ProcessSnapshot> {
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);
    sys.processes()
        .iter()
        .map(|(pid, p)| {
            let name = p.name().to_string_lossy().into_owned();
            let exe_path = p.exe().map(|x| x.to_string_lossy().into_owned());
            let cmdline = {
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
