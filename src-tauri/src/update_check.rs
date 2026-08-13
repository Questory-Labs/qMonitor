//! Daily GitHub Releases poll: notify with a link, never auto-install.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::config::{AppConfig, UpdateChannel};

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const CHECK_INTERVAL_SECS: u64 = 24 * 60 * 60;
const RELEASES_PATH: &str = "/Questory-Labs/qMonitor/releases";

const API_LATEST: &str = "https://api.github.com/repos/Questory-Labs/qMonitor/releases/latest";
const API_LIST: &str = "https://api.github.com/repos/Questory-Labs/qMonitor/releases?per_page=30";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PendingUpdate {
    pub tag: String,
    pub version: String,
    pub html_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct UpdateCheckCache {
    #[serde(default)]
    last_checked_at: u64,
    #[serde(default)]
    last_channel: Option<UpdateChannel>,
    #[serde(default)]
    dismissed_key: Option<String>,
    #[serde(default)]
    last_known: Option<PendingUpdate>,
}

#[derive(Debug, Clone, Deserialize)]
struct GhRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    prerelease: bool,
}

fn cache_path() -> PathBuf {
    AppConfig::config_dir().join("update-check.json")
}

pub fn installed_version() -> &'static str {
    APP_VERSION
}

pub async fn check(
    channel: UpdateChannel,
    force: bool,
) -> Result<Option<PendingUpdate>, String> {
    check_at(
        channel,
        force,
        APP_VERSION,
        &cache_path(),
        now_unix_secs(),
        github_fetch,
    )
    .await
}

pub fn dismiss_current() -> Result<(), String> {
    dismiss_at(&cache_path())
}

pub fn open_release_url(url: &str) -> Result<(), String> {
    if !is_allowed_release_url(url) {
        return Err("refusing to open a non-qMonitor GitHub release URL".into());
    }
    open::that(url).map_err(|e| e.to_string())
}

fn is_canary_tag(tag: &str) -> bool {
    let rest = tag.strip_prefix('v').unwrap_or(tag);
    rest.split_once("-canary.")
        .is_some_and(|(ver, sha)| !ver.is_empty() && !sha.is_empty() && !sha.contains('/'))
}

fn version_from_tag(tag: &str) -> &str {
    tag.strip_prefix('v').unwrap_or(tag)
}

fn is_newer_stable(remote_tag: &str, installed: &str) -> bool {
    let remote = parse_semver(remote_tag);
    let local = parse_semver(installed);
    match (remote, local) {
        (Some(r), Some(l)) => r > l,
        _ => false,
    }
}

fn is_newer_canary(remote_tag: &str, installed: &str) -> bool {
    version_from_tag(remote_tag) != installed
}

fn is_update(channel: UpdateChannel, remote_tag: &str, installed: &str) -> bool {
    match channel {
        UpdateChannel::Stable => is_newer_stable(remote_tag, installed),
        UpdateChannel::Canary => is_newer_canary(remote_tag, installed),
    }
}

fn is_allowed_release_url(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    if parsed.scheme() != "https" || parsed.host_str() != Some("github.com") {
        return false;
    }
    let path = parsed.path();
    path == RELEASES_PATH || path.starts_with(&format!("{RELEASES_PATH}/"))
}

fn pick_stable(release: &GhRelease) -> Option<PendingUpdate> {
    if release.prerelease || is_canary_tag(&release.tag_name) {
        return None;
    }
    Some(pending_from_release(release))
}

fn pick_canary(releases: &[GhRelease]) -> Option<PendingUpdate> {
    releases
        .iter()
        .find(|r| r.prerelease && is_canary_tag(&r.tag_name))
        .map(pending_from_release)
}

fn pending_from_release(release: &GhRelease) -> PendingUpdate {
    PendingUpdate {
        tag: release.tag_name.clone(),
        version: version_from_tag(&release.tag_name).to_string(),
        html_url: release.html_url.clone(),
    }
}

fn parse_semver(tag_or_version: &str) -> Option<semver::Version> {
    semver::Version::parse(version_from_tag(tag_or_version)).ok()
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}

fn should_skip_network(cache: &UpdateCheckCache, channel: UpdateChannel, force: bool, now: u64) -> bool {
    if force {
        return false;
    }
    if cache.last_channel != Some(channel) {
        return false;
    }
    now.saturating_sub(cache.last_checked_at) < CHECK_INTERVAL_SECS
}

fn filter_pending(
    pending: Option<&PendingUpdate>,
    channel: UpdateChannel,
    installed: &str,
    dismissed_key: Option<&str>,
) -> Option<PendingUpdate> {
    let pending = pending?;
    if dismissed_key == Some(pending.tag.as_str()) {
        return None;
    }
    if !is_update(channel, &pending.tag, installed) {
        return None;
    }
    Some(pending.clone())
}

fn load_cache(path: &Path) -> UpdateCheckCache {
    match fs::read_to_string(path) {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
        Err(_) => UpdateCheckCache::default(),
    }
}

fn save_cache(path: &Path, cache: &UpdateCheckCache) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let raw = serde_json::to_string_pretty(cache).map_err(|e| e.to_string())?;
    fs::write(path, raw).map_err(|e| e.to_string())
}

fn dismiss_at(path: &Path) -> Result<(), String> {
    let mut cache = load_cache(path);
    if let Some(known) = &cache.last_known {
        cache.dismissed_key = Some(known.tag.clone());
        save_cache(path, &cache)?;
    }
    Ok(())
}

async fn check_at<F, Fut>(
    channel: UpdateChannel,
    force: bool,
    installed: &str,
    cache_path: &Path,
    now: u64,
    fetch: F,
) -> Result<Option<PendingUpdate>, String>
where
    F: FnOnce(UpdateChannel) -> Fut,
    Fut: std::future::Future<Output = Result<Option<PendingUpdate>, String>>,
{
    let mut cache = load_cache(cache_path);
    if should_skip_network(&cache, channel, force, now) {
        return Ok(filter_pending(
            cache.last_known.as_ref(),
            channel,
            installed,
            cache.dismissed_key.as_deref(),
        ));
    }

    let fetched = fetch(channel).await?;
    cache.last_checked_at = now;
    cache.last_channel = Some(channel);
    cache.last_known = fetched.clone();
    save_cache(cache_path, &cache)?;
    Ok(filter_pending(
        fetched.as_ref(),
        channel,
        installed,
        cache.dismissed_key.as_deref(),
    ))
}

async fn github_fetch(channel: UpdateChannel) -> Result<Option<PendingUpdate>, String> {
    match channel {
        UpdateChannel::Stable => fetch_latest_stable().await,
        UpdateChannel::Canary => fetch_latest_canary().await,
    }
}

async fn fetch_latest_stable() -> Result<Option<PendingUpdate>, String> {
    let resp = github_get(API_LATEST).await?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !resp.status().is_success() {
        return Err(format!("GitHub latest release HTTP {}", resp.status()));
    }
    let release: GhRelease = resp.json().await.map_err(|e| e.to_string())?;
    Ok(pick_stable(&release))
}

async fn fetch_latest_canary() -> Result<Option<PendingUpdate>, String> {
    let resp = github_get(API_LIST).await?;
    if !resp.status().is_success() {
        return Err(format!("GitHub releases HTTP {}", resp.status()));
    }
    let releases: Vec<GhRelease> = resp.json().await.map_err(|e| e.to_string())?;
    Ok(pick_canary(&releases))
}

async fn github_get(url: &str) -> Result<reqwest::Response, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;
    client
        .get(url)
        .header("User-Agent", format!("qMonitor/{APP_VERSION}"))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, prerelease: bool) -> GhRelease {
        GhRelease {
            tag_name: tag.into(),
            html_url: format!("https://github.com{RELEASES_PATH}/tag/{tag}"),
            prerelease,
        }
    }

    #[test]
    fn canary_tag_shape() {
        assert!(is_canary_tag("v0.0.1-canary.a1b2c3d"));
        assert!(is_canary_tag("0.0.1-canary.deadbee"));
        assert!(!is_canary_tag("canary"));
        assert!(!is_canary_tag("v0.0.1"));
        assert!(!is_canary_tag("v0.0.1-beta.1"));
    }

    #[test]
    fn stable_semver_newer() {
        assert!(is_newer_stable("v0.0.2", "0.0.1"));
        assert!(!is_newer_stable("v0.0.1", "0.0.1"));
        assert!(is_newer_stable("v0.0.1", "0.0.1-canary.abc1234"));
        assert!(!is_newer_stable("v0.0.1", "0.0.2-canary.abc1234"));
        assert!(!is_newer_stable("not-a-version", "0.0.1"));
    }

    #[test]
    fn canary_identity_differs() {
        assert!(is_newer_canary("v0.0.1-canary.aaa1111", "0.0.1"));
        assert!(!is_newer_canary("v0.0.1-canary.aaa1111", "0.0.1-canary.aaa1111"));
        assert!(is_newer_canary(
            "v0.0.1-canary.bbb2222",
            "0.0.1-canary.aaa1111"
        ));
    }

    #[test]
    fn pick_newest_canary_prerelease() {
        let list = vec![
            release("v0.0.1", false),
            release("v0.0.1-canary.bbbbbbb", true),
            release("v0.0.1-canary.aaaaaaa", true),
            release("canary", true),
        ];
        let picked = pick_canary(&list).expect("canary");
        assert_eq!(picked.tag, "v0.0.1-canary.bbbbbbb");
        assert_eq!(picked.version, "0.0.1-canary.bbbbbbb");
    }

    #[test]
    fn pick_stable_skips_prerelease() {
        assert!(pick_stable(&release("v0.0.1-canary.abc", true)).is_none());
        let picked = pick_stable(&release("v0.0.1", false)).expect("stable");
        assert_eq!(picked.version, "0.0.1");
    }

    #[test]
    fn release_url_allowlist() {
        assert!(is_allowed_release_url(
            "https://github.com/Questory-Labs/qMonitor/releases/tag/v0.0.1"
        ));
        assert!(is_allowed_release_url(
            "https://github.com/Questory-Labs/qMonitor/releases"
        ));
        assert!(!is_allowed_release_url(
            "https://github.com/evil/qMonitor/releases/tag/v0.0.1"
        ));
        assert!(!is_allowed_release_url("https://example.com/releases"));
        assert!(!is_allowed_release_url("file:///tmp/x"));
    }

    #[test]
    fn dismissed_tag_is_silent() {
        let pending = PendingUpdate {
            tag: "v0.0.2".into(),
            version: "0.0.2".into(),
            html_url: format!("https://github.com{RELEASES_PATH}/tag/v0.0.2"),
        };
        assert!(filter_pending(
            Some(&pending),
            UpdateChannel::Stable,
            "0.0.1",
            Some("v0.0.2")
        )
        .is_none());
        assert!(filter_pending(
            Some(&pending),
            UpdateChannel::Stable,
            "0.0.1",
            Some("v0.0.1")
        )
        .is_some());
    }

    #[test]
    fn skip_network_window() {
        let cache = UpdateCheckCache {
            last_checked_at: 1_000,
            last_channel: Some(UpdateChannel::Stable),
            ..Default::default()
        };
        assert!(should_skip_network(
            &cache,
            UpdateChannel::Stable,
            false,
            1_000 + CHECK_INTERVAL_SECS - 1
        ));
        assert!(!should_skip_network(
            &cache,
            UpdateChannel::Stable,
            true,
            1_000 + 1
        ));
        assert!(!should_skip_network(
            &cache,
            UpdateChannel::Canary,
            false,
            1_000 + 1
        ));
        assert!(!should_skip_network(
            &cache,
            UpdateChannel::Stable,
            false,
            1_000 + CHECK_INTERVAL_SECS
        ));
    }

    #[tokio::test]
    async fn cache_skip_and_force_refetch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("update-check.json");
        let pending = PendingUpdate {
            tag: "v0.0.2".into(),
            version: "0.0.2".into(),
            html_url: format!("https://github.com{RELEASES_PATH}/tag/v0.0.2"),
        };

        let first = check_at(
            UpdateChannel::Stable,
            false,
            "0.0.1",
            &path,
            1_000,
            |_ch| {
                let p = pending.clone();
                async move { Ok(Some(p)) }
            },
        )
        .await
        .unwrap();
        assert_eq!(first.as_ref().map(|p| p.tag.as_str()), Some("v0.0.2"));

        let skipped = check_at(
            UpdateChannel::Stable,
            false,
            "0.0.1",
            &path,
            1_000 + 60,
            |_ch| async { Err("should not hit network".into()) },
        )
        .await
        .unwrap();
        assert_eq!(skipped.as_ref().map(|p| p.tag.as_str()), Some("v0.0.2"));

        let newer = PendingUpdate {
            tag: "v0.0.3".into(),
            version: "0.0.3".into(),
            html_url: format!("https://github.com{RELEASES_PATH}/tag/v0.0.3"),
        };
        let forced = check_at(
            UpdateChannel::Stable,
            true,
            "0.0.1",
            &path,
            1_000 + 60,
            |_ch| {
                let p = newer.clone();
                async move { Ok(Some(p)) }
            },
        )
        .await
        .unwrap();
        assert_eq!(forced.as_ref().map(|p| p.tag.as_str()), Some("v0.0.3"));
    }

    #[test]
    fn github_list_fixture() {
        let raw = r#"[
            {"tag_name":"v0.0.2","html_url":"https://github.com/Questory-Labs/qMonitor/releases/tag/v0.0.2","prerelease":false},
            {"tag_name":"v0.0.2-canary.abc1234","html_url":"https://github.com/Questory-Labs/qMonitor/releases/tag/v0.0.2-canary.abc1234","prerelease":true},
            {"tag_name":"v0.0.1","html_url":"https://github.com/Questory-Labs/qMonitor/releases/tag/v0.0.1","prerelease":false}
        ]"#;
        let list: Vec<GhRelease> = serde_json::from_str(raw).unwrap();
        assert_eq!(pick_canary(&list).unwrap().tag, "v0.0.2-canary.abc1234");
        assert_eq!(pick_stable(&list[0]).unwrap().tag, "v0.0.2");
    }
}
