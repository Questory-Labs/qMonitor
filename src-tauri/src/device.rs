//! Stable hashed device id for refresh binding (anti-exfil).

use std::fs;

use sha2::{Digest, Sha256};

use crate::config::AppConfig;

/// Privacy-safe device id: SHA-256 hex of install salt + hostname + OS.
pub fn device_id() -> Result<String, String> {
    let salt = load_or_create_salt()?;
    let host = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "unknown".into());
    let os = std::env::consts::OS;
    let mut hasher = Sha256::new();
    hasher.update(salt.as_bytes());
    hasher.update(b"|");
    hasher.update(host.as_bytes());
    hasher.update(b"|");
    hasher.update(os.as_bytes());
    Ok(hex::encode(hasher.finalize()))
}

fn load_or_create_salt() -> Result<String, String> {
    let dir = AppConfig::config_dir();
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join("device_salt");
    if let Ok(existing) = fs::read_to_string(&path) {
        let trimmed = existing.trim().to_string();
        if trimmed.len() >= 16 {
            return Ok(trimmed);
        }
    }
    let salt = uuid::Uuid::new_v4().to_string();
    fs::write(&path, &salt).map_err(|e| e.to_string())?;
    Ok(salt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_id_is_hex_64() {
        let id = device_id().unwrap();
        assert_eq!(id.len(), 64);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
