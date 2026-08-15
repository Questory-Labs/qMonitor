//! Liveness stamps, tray tooltip, and file logging.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Utc};
use tracing_appender::non_blocking::{NonBlocking, WorkerGuard};
use tracing_subscriber::fmt::writer::MakeWriterExt;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{reload, EnvFilter};

use crate::config::{AppConfig, LogLevel};

/// Delete log files older than this when logging is on.
pub const LOG_KEEP_DAYS: u64 = 3;
/// Cap total size of `qmonitor.log*` files.
pub const LOG_MAX_BYTES: u64 = 5 * 1024 * 1024;
pub const LOG_PRUNE_EVERY: Duration = Duration::from_secs(30 * 60);

static FILE_SINK: Mutex<Option<(NonBlocking, WorkerGuard)>> = Mutex::new(None);
static FILE_ON: AtomicBool = AtomicBool::new(false);
static FILTER_RELOAD: OnceLock<reload::Handle<EnvFilter, tracing_subscriber::Registry>> =
    OnceLock::new();

#[derive(Debug, Clone, Default)]
pub struct RuntimeHealth {
    pub last_detect_at: Option<DateTime<Utc>>,
    pub last_persist_at: Option<DateTime<Utc>>,
    pub last_push_at: Option<DateTime<Utc>>,
    pub db_ok: bool,
    pub db_generation: u64,
    pub db_reconnects: u64,
    pub detect_timeouts: u64,
    pub last_error: Option<String>,
}

impl RuntimeHealth {
    pub fn loop_alive(&self, poll_interval_secs: u64) -> bool {
        let Some(at) = self.last_detect_at else {
            return false;
        };
        let max = chrono::Duration::seconds((poll_interval_secs.max(1) * 3) as i64);
        Utc::now().signed_duration_since(at) < max
    }

    pub fn tray_tooltip(&self, poll_interval_secs: u64) -> String {
        let poll = match self.last_detect_at {
            Some(at) => {
                let secs = Utc::now().signed_duration_since(at).num_seconds().max(0);
                format!("poll {secs}s ago")
            }
            None => "poll —".into(),
        };
        let db = if self.db_ok { "DB ok" } else { "DB down" };
        let stuck = if self.loop_alive(poll_interval_secs) {
            ""
        } else {
            " · loop stuck"
        };
        format!("qMonitor · {poll} · {db}{stuck}")
    }
}

pub fn log_dir() -> PathBuf {
    AppConfig::config_dir().join("logs")
}

/// Stderr + rolling daily file. File sink is created on first enabled write (default off).
pub fn init_tracing() {
    let level = AppConfig::load().log_level;
    FILE_ON.store(level.file_enabled(), Ordering::Relaxed);
    prune_now(level);

    let (filter, reload_handle) =
        reload::Layer::new(EnvFilter::new(level.env_filter()));
    let _ = FILTER_RELOAD.set(reload_handle);

    tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stderr.and(GatedMakeWriter)),
        )
        .init();

    if level.file_enabled() {
        tracing::info!(path = %log_dir().display(), level = ?level, "file logging on");
    }
}

pub fn apply_log_level(level: LogLevel) {
    FILE_ON.store(level.file_enabled(), Ordering::Relaxed);
    if !level.file_enabled() {
        drop_file_sink();
    }
    if let Some(handle) = FILTER_RELOAD.get() {
        if let Err(e) = handle.reload(EnvFilter::new(level.env_filter())) {
            tracing::warn!(%e, "failed to reload log filter");
        }
    }
    let dir = log_dir();
    if level.file_enabled() {
        let _ = fs::create_dir_all(&dir);
    }
    prune_log_dir(&dir, prune_keep_days(level), prune_max_bytes(level));
}

pub fn prune_now(level: LogLevel) {
    prune_log_dir(&log_dir(), prune_keep_days(level), prune_max_bytes(level));
}

fn prune_keep_days(level: LogLevel) -> u64 {
    if level.file_enabled() {
        LOG_KEEP_DAYS
    } else {
        0
    }
}

fn prune_max_bytes(level: LogLevel) -> u64 {
    if level.file_enabled() {
        LOG_MAX_BYTES
    } else {
        0
    }
}

fn is_qmonitor_log(name: &str) -> bool {
    name.starts_with("qmonitor.log")
}

/// Age + size cap. `keep_days == 0` or `max_bytes == 0` deletes every `qmonitor.log*` file.
pub fn prune_log_dir(dir: &Path, keep_days: u64, max_bytes: u64) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(PathBuf, SystemTime, u64)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if !is_qmonitor_log(name) {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        files.push((path, modified, meta.len()));
    }

    if keep_days == 0 || max_bytes == 0 {
        for (path, _, _) in files {
            let _ = fs::remove_file(path);
        }
        return;
    }

    let cutoff = SystemTime::now()
        .checked_sub(Duration::from_secs(keep_days.saturating_mul(86_400)))
        .unwrap_or(SystemTime::UNIX_EPOCH);
    files.retain(|(path, modified, _)| {
        if *modified < cutoff {
            let _ = fs::remove_file(path);
            false
        } else {
            true
        }
    });

    files.sort_by_key(|(_, modified, _)| *modified);
    let mut total: u64 = files.iter().map(|(_, _, len)| *len).sum();
    for (path, _, len) in files {
        if total <= max_bytes {
            break;
        }
        if fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(len);
        }
    }
}

fn drop_file_sink() {
    if let Ok(mut g) = FILE_SINK.lock() {
        *g = None;
    }
}

fn file_nonblocking() -> Option<NonBlocking> {
    if !FILE_ON.load(Ordering::Relaxed) {
        return None;
    }
    let mut g = FILE_SINK.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_none() {
        let dir = log_dir();
        let _ = fs::create_dir_all(&dir);
        let appender = tracing_appender::rolling::daily(&dir, "qmonitor.log");
        let (nb, guard) = tracing_appender::non_blocking(appender);
        *g = Some((nb.clone(), guard));
        return Some(nb);
    }
    g.as_ref().map(|(nb, _)| nb.clone())
}

#[derive(Clone, Copy)]
struct GatedMakeWriter;

struct GatedWriter {
    inner: Option<NonBlocking>,
}

impl Write for GatedWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.inner.as_mut() {
            Some(w) => w.write(buf),
            None => Ok(buf.len()),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.inner.as_mut() {
            Some(w) => w.flush(),
            None => Ok(()),
        }
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for GatedMakeWriter {
    type Writer = GatedWriter;

    fn make_writer(&'a self) -> Self::Writer {
        GatedWriter {
            inner: file_nonblocking(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loop_alive_false_without_detect() {
        let h = RuntimeHealth::default();
        assert!(!h.loop_alive(3));
    }

    #[test]
    fn loop_alive_true_when_recent() {
        let h = RuntimeHealth {
            last_detect_at: Some(Utc::now()),
            ..Default::default()
        };
        assert!(h.loop_alive(3));
    }

    #[test]
    fn tooltip_includes_db_state() {
        let h = RuntimeHealth {
            last_detect_at: Some(Utc::now()),
            db_ok: true,
            ..Default::default()
        };
        let tip = h.tray_tooltip(3);
        assert!(tip.contains("DB ok"), "{tip}");
        assert!(tip.contains("qMonitor"), "{tip}");
    }

    #[test]
    fn prune_deletes_oversized_logs_keeps_other_files() {
        let dir = tempfile::tempdir().unwrap();
        let big = dir.path().join("qmonitor.log.2026-01-01");
        fs::write(&big, vec![b'y'; 200]).unwrap();
        let other = dir.path().join("notes.txt");
        fs::write(&other, b"keep").unwrap();

        prune_log_dir(dir.path(), 3, 50);
        assert!(!big.exists(), "size-pruned");
        assert!(other.exists(), "non-log kept");
    }

    #[test]
    fn prune_zero_days_deletes_all_logs() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("qmonitor.log");
        fs::write(&log, b"x").unwrap();
        prune_log_dir(dir.path(), 0, 0);
        assert!(!log.exists());
    }

    #[test]
    fn prune_deletes_old_logs() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("qmonitor.log.old");
        fs::write(&old, vec![b'x'; 100]).unwrap();
        let old_time = SystemTime::now() - Duration::from_secs(10 * 86_400);
        let ok = fs::OpenOptions::new()
            .write(true)
            .open(&old)
            .and_then(|f| f.set_modified(old_time))
            .is_ok();
        if !ok {
            return;
        }
        prune_log_dir(dir.path(), 3, LOG_MAX_BYTES);
        assert!(!old.exists(), "age-pruned");
    }
}
