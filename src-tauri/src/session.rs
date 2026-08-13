//! Session state machine: open on identity appear, end on disappear, push pending.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;
use tokio::sync::RwLock;

use crate::auth;
use crate::config::AppConfig;
use crate::db::{SessionRow, TursoDb};
use crate::detect::{foreground_pid, primary_identity, snapshot_processes};
use crate::identity::detectable::{self, DetectableCatalog, DETECTABLE_MAX_AGE};
use crate::identity::resolver::{parse_exe_input, IdentityPipeline, UserMapping};
use crate::identity::{ManualGame, PendingDetection, TrackableGame};
use crate::identity::GameIdentity;
use crate::push::WebhookClient;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub turso_ok: bool,
    pub pending_count: i64,
    pub last_error: Option<String>,
    pub active_title: Option<String>,
    pub webhook_configured: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HomeState {
    pub sync: SyncStatus,
    pub active: Option<SessionRow>,
    pub history: Vec<SessionRow>,
    pub pending_detections: Vec<PendingDetection>,
}

pub struct AppState {
    pub config: RwLock<AppConfig>,
    pub db: RwLock<Option<Arc<TursoDb>>>,
    pub pipeline: RwLock<IdentityPipeline>,
    pub active_identity_id: RwLock<Option<String>>,
    pub pending_detections: RwLock<Vec<PendingDetection>>,
    pub last_error: RwLock<Option<String>>,
    pub webhook: WebhookClient,
}

impl AppState {
    pub fn new() -> Self {
        let config = AppConfig::load();
        let steam = config
            .steam_path_override
            .as_ref()
            .map(PathBuf::from);
        let catalog = config
            .catalog_path
            .as_ref()
            .map(PathBuf::from)
            .or_else(|| {
                let p = PathBuf::from("catalogs/games.example.json");
                if p.exists() {
                    Some(p)
                } else {
                    None
                }
            });
        let pipeline = IdentityPipeline::new(
            steam.as_deref(),
            catalog.as_deref(),
            Default::default(),
        );
        Self {
            config: RwLock::new(config),
            db: RwLock::new(None),
            pipeline: RwLock::new(pipeline),
            active_identity_id: RwLock::new(None),
            pending_detections: RwLock::new(Vec::new()),
            last_error: RwLock::new(None),
            webhook: WebhookClient::default(),
        }
    }

    pub async fn connect_db(&self) -> Result<(), String> {
        const ATTEMPTS: u32 = 8;
        let mut last_err = String::from("db connect failed");
        for attempt in 0..ATTEMPTS {
            match self.connect_db_once().await {
                Ok(()) => {
                    // Clear a prior connect failure once we're healthy again.
                    let mut err = self.last_error.write().await;
                    if err
                        .as_ref()
                        .is_some_and(|e| e.starts_with("db connect:") || e.starts_with("db open:"))
                    {
                        *err = None;
                    }
                    return Ok(());
                }
                Err(e) => {
                    last_err = e;
                    if attempt + 1 < ATTEMPTS {
                        let backoff_ms = 40u64.saturating_mul(2u64.pow(attempt.min(4)));
                        tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                    }
                }
            }
        }
        let msg = format!("db connect: {last_err}");
        tracing::error!(%msg, attempts = ATTEMPTS, "local database unavailable");
        *self.last_error.write().await = Some(msg.clone());
        Err(msg)
    }

    async fn connect_db_once(&self) -> Result<(), String> {
        let cfg = self.config.read().await.clone();
        let path = cfg.resolved_db_path();
        let db = TursoDb::open(&path).await.map_err(|e| format!("db open: {e}"))?;
        db.ping().await.map_err(|e| format!("db ping: {e}"))?;
        self.load_pipeline_prefs(&db).await;
        *self.db.write().await = Some(Arc::new(db));
        Ok(())
    }

    /// Open (or reopen) the local DB if missing or unresponsive.
    pub async fn ensure_db(&self) -> Result<Arc<TursoDb>, String> {
        if let Some(db) = self.db.read().await.clone() {
            if db.ping().await.is_ok() {
                return Ok(db);
            }
            tracing::warn!("local database ping failed; reconnecting");
            *self.db.write().await = None;
        }
        self.connect_db().await?;
        self.db
            .read()
            .await
            .clone()
            .ok_or_else(|| "db connect succeeded but handle missing".into())
    }

    async fn load_pipeline_prefs(&self, db: &TursoDb) {
        let mappings = db.list_mappings().await.unwrap_or_default();
        let ignored = db.list_ignored().await.unwrap_or_default();
        let manuals = db.list_manual_games().await.unwrap_or_default();
        let mut pipe = self.pipeline.write().await;
        pipe.user_mappings = mappings
            .into_iter()
            .map(|m| (m.fingerprint.clone(), m))
            .collect();
        pipe.ignored_identities = ignored.into_iter().map(|i| i.identity_id).collect();
        pipe.manual_games = manuals;
    }

    pub async fn reload_pipeline(&self) {
        let cfg = self.config.read().await.clone();
        let steam = cfg.steam_path_override.as_ref().map(PathBuf::from);
        let catalog = cfg.catalog_path.as_ref().map(PathBuf::from);
        let (mappings, ignored, manuals): (
            std::collections::HashMap<String, UserMapping>,
            HashSet<String>,
            Vec<ManualGame>,
        ) = if let Some(db) = self.db.read().await.as_ref() {
            let mappings = db
                .list_mappings()
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|m| (m.fingerprint.clone(), m))
                .collect();
            let ignored = db
                .list_ignored()
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|i| i.identity_id)
                .collect();
            let manuals = db.list_manual_games().await.unwrap_or_default();
            (mappings, ignored, manuals)
        } else {
            (Default::default(), HashSet::new(), Vec::new())
        };
        // Preserve in-memory detectable if already loaded; otherwise load disk cache.
        let detectable = {
            let pipe = self.pipeline.read().await;
            if pipe.detectable.is_empty() {
                DetectableCatalog::load_from_disk()
            } else {
                pipe.detectable.clone()
            }
        };
        let mut pipe = IdentityPipeline::new(steam.as_deref(), catalog.as_deref(), mappings);
        pipe.detectable = detectable;
        pipe.ignored_identities = ignored;
        pipe.manual_games = manuals;
        *self.pipeline.write().await = pipe;
    }

    /// Fetch/refresh Discord detectable catalog and swap into the pipeline.
    pub async fn refresh_detectable(&self, force: bool) {
        let url = self.config.read().await.resolved_detectable_url();
        let catalog = if force {
            detectable::force_refresh(&url).await
        } else {
            detectable::ensure_fresh(&url, DETECTABLE_MAX_AGE).await
        };
        if catalog.is_empty() {
            tracing::warn!("detectable catalog empty after refresh");
            return;
        }
        tracing::info!(
            entries = catalog.len(),
            %url,
            "detectable catalog ready"
        );
        self.pipeline.write().await.detectable = catalog;
    }

    pub async fn tick(&self) {
        let processes = snapshot_processes();
        let (identities, pending) = {
            let pipe = self.pipeline.read().await;
            pipe.resolve_running(&processes)
        };
        *self.pending_detections.write().await = pending;

        let fg = foreground_pid();
        let primary = primary_identity(&identities, fg, &processes).cloned();

        let db = match self.ensure_db().await {
            Ok(db) => db,
            Err(e) => {
                *self.last_error.write().await = Some(e);
                return;
            }
        };

        let prev = self.active_identity_id.read().await.clone();
        if let Err(e) = reconcile_active_sessions(&db, primary.as_ref(), prev.as_deref()).await {
            *self.last_error.write().await = Some(e);
        }
        *self.active_identity_id.write().await = primary.as_ref().map(|i| i.id.clone());

        // Push due sessions
        self.flush_pushes(&db).await;

        // Periodic purge
        let days = self.config.read().await.retention_acked_days;
        let _ = db.purge_synced(days).await;
    }

    async fn flush_pushes(&self, db: &TursoDb) {
        let cfg = self.config.read().await.clone();
        let Some(webhook_url) = cfg.webhook_url() else {
            return;
        };
        let Some(token) = auth::get_access_token(&cfg) else {
            return;
        };
        let due = match db.list_due_pushes().await {
            Ok(d) => d,
            Err(e) => {
                *self.last_error.write().await = Some(e);
                return;
            }
        };
        for row in due {
            match self.webhook.push(&cfg, &webhook_url, &token, &row).await {
                Ok(()) => {
                    let _ = db.mark_synced(&row.id).await;
                }
                Err(e) => {
                    let _ = db
                        .mark_push_failed(&row.id, &e, row.retry_count + 1)
                        .await;
                    *self.last_error.write().await = Some(e);
                }
            }
        }
    }

    pub async fn home_state(&self) -> HomeState {
        let cfg = self.config.read().await.clone();
        let mut turso_ok = false;
        let mut pending_count = 0;
        let mut active = None;
        let mut history = Vec::new();
        match self.ensure_db().await {
            Ok(db) => {
                turso_ok = db.ping().await.is_ok();
                pending_count = db.count_pending().await.unwrap_or(0);
                active = db.get_active().await.ok().flatten();
                history = db.list_sessions(100).await.unwrap_or_default();
            }
            Err(e) => {
                *self.last_error.write().await = Some(e);
            }
        }
        let sync = SyncStatus {
            turso_ok,
            pending_count,
            last_error: self.last_error.read().await.clone(),
            active_title: active.as_ref().map(|a| a.title.clone()),
            webhook_configured: cfg.webhook_url().is_some()
                && auth::get_access_token(&cfg).is_some(),
        };
        HomeState {
            sync,
            active,
            history,
            pending_detections: self.pending_detections.read().await.clone(),
        }
    }

    pub async fn confirm_detection(&self, fingerprint: String, title: String) -> Result<(), String> {
        let identity_id = format!("user:{fingerprint}");
        let mapping = UserMapping {
            fingerprint: fingerprint.clone(),
            title: title.clone(),
            identity_id: identity_id.clone(),
        };
        if let Some(db) = self.db.read().await.as_ref() {
            db.upsert_mapping(&mapping.fingerprint, &mapping.title, &mapping.identity_id)
                .await?;
            // Confirming implies tracking — clear any prior ignore.
            let _ = db.remove_ignored(&identity_id).await;
        }
        {
            let mut pipe = self.pipeline.write().await;
            pipe.ignored_identities.remove(&identity_id);
            pipe.user_mappings.insert(fingerprint, mapping);
        }
        Ok(())
    }

    pub async fn ignore_game(&self, identity_id: String, title: String) -> Result<(), String> {
        if let Some(db) = self.db.read().await.as_ref() {
            db.upsert_ignored(&identity_id, &title).await?;
            // Don't track ⇒ drop any in-flight session without pushing.
            let actives = db.list_active().await.unwrap_or_default();
            let discard_ids: Vec<String> = actives
                .into_iter()
                .filter(|s| s.identity_id == identity_id)
                .map(|s| s.id)
                .collect();
            if !discard_ids.is_empty() {
                let _ = db.discard_active_sessions(&discard_ids).await;
            }
            let prev = self.active_identity_id.read().await.clone();
            if prev.as_deref() == Some(identity_id.as_str()) {
                *self.active_identity_id.write().await = None;
            }
        }
        self.pipeline
            .write()
            .await
            .ignored_identities
            .insert(identity_id);
        Ok(())
    }

    pub async fn unignore_game(&self, identity_id: String) -> Result<(), String> {
        if let Some(db) = self.db.read().await.as_ref() {
            db.remove_ignored(&identity_id).await?;
        }
        self.pipeline
            .write()
            .await
            .ignored_identities
            .remove(&identity_id);
        Ok(())
    }

    pub async fn add_manual_game(
        &self,
        title: String,
        exe_path: String,
        steam_app_id: Option<u32>,
    ) -> Result<ManualGame, String> {
        let title = title.trim().to_string();
        if title.is_empty() {
            return Err("Title is required".into());
        }
        let (exe_name, path_hint) = parse_exe_input(&exe_path)?;
        let id = uuid::Uuid::new_v4().to_string();
        let game = ManualGame {
            id,
            title: title.clone(),
            exe_name,
            path_hint,
            steam_app_id,
        };
        let identity_id = steam_app_id
            .map(|sid| format!("steam:{sid}"))
            .unwrap_or_else(|| format!("manual:{}", game.id));
        if let Some(db) = self.db.read().await.as_ref() {
            db.upsert_manual_game(&game).await?;
            let _ = db.remove_ignored(&identity_id).await;
        }
        {
            let mut pipe = self.pipeline.write().await;
            pipe.ignored_identities.remove(&identity_id);
            // Replace existing entry with same exe+hint if present.
            pipe.manual_games
                .retain(|g| !(g.exe_name.eq_ignore_ascii_case(&game.exe_name)
                    && g.path_hint == game.path_hint));
            pipe.manual_games.push(game.clone());
        }
        Ok(game)
    }
}

/// DB-authoritative session reconcile. Call every tick so process restarts cannot
/// leave orphan `active` rows or open duplicates.
pub async fn reconcile_active_sessions(
    db: &TursoDb,
    primary: Option<&GameIdentity>,
    prev_identity_id: Option<&str>,
) -> Result<(), String> {
    let actives = db.list_active().await?;

    match primary {
        Some(identity) => {
            // End (push) actives for other identities — real switch / leftover.
            for row in actives.iter().filter(|s| s.identity_id != identity.id) {
                db.end_session(&row.id).await?;
            }

            let mut same: Vec<_> = actives
                .into_iter()
                .filter(|s| s.identity_id == identity.id)
                .collect();
            // list_active is oldest-first, but sort defensively.
            same.sort_by_key(|s| s.started_at);

            if same.is_empty() {
                db.open_session(identity).await?;
            } else {
                // Keep oldest continuous session; discard restart duplicates.
                let discard_ids: Vec<String> = same.iter().skip(1).map(|s| s.id.clone()).collect();
                if !discard_ids.is_empty() {
                    db.discard_active_sessions(&discard_ids).await?;
                }
            }
        }
        None => {
            if prev_identity_id.is_some() {
                // Monitor was tracking something; game quit → end and push.
                for row in &actives {
                    db.end_session(&row.id).await?;
                }
            } else {
                // Cold start / restart with nothing running → discard orphans (no webhook spam).
                let ids: Vec<String> = actives.into_iter().map(|s| s.id).collect();
                if !ids.is_empty() {
                    tracing::warn!(
                        count = ids.len(),
                        "discarding orphaned active sessions after cold idle reconcile"
                    );
                    db.discard_active_sessions(&ids).await?;
                }
            }
        }
    }
    Ok(())
}

pub async fn list_trackable_games(state: &AppState) -> Vec<TrackableGame> {
    let ignored_titles: std::collections::HashMap<String, String> =
        if let Some(db) = state.db.read().await.as_ref() {
            db.list_ignored()
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|i| (i.identity_id, i.title))
                .collect()
        } else {
            Default::default()
        };

    let pipe = state.pipeline.read().await;
    let ignored = &pipe.ignored_identities;
    let mut games: Vec<TrackableGame> = pipe
        .steam
        .games
        .values()
        .map(|g| {
            let id = format!("steam:{}", g.app_id);
            TrackableGame {
                tracking_enabled: !ignored.contains(&id),
                id,
                title: g.title.clone(),
                steam_app_id: Some(g.app_id),
                source: "steam".into(),
            }
        })
        .collect();

    for mapping in pipe.user_mappings.values() {
        if games.iter().any(|g| g.id == mapping.identity_id) {
            continue;
        }
        games.push(TrackableGame {
            id: mapping.identity_id.clone(),
            title: mapping.title.clone(),
            steam_app_id: None,
            source: "user".into(),
            tracking_enabled: !ignored.contains(&mapping.identity_id),
        });
    }

    for manual in &pipe.manual_games {
        let id = manual
            .steam_app_id
            .map(|sid| format!("steam:{sid}"))
            .unwrap_or_else(|| format!("manual:{}", manual.id));
        let tracking_enabled = !ignored.contains(&id);
        if let Some(existing) = games.iter_mut().find(|g| g.id == id) {
            existing.source = "manual".into();
            existing.title = manual.title.clone();
            existing.tracking_enabled = tracking_enabled;
            continue;
        }
        games.push(TrackableGame {
            id,
            title: manual.title.clone(),
            steam_app_id: manual.steam_app_id,
            source: "manual".into(),
            tracking_enabled,
        });
    }

    for (id, title) in &ignored_titles {
        if games.iter().any(|g| &g.id == id) {
            continue;
        }
        games.push(TrackableGame {
            id: id.clone(),
            title: title.clone(),
            steam_app_id: None,
            source: "ignored".into(),
            tracking_enabled: false,
        });
    }

    // Refresh titles for ignored steam/user rows from DB when present.
    for g in &mut games {
        if let Some(title) = ignored_titles.get(&g.id) {
            if !g.tracking_enabled {
                g.title = title.clone();
            }
        }
    }

    games.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
    games
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{PushStatus, TursoDb};
    use crate::identity::{Confidence, GameIdentity};
    use chrono::{Duration, Utc};
    use tempfile::tempdir;

    fn identity(id: &str, title: &str) -> GameIdentity {
        GameIdentity {
            id: id.into(),
            title: title.into(),
            steam_app_id: None,
            exe: Some(format!("{title}.exe")),
            confidence: Confidence::High,
            source: "test".into(),
            fingerprint: None,
        }
    }

    #[tokio::test]
    async fn reconcile_keeps_oldest_discards_duplicates_when_still_playing() {
        let dir = tempdir().unwrap();
        let db = TursoDb::open(dir.path().join("r1.db")).await.unwrap();
        let apex = identity("steam:1172470", "Apex");
        let older = db
            .force_insert_active(&apex, Utc::now() - Duration::minutes(20))
            .await
            .unwrap();
        let newer = db
            .force_insert_active(&apex, Utc::now() - Duration::minutes(5))
            .await
            .unwrap();

        reconcile_active_sessions(&db, Some(&apex), None)
            .await
            .unwrap();

        let actives = db.list_active().await.unwrap();
        assert_eq!(actives.len(), 1);
        assert_eq!(actives[0].id, older.id);
        assert!(db.get_session(&newer.id).await.unwrap().is_none());
        assert!(db.list_due_pushes().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn reconcile_ends_other_identity_and_opens_current() {
        let dir = tempdir().unwrap();
        let db = TursoDb::open(dir.path().join("r2.db")).await.unwrap();
        let a = identity("steam:1", "A");
        let b = identity("steam:2", "B");
        let old = db.open_session(&a).await.unwrap();

        reconcile_active_sessions(&db, Some(&b), Some("steam:1"))
            .await
            .unwrap();

        let ended = db.get_session(&old.id).await.unwrap().unwrap();
        assert_eq!(ended.push_status, PushStatus::Pending);
        assert!(ended.ended_at.is_some());

        let actives = db.list_active().await.unwrap();
        assert_eq!(actives.len(), 1);
        assert_eq!(actives[0].identity_id, "steam:2");
    }

    #[tokio::test]
    async fn reconcile_cold_idle_discards_orphans_without_push() {
        let dir = tempdir().unwrap();
        let db = TursoDb::open(dir.path().join("r3.db")).await.unwrap();
        let apex = identity("steam:1172470", "Apex");
        db.force_insert_active(&apex, Utc::now() - Duration::hours(1))
            .await
            .unwrap();
        db.force_insert_active(&apex, Utc::now() - Duration::minutes(30))
            .await
            .unwrap();

        reconcile_active_sessions(&db, None, None).await.unwrap();

        assert!(db.list_active().await.unwrap().is_empty());
        assert!(db.list_due_pushes().await.unwrap().is_empty());
        assert!(db.list_sessions(10).await.unwrap().is_empty());
    }

    /// Game quit (`primary = None`) ends and queues push on that tick — no end-grace.
    #[tokio::test]
    async fn reconcile_warm_idle_ends_and_queues_push() {
        let dir = tempdir().unwrap();
        let db = TursoDb::open(dir.path().join("r4.db")).await.unwrap();
        let apex = identity("steam:1172470", "Apex");
        let row = db.open_session(&apex).await.unwrap();

        reconcile_active_sessions(&db, None, Some("steam:1172470"))
            .await
            .unwrap();

        assert!(db.list_active().await.unwrap().is_empty());
        let due = db.list_due_pushes().await.unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].id, row.id);
        assert_eq!(due[0].push_status, PushStatus::Pending);
    }

    #[tokio::test]
    async fn reconcile_opens_when_playing_with_no_active() {
        let dir = tempdir().unwrap();
        let db = TursoDb::open(dir.path().join("r5.db")).await.unwrap();
        let apex = identity("steam:1172470", "Apex");

        reconcile_active_sessions(&db, Some(&apex), None)
            .await
            .unwrap();

        let actives = db.list_active().await.unwrap();
        assert_eq!(actives.len(), 1);
        assert_eq!(actives[0].identity_id, "steam:1172470");
    }

    #[tokio::test]
    async fn ignore_discards_all_actives_without_push() {
        let state = AppState::new();
        let dir = tempdir().unwrap();
        let db = TursoDb::open(dir.path().join("ign.db")).await.unwrap();
        let apex = identity("steam:1172470", "Apex");
        let older = db
            .force_insert_active(&apex, Utc::now() - Duration::minutes(10))
            .await
            .unwrap();
        let newer = db
            .force_insert_active(&apex, Utc::now() - Duration::minutes(1))
            .await
            .unwrap();
        *state.db.write().await = Some(std::sync::Arc::new(db));
        *state.active_identity_id.write().await = Some("steam:1172470".into());

        state
            .ignore_game("steam:1172470".into(), "Apex".into())
            .await
            .unwrap();

        let db = state.db.read().await.clone().unwrap();
        assert!(db.list_active().await.unwrap().is_empty());
        assert!(db.get_session(&older.id).await.unwrap().is_none());
        assert!(db.get_session(&newer.id).await.unwrap().is_none());
        assert!(db.list_due_pushes().await.unwrap().is_empty());
        assert!(state.active_identity_id.read().await.is_none());
        assert!(state
            .pipeline
            .read()
            .await
            .ignored_identities
            .contains("steam:1172470"));
    }

    #[tokio::test]
    async fn ensure_db_opens_missing_handle() {
        let state = AppState::new();
        let dir = tempdir().unwrap();
        let path = dir.path().join("ensure.db");
        {
            let mut cfg = state.config.write().await;
            cfg.db_path = Some(path.to_string_lossy().to_string());
        }
        assert!(state.db.read().await.is_none());
        let db = state.ensure_db().await.expect("ensure");
        assert!(db.ping().await.is_ok());
        // Second call reuses the live handle.
        let db2 = state.ensure_db().await.expect("ensure again");
        assert!(db2.ping().await.is_ok());
    }
}
