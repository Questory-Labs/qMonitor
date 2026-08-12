use sha2::{Digest, Sha256};

use super::ProcessSnapshot;

/// Privacy-safe fingerprint for future crowdsourced identity (qIdentity).
pub fn fingerprint_process(proc: &ProcessSnapshot) -> String {
    let mut hasher = Sha256::new();
    hasher.update(proc.name.to_ascii_lowercase().as_bytes());
    if let Some(path) = &proc.exe_path {
        // Use parent folder name + basename, not full user path.
        let p = std::path::Path::new(path);
        let parent = p
            .parent()
            .and_then(|x| x.file_name())
            .and_then(|x| x.to_str())
            .unwrap_or("");
        let file = p
            .file_name()
            .and_then(|x| x.to_str())
            .unwrap_or(proc.name.as_str());
        hasher.update(b"|");
        hasher.update(parent.to_ascii_lowercase().as_bytes());
        hasher.update(b"|");
        hasher.update(file.to_ascii_lowercase().as_bytes());
    }
    if let Some(cmd) = &proc.cmdline {
        // Only hash short stable flags, not full paths in argv.
        for token in cmd.split_whitespace().take(8) {
            if token.starts_with('-') {
                hasher.update(b"|");
                hasher.update(token.as_bytes());
            }
        }
    }
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_stable() {
        let proc = ProcessSnapshot {
            pid: 1,
            name: "Hades.exe".into(),
            exe_path: Some(r"D:\Games\Hades\Hades.exe".into()),
            cmdline: Some(r"D:\Games\Hades\Hades.exe -windowed".into()),
        };
        let a = fingerprint_process(&proc);
        let b = fingerprint_process(&proc);
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
    }
}
