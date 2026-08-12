use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

use super::{Confidence, GameIdentity, ProcessSnapshot};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogExecutable {
    pub os: String,
    pub name: String,
    #[serde(default)]
    pub is_launcher: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogEntry {
    pub id: String,
    pub name: String,
    pub executables: Vec<CatalogExecutable>,
    #[serde(default)]
    pub path_hints: Vec<String>,
    #[serde(default)]
    pub arguments: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct LocalCatalog {
    pub entries: Vec<CatalogEntry>,
}

impl LocalCatalog {
    pub fn load_from_path(path: &Path) -> Self {
        match fs::read_to_string(path) {
            Ok(raw) => Self {
                entries: serde_json::from_str(&raw).unwrap_or_default(),
            },
            Err(_) => Self::default(),
        }
    }

    pub fn match_process(&self, proc: &ProcessSnapshot) -> Option<GameIdentity> {
        let os = current_os_label();
        let pname = proc.name.to_ascii_lowercase();
        let path_l = proc
            .exe_path
            .as_deref()
            .unwrap_or("")
            .to_ascii_lowercase();
        let cmd = proc.cmdline.as_deref().unwrap_or("").to_ascii_lowercase();

        let mut best: Option<(Confidence, &CatalogEntry)> = None;

        for entry in &self.entries {
            let exe_hit = entry.executables.iter().any(|e| {
                e.os == os
                    && !e.is_launcher
                    && (pname == e.name.to_ascii_lowercase()
                        || pname == format!("{}.exe", e.name.to_ascii_lowercase()))
            });
            if !exe_hit {
                continue;
            }

            if let Some(args) = &entry.arguments {
                if !cmd.contains(&args.to_ascii_lowercase()) {
                    continue;
                }
            }

            let hint_hit = !entry.path_hints.is_empty()
                && entry
                    .path_hints
                    .iter()
                    .any(|h| path_l.contains(&h.to_ascii_lowercase()));
            // Medium when path hint hits and/or required arguments matched; bare exe name → Low.
            let conf = if hint_hit || entry.arguments.is_some() {
                Confidence::Medium
            } else {
                Confidence::Low
            };

            let replace = match &best {
                None => true,
                Some((c, _)) => confidence_rank(conf) > confidence_rank(*c),
            };
            if replace {
                best = Some((conf, entry));
            }
        }

        best.map(|(confidence, entry)| GameIdentity {
            id: format!("catalog:{}", entry.id),
            title: entry.name.clone(),
            steam_app_id: None,
            exe: Some(proc.name.clone()),
            confidence,
            source: "catalog".into(),
            fingerprint: None,
        })
    }
}

fn confidence_rank(c: Confidence) -> u8 {
    match c {
        Confidence::High => 3,
        Confidence::Medium => 2,
        Confidence::Low => 1,
        Confidence::None => 0,
    }
}

fn current_os_label() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "other"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_hint_raises_confidence() {
        let catalog = LocalCatalog {
            entries: vec![CatalogEntry {
                id: "local:hades".into(),
                name: "Hades".into(),
                executables: vec![CatalogExecutable {
                    os: current_os_label().into(),
                    name: "Hades.exe".into(),
                    is_launcher: false,
                }],
                path_hints: vec!["Hades".into()],
                arguments: None,
            }],
        };
        let proc = ProcessSnapshot {
            pid: 1,
            name: "Hades.exe".into(),
            exe_path: Some(r"C:\Games\Hades\Hades.exe".into()),
            cmdline: None,
        };
        let id = catalog.match_process(&proc).unwrap();
        assert_eq!(id.confidence, Confidence::Medium);
        assert_eq!(id.title, "Hades");
    }

    #[test]
    fn bare_exe_is_low_confidence() {
        let catalog = LocalCatalog {
            entries: vec![CatalogEntry {
                id: "local:hades".into(),
                name: "Hades".into(),
                executables: vec![CatalogExecutable {
                    os: current_os_label().into(),
                    name: "Hades.exe".into(),
                    is_launcher: false,
                }],
                path_hints: vec!["SupergiantGames".into()],
                arguments: None,
            }],
        };
        let proc = ProcessSnapshot {
            pid: 1,
            name: "Hades.exe".into(),
            exe_path: Some(r"C:\elsewhere\Hades.exe".into()),
            cmdline: None,
        };
        let id = catalog.match_process(&proc).unwrap();
        assert_eq!(id.confidence, Confidence::Low);
    }

    #[test]
    fn arguments_disambiguate_shared_exe() {
        let catalog = LocalCatalog {
            entries: vec![CatalogEntry {
                id: "local:gmod".into(),
                name: "Garry's Mod".into(),
                executables: vec![CatalogExecutable {
                    os: current_os_label().into(),
                    name: "hl2.exe".into(),
                    is_launcher: false,
                }],
                path_hints: vec!["garrysmod".into()],
                arguments: Some("-game garrysmod".into()),
            }],
        };
        let hit = ProcessSnapshot {
            pid: 1,
            name: "hl2.exe".into(),
            exe_path: Some(r"C:\Steam\steamapps\common\GarrysMod\hl2.exe".into()),
            cmdline: Some("hl2.exe -game garrysmod".into()),
        };
        let miss = ProcessSnapshot {
            pid: 2,
            name: "hl2.exe".into(),
            exe_path: Some(r"C:\Steam\steamapps\common\HalfLife2\hl2.exe".into()),
            cmdline: Some("hl2.exe -game hl2".into()),
        };
        assert!(catalog.match_process(&hit).is_some());
        assert!(catalog.match_process(&miss).is_none());
    }
}
