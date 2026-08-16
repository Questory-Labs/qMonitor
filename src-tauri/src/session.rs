//! App state, pipeline prefs, and UI-facing home snapshot.

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tokio::sync::{oneshot, RwLock};

use crate::auth;
use crate::config::AppConfig;
use crate::db::{PushStatus, SessionRow, TursoDb};
use crate::health::RuntimeHealth;
use crate::identity::detectable::{self, DetectableCatalog, DETECTABLE_MAX_AGE};
use crate::identity::resolver::{parse_exe_input, IdentityPipeline, UserMapping};
use crate::identity::{ManualGame, PendingDetection, TrackableGame};
use crate::live_session::LiveSession;
use crate::persist::{DbView, PersistCmd};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub turso_ok: bool,
    pub pending_count: i64,
    pub last_error: Option<String>,
    pub active_title: Option<String>,
    pub webhook_configured: bool,
    pub last_tick_at: Option<String>,
    pub loop_alive: bool,
    pub db_reconnects: u64,
    pub detect_timeouts: u64,
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
    /// Test-only injected DB. Production persist owns the connection.
    pub db: RwLock<Option<Arc<TursoDb>>>,
    pub pipeline: RwLock<IdentityPipeline>,
    pub live: RwLock<LiveSession>,
    pub db_view: RwLock<DbView>,
    pub health: RwLock<RuntimeHealth>,
    pub persist_tx: Mutex<Option<tokio::sync::mpsc::Sender<PersistCmd>>>,
    pub ignored_titles: RwLock<HashMap<String, String>>,
    pub pending_detections: RwLock<Vec<PendingDetection>>,
    pub last_error: RwLock<Option<String>>,
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
            live: RwLock::new(LiveSession::default()),
            db_view: RwLock::new(DbView::default()),
            health: RwLock::new(RuntimeHealth::default()),
            persist_tx: Mutex::new(None),
            ignored_titles: RwLock::new(HashMap::new()),
            pending_detections: RwLock::new(Vec::new()),
            last_error: RwLock::new(None),
        }
    }

    pub async fn connect_db(&self) -> Result<(), String> {
        self.call_persist(|reply| PersistCmd::EnsureOpen { reply })
            .await
            .map(|_| ())
    }

    async fn call_persist<T>(
        &self,
        make: impl FnOnce(oneshot::Sender<Result<T, String>>) -> PersistCmd,
    ) -> Result<T, String> {
        let tx = self
            .persist_tx
            .lock()
            .map_err(|e| e.to_string())?
            .clone()
            .ok_or_else(|| "persist offline".to_string())?;
        let (reply, rx) = oneshot::channel();
        tx.send(make(reply))
            .await
            .map_err(|_| "persist offline".to_string())?;
        tokio::time::timeout(Duration::from_secs(8), rx)
            .await
            .map_err(|_| "persist timeout".to_string())?
            .map_err(|_| "persist dropped".to_string())?
    }

    async fn persist_or_db<F, Fut>(
        &self,
        make: impl FnOnce(oneshot::Sender<Result<(), String>>) -> PersistCmd,
        fallback: F,
    ) -> Result<(), String>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<(), String>>,
    {
        if self.persist_tx.lock().ok().and_then(|g| g.clone()).is_some() {
            drop(fallback);
            self.call_persist(make).await
        } else {
            fallback().await
        }
    }

    pub async fn reload_pipeline(&self) {
        let cfg = self.config.read().await.clone();
        let steam = cfg.steam_path_override.as_ref().map(PathBuf::from);
        let catalog = cfg.catalog_path.as_ref().map(PathBuf::from);
        let (mappings, ignored, manuals) = {
            let pipe = self.pipeline.read().await;
            (
                pipe.user_mappings.clone(),
                pipe.ignored_identities.clone(),
                pipe.manual_games.clone(),
            )
        };
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

    pub async fn home_state(&self) -> HomeState {
        let cfg = self.config.read().await.clone();
        let poll = cfg.poll_interval_secs.max(1);
        let view = self.db_view.read().await.clone();
        let live = self.live.read().await.clone();
        let health = self.health.read().await.clone();
        let last_error = health
            .last_error
            .clone()
            .or(self.last_error.read().await.clone());
        let active = overlay_active(&live, view.active.clone());
        let sync = SyncStatus {
            turso_ok: view.turso_ok,
            pending_count: view.pending_count,
            last_error,
            active_title: active.as_ref().map(|a| a.title.clone()),
            webhook_configured: cfg.webhook_url().is_some()
                && auth::get_access_token(&cfg).is_some(),
            last_tick_at: health.last_detect_at.map(|t| t.to_rfc3339()),
            loop_alive: health.loop_alive(poll),
            db_reconnects: health.db_reconnects,
            detect_timeouts: health.detect_timeouts,
        };
        HomeState {
            sync,
            active,
            history: view.history,
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
        {
            let mut pipe = self.pipeline.write().await;
            pipe.ignored_identities.remove(&identity_id);
            pipe.user_mappings.insert(fingerprint.clone(), mapping);
        }
        self.persist_or_db(
            |reply| PersistCmd::Confirm {
                fingerprint: fingerprint.clone(),
                title: title.clone(),
                reply,
            },
            || async {
                if let Some(db) = self.db.read().await.as_ref() {
                    db.upsert_mapping(&fingerprint, &title, &identity_id)
                        .await?;
                    let _ = db.remove_ignored(&identity_id).await;
                }
                Ok(())
            },
        )
        .await?;
        Ok(())
    }

    pub async fn ignore_game(&self, identity_id: String, title: String) -> Result<(), String> {
        {
            let mut live = self.live.write().await;
            if live.identity_id() == Some(identity_id.as_str()) {
                live.clear_identity();
            }
        }
        self.pipeline
            .write()
            .await
            .ignored_identities
            .insert(identity_id.clone());
        self.ignored_titles
            .write()
            .await
            .insert(identity_id.clone(), title.clone());
        self.persist_or_db(
            |reply| PersistCmd::Ignore {
                identity_id: identity_id.clone(),
                title: title.clone(),
                reply,
            },
            || async {
                if let Some(db) = self.db.read().await.as_ref() {
                    db.upsert_ignored(&identity_id, &title).await?;
                    let actives = db.list_active().await.unwrap_or_default();
                    let discard_ids: Vec<String> = actives
                        .into_iter()
                        .filter(|s| s.identity_id == identity_id)
                        .map(|s| s.id)
                        .collect();
                    if !discard_ids.is_empty() {
                        let _ = db.discard_active_sessions(&discard_ids).await;
                    }
                }
                Ok(())
            },
        )
        .await?;
        Ok(())
    }

    pub async fn unignore_game(&self, identity_id: String) -> Result<(), String> {
        self.pipeline
            .write()
            .await
            .ignored_identities
            .remove(&identity_id);
        self.ignored_titles.write().await.remove(&identity_id);
        self.persist_or_db(
            |reply| PersistCmd::Unignore {
                identity_id: identity_id.clone(),
                reply,
            },
            || async {
                if let Some(db) = self.db.read().await.as_ref() {
                    db.remove_ignored(&identity_id).await?;
                }
                Ok(())
            },
        )
        .await?;
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
        {
            let mut pipe = self.pipeline.write().await;
            pipe.ignored_identities.remove(&identity_id);
            pipe.manual_games.retain(|g| {
                !(g.exe_name.eq_ignore_ascii_case(&game.exe_name) && g.path_hint == game.path_hint)
            });
            pipe.manual_games.push(game.clone());
        }
        self.persist_or_db(
            |reply| PersistCmd::AddManual {
                game: game.clone(),
                identity_id: identity_id.clone(),
                reply,
            },
            || async {
                if let Some(db) = self.db.read().await.as_ref() {
                    db.upsert_manual_game(&game).await?;
                    let _ = db.remove_ignored(&identity_id).await;
                }
                Ok(())
            },
        )
        .await?;
        Ok(game)
    }
}

fn overlay_active(live: &LiveSession, db_active: Option<SessionRow>) -> Option<SessionRow> {
    let Some(identity) = live.identity.as_ref() else {
        return db_active;
    };
    if db_active
        .as_ref()
        .is_some_and(|a| a.identity_id == identity.id)
    {
        return db_active;
    }
    Some(SessionRow {
        id: live
            .db_session_id
            .clone()
            .unwrap_or_else(|| format!("live:{}", identity.id)),
        identity_id: identity.id.clone(),
        title: identity.title.clone(),
        steam_app_id: identity.steam_app_id,
        exe: identity.exe.clone(),
        source: identity.source.clone(),
        started_at: live.started_at.unwrap_or_else(chrono::Utc::now),
        ended_at: None,
        duration_secs: None,
        push_status: PushStatus::Active,
        acked_at: None,
        retry_count: 0,
        next_retry_at: None,
        last_error: None,
    })
}


pub async fn list_trackable_games(state: &AppState) -> Vec<TrackableGame> {
    let ignored_titles = state.ignored_titles.read().await.clone();

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
    use crate::db::TursoDb;
    use crate::identity::{Confidence, GameIdentity};
    use crate::live_session::{DetectSample, LiveSession};
    use crate::persist::flush_live;
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
        state.live.write().await.identity = Some(apex);

        state
            .ignore_game("steam:1172470".into(), "Apex".into())
            .await
            .unwrap();

        let db = state.db.read().await.clone().unwrap();
        assert!(db.list_active().await.unwrap().is_empty());
        assert!(db.get_session(&older.id).await.unwrap().is_none());
        assert!(db.get_session(&newer.id).await.unwrap().is_none());
        assert!(db.list_due_pushes().await.unwrap().is_empty());
        assert!(state.live.read().await.identity.is_none());
        assert!(state
            .pipeline
            .read()
            .await
            .ignored_identities
            .contains("steam:1172470"));
    }

    #[tokio::test]
    async fn detect_updates_live_when_persist_is_offline() {
        let state = AppState::new();
        let t0 = Utc::now();
        let sample = DetectSample {
            observed_at: t0,
            primary: Some(identity("steam:1", "RL")),
        };
        state.live.write().await.apply(&sample);
        let home = state.home_state().await;
        assert_eq!(home.active.as_ref().map(|a| a.identity_id.as_str()), Some("steam:1"));
        assert!(!home.sync.turso_ok);
        state.live.write().await.apply(&DetectSample {
            observed_at: t0 + Duration::seconds(9),
            primary: None,
        });
        assert_eq!(state.live.read().await.pending_ends.len(), 1);
    }

    #[tokio::test]
    async fn overlay_shows_live_when_db_view_empty() {
        let mut live = LiveSession::default();
        live.apply(&DetectSample {
            observed_at: Utc::now(),
            primary: Some(identity("steam:1", "RL")),
        });
        let row = overlay_active(&live, None).unwrap();
        assert_eq!(row.identity_id, "steam:1");
        assert_eq!(row.push_status, crate::db::PushStatus::Active);
    }

    #[tokio::test]
    async fn flush_live_opens_when_playing() {
        let dir = tempdir().unwrap();
        let db = TursoDb::open(dir.path().join("open.db")).await.unwrap();
        let mut live = LiveSession::default();
        live.apply(&DetectSample {
            observed_at: Utc::now(),
            primary: Some(identity("steam:1172470", "Apex")),
        });
        flush_live(&db, &mut live).await.unwrap();
        let actives = db.list_active().await.unwrap();
        assert_eq!(actives.len(), 1);
        assert_eq!(actives[0].identity_id, "steam:1172470");
    }
}
