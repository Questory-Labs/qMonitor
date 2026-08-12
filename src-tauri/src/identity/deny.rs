//! Processes that must never be treated as games.

pub fn is_denied(process_name: &str) -> bool {
    let name = process_name.to_ascii_lowercase();
    let stem = name.trim_end_matches(".exe");

    const EXACT: &[&str] = &[
        "steam",
        "steamwebhelper",
        "gameoverlayui",
        "gameoverlayui64",
        "steamerrorreporter",
        "steamerrorreporter64",
        "steamservice",
        "cef_server",
        "crashpad_handler",
        "unitycrashhandler",
        "unitycrashhandler64",
        "unitycrashhandler32",
        "vcredist_x64",
        "vcredist_x86",
        "dxsetup",
        "dotnet",
        "msiexec",
        "setup",
        "installer",
        // Helpers / overlays that must never surface as games
        "msedgewebview2",
        "msedge",
        "chrome",
        "firefox",
        "explorer",
        "qtwebengineprocess",
        "obs-browser-page",
        "obs64",
        "obs32",
        "obs",
        "nvidia broadcast",
        "nvidia share",
        "nvcontainer",
        "textinputhost",
        "applicationframehost",
        "searchhost",
        "runtimebroker",
        "svchost",
        "dllhost",
        "conhost",
        "taskmgr",
        "powershell",
        "pwsh",
        "cmd",
        "code",
        "cursor",
        "node",
        "python",
        "cargo",
        "rustc",
    ];

    if EXACT.iter().any(|d| stem == *d || name == *d) {
        return true;
    }

    stem.starts_with("unitycrashhandler")
        || stem.starts_with("vcredist")
        || stem.contains("crashhandler")
        || stem.contains("crashreporter")
        || stem.contains("webview")
        || stem.starts_with("qtwebengine")
        || name.contains("easyanticheat")
        || name.contains("battleye")
        || name.contains("nvidia broadcast")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn denies_steam_helpers() {
        assert!(is_denied("steam.exe"));
        assert!(is_denied("steamwebhelper"));
        assert!(is_denied("UnityCrashHandler64.exe"));
        assert!(!is_denied("dota2.exe"));
        assert!(!is_denied("Hades.exe"));
    }

    #[test]
    fn denies_common_non_games() {
        assert!(is_denied("msedgewebview2.exe"));
        assert!(is_denied("QtWebEngineProcess.exe"));
        assert!(is_denied("obs-browser-page.exe"));
        assert!(is_denied("NVIDIA Broadcast.exe"));
    }
}
