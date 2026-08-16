//! Persist worker: exclusive owner of the Turso connection.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::sync::{mpsc, watch};
use tokio::time::timeout;

use crate::db::{SessionRow, TursoDb};
use crate::identity::{GameIdentity, ManualGame};
use crate::live_session::{DetectSample, PendingEnd, SLEEP_SPLIT};
#[cfg(test)]
use crate::live_session::LiveSession;
use crate::session::AppState;

pub const DB_OP_TIMEOUT: Duration = Duration::from_secs(2);
pub const DB_OPEN_TIMEOUT: Duration = Duration::from_secs(5);
pub const RECONNECT_BACKOFF: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub enum PersistError {
    Poison(String),
    PathChanged,
}

pub enum PersistCmd {
    Confirm {
        fingerprint: String,
        title: String,
        reply: tokio::sync::oneshot::Sender<Result<(), String>>,
    },
    Ignore {
        identity_id: String,
        title: String,
        reply: tokio::sync::oneshot::Sender<Result<(), String>>,
    },
    Unignore {
        identity_id: String,
        reply: tokio::sync::oneshot::Sender<Result<(), String>>,
    },
    AddManual {
        game: ManualGame,
        identity_id: String,
        reply: tokio::sync::oneshot::Sender<Result<(), String>>,
    },
    EnsureOpen {
        reply: tokio::sync::oneshot::Sender<Result<String, String>>,
    },
}

#[derive(Debug, Clone, Default)]
pub struct DbView {
    pub turso_ok: bool,
    pub pending_count: i64,
    pub active: Option<SessionRow>,
    pub history: Vec<SessionRow>,
}

pub async fn timed<T>(fut: impl Future<Output = Result<T, String>>) -> Result<T, PersistError> {
    match timeout(DB_OP_TIMEOUT, fut).await {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(PersistError::Poison(e)),
        Err(_) => Err(PersistError::Poison("db op timeout".into())),
    }
}

fn persist_reply(res: &Result<(), PersistError>) -> Result<(), String> {
    match res {
        Ok(()) => Ok(()),
        Err(PersistError::Poison(e)) => Err(e.clone()),
        Err(PersistError::PathChanged) => Err("db path changed".into()),
    }
}

/// Flush pending ends + current tracking onto the DB. DB start wins when a row exists.
#[cfg(test)]
pub async fn flush_live(db: &TursoDb, live: &mut LiveSession) -> Result<(), String> {
    let pending = std::mem::take(&mut live.pending_ends);
    for end in pending {
        write_pending_end(db, &end).await?;
    }
    if let Some((id, started)) = reconcile_live(
        db,
        live.identity.as_ref(),
        live.started_at,
        live.last_seen_at,
        live.last_tick_at,
    )
    .await?
    {
        live.db_session_id = Some(id);
        live.started_at = Some(started);
    }
    Ok(())
}

async fn write_pending_end(db: &TursoDb, end: &PendingEnd) -> Result<(), String> {
    if let Some(id) = &end.db_session_id {
        if let Some(row) = db.get_session(id).await? {
            if row.push_status == crate::db::PushStatus::Active {
                db.end_session_at(id, end.ended_at).await?;
                return Ok(());
            }
        }
    }
    let actives = db.list_active().await?;
    let mut found = false;
    for row in actives.iter().filter(|s| s.identity_id == end.identity.id) {
        db.end_session_at(&row.id, end.ended_at).await?;
        found = true;
    }
    if !found {
        let opened = db
            .open_session_at(&end.identity, end.started_at)
            .await?;
        db.end_session_at(&opened.id, end.ended_at).await?;
    }
    Ok(())
}

/// Open-or-reuse the tracked identity, end other actives, discard duplicate actives,
/// and cap orphan endings at `SLEEP_SPLIT`.
async fn reconcile_live(
    db: &TursoDb,
    identity: Option<&GameIdentity>,
    started_at: Option<DateTime<Utc>>,
    last_seen_at: Option<DateTime<Utc>>,
    last_tick_at: Option<DateTime<Utc>>,
) -> Result<Option<(String, DateTime<Utc>)>, String> {
    if let Some(identity) = identity {
        let started = started_at.unwrap_or_else(Utc::now);
        let row = db.open_session_at(identity, started).await?;
        let mut keep_id = row.id.clone();
        let mut keep_started = row.started_at;

        let actives = db.list_active().await?;
        let last_seen = last_seen_at.unwrap_or(row.started_at);
        for other in actives.iter().filter(|s| s.identity_id != identity.id) {
            db.end_session_at(&other.id, last_seen).await?;
        }

        let mut same: Vec<_> = db
            .list_active()
            .await?
            .into_iter()
            .filter(|s| s.identity_id == identity.id)
            .collect();
        same.sort_by_key(|s| s.started_at);
        if same.len() > 1 {
            let discard: Vec<String> = same.iter().skip(1).map(|s| s.id.clone()).collect();
            db.discard_active_sessions(&discard).await?;
        }
        if let Some(keep) = same.first() {
            keep_id = keep.id.clone();
            keep_started = keep.started_at;
        }
        Ok(Some((keep_id, keep_started)))
    } else if last_tick_at.is_some() {
        for row in db.list_active().await? {
            let cap = row.started_at + SLEEP_SPLIT;
            let ended = Utc::now().min(cap);
            db.end_session_at(&row.id, ended).await?;
        }
        Ok(None)
    } else {
        Ok(None)
    }
}

async fn refresh_view(db: &TursoDb) -> Result<DbView, String> {
    Ok(DbView {
        turso_ok: true,
        pending_count: db.count_pending().await?,
        active: db.get_active().await?,
        history: db.list_sessions(100).await?,
    })
}

async fn load_prefs(db: &TursoDb, state: &AppState) -> Result<(), String> {
    let mappings = db.list_mappings().await?;
    let ignored = db.list_ignored().await?;
    let manuals = db.list_manual_games().await?;
    {
        let mut titles = state.ignored_titles.write().await;
        *titles = ignored
            .iter()
            .map(|i| (i.identity_id.clone(), i.title.clone()))
            .collect();
    }
    let mut pipe = state.pipeline.write().await;
    pipe.user_mappings = mappings
        .into_iter()
        .map(|m| (m.fingerprint.clone(), m))
        .collect();
    pipe.ignored_identities = ignored.into_iter().map(|i| i.identity_id).collect();
    pipe.manual_games = manuals;
    Ok(())
}

pub async fn run_persist(
    state: Arc<AppState>,
    mut sample_rx: watch::Receiver<DetectSample>,
    mut cmd_rx: mpsc::Receiver<PersistCmd>,
    push_tx: mpsc::Sender<SessionRow>,
    mut push_result_rx: mpsc::Receiver<(String, Result<(), String>)>,
) {
    let mut generation: u64 = 0;
    loop {
        let path = state.config.read().await.resolved_db_path();
        generation += 1;
        if generation > 1 {
            let mut h = state.health.write().await;
            h.db_reconnects += 1;
            h.db_ok = false;
            h.db_generation = generation;
        }
        match open_db(&path).await {
            Ok(db) => {
                tracing::info!(gen = generation, path = %path.display(), "persist db open");
                {
                    let mut h = state.health.write().await;
                    h.db_ok = true;
                    h.db_generation = generation;
                    h.last_persist_at = Some(Utc::now());
                }
                if let Err(e) = load_prefs(&db, &state).await {
                    tracing::warn!(%e, "load pipeline prefs failed");
                }
                match persist_loop(
                    &state,
                    &db,
                    &path,
                    &mut sample_rx,
                    &mut cmd_rx,
                    &push_tx,
                    &mut push_result_rx,
                )
                .await
                {
                    Ok(()) => return,
                    Err(PersistError::PathChanged) => {
                        tracing::info!(gen = generation, "db path changed; reopening");
                    }
                    Err(PersistError::Poison(e)) => {
                        tracing::warn!(%e, gen = generation, "persist poisoned; dropping connection");
                        *state.last_error.write().await = Some(e.clone());
                        state.health.write().await.db_ok = false;
                        state.health.write().await.last_error = Some(e);
                        *state.db_view.write().await = DbView::default();
                    }
                }
                drop(db);
            }
            Err(e) => {
                tracing::error!(%e, "persist db open failed");
                *state.last_error.write().await = Some(e.clone());
                state.health.write().await.db_ok = false;
                state.health.write().await.last_error = Some(e);
                *state.db_view.write().await = DbView::default();
            }
        }
        tokio::time::sleep(RECONNECT_BACKOFF).await;
    }
}

async fn open_db(path: &PathBuf) -> Result<TursoDb, String> {
    timeout(DB_OPEN_TIMEOUT, TursoDb::open(path))
        .await
        .map_err(|_| "db open timeout".to_string())?
}

async fn persist_loop(
    state: &AppState,
    db: &TursoDb,
    opened_path: &Path,
    sample_rx: &mut watch::Receiver<DetectSample>,
    cmd_rx: &mut mpsc::Receiver<PersistCmd>,
    push_tx: &mpsc::Sender<SessionRow>,
    push_result_rx: &mut mpsc::Receiver<(String, Result<(), String>)>,
) -> Result<(), PersistError> {
    let mut purge_ticks: u32 = 0;
    loop {
        tokio::select! {
            biased;
            r = sample_rx.changed() => {
                if r.is_err() {
                    return Ok(());
                }
                apply_sample(state, db, push_tx).await?;
            }
            Some(cmd) = cmd_rx.recv() => {
                handle_cmd(state, db, opened_path, cmd).await?;
            }
            Some((id, result)) = push_result_rx.recv() => {
                match result {
                    Ok(()) => timed(db.mark_synced(&id)).await?,
                    Err(e) => {
                        let retry = match timed(db.get_session(&id)).await {
                            Ok(Some(r)) => r.retry_count + 1,
                            _ => 1,
                        };
                        let _ = timed(db.mark_push_failed(&id, &e, retry)).await;
                        *state.last_error.write().await = Some(e.clone());
                    }
                }
                refresh_and_store(state, db).await?;
            }
            _ = tokio::time::sleep(Duration::from_secs(2)) => {
                apply_sample(state, db, push_tx).await?;
                purge_ticks += 1;
                if purge_ticks >= 30 {
                    purge_ticks = 0;
                    let days = state.config.read().await.retention_acked_days;
                    let _ = timed(db.purge_synced(days)).await;
                }
            }
        }
    }
}

async fn apply_sample(
    state: &AppState,
    db: &TursoDb,
    push_tx: &mpsc::Sender<SessionRow>,
) -> Result<(), PersistError> {
    let pending = {
        let mut live = state.live.write().await;
        std::mem::take(&mut live.pending_ends)
    };
    let mut pending = pending.into_iter();
    while let Some(end) = pending.next() {
        if let Err(e) = timed(write_pending_end(db, &end)).await {
            let mut live = state.live.write().await;
            let mut rest: Vec<_> = std::iter::once(end).chain(pending).collect();
            rest.append(&mut live.pending_ends);
            live.pending_ends = rest;
            return Err(e);
        }
    }

    let snapshot = state.live.read().await.clone();
    let keep = timed(reconcile_live(
        db,
        snapshot.identity.as_ref(),
        snapshot.started_at,
        snapshot.last_seen_at,
        snapshot.last_tick_at,
    ))
    .await?;
    if let (Some(identity), Some((id, started))) = (snapshot.identity.as_ref(), keep) {
        let mut live = state.live.write().await;
        if live.identity_id() == Some(identity.id.as_str()) {
            live.db_session_id = Some(id);
            live.started_at = Some(started);
        }
    }

    enqueue_due(db, push_tx).await?;
    refresh_and_store(state, db).await?;
    state.health.write().await.last_persist_at = Some(Utc::now());
    Ok(())
}

async fn enqueue_due(db: &TursoDb, push_tx: &mpsc::Sender<SessionRow>) -> Result<(), PersistError> {
    let due = timed(db.list_due_pushes()).await?;
    for row in due {
        if push_tx.try_send(row).is_err() {
            break;
        }
    }
    Ok(())
}

async fn refresh_and_store(state: &AppState, db: &TursoDb) -> Result<(), PersistError> {
    let view = timed(refresh_view(db)).await?;
    *state.db_view.write().await = view;
    Ok(())
}

async fn handle_cmd(
    state: &AppState,
    db: &TursoDb,
    opened_path: &Path,
    cmd: PersistCmd,
) -> Result<(), PersistError> {
    match cmd {
        PersistCmd::Confirm {
            fingerprint,
            title,
            reply,
        } => {
            let identity_id = format!("user:{fingerprint}");
            let res = timed(async {
                db.upsert_mapping(&fingerprint, &title, &identity_id)
                    .await?;
                let _ = db.remove_ignored(&identity_id).await;
                Ok(())
            })
            .await;
            let out = persist_reply(&res);
            let _ = reply.send(out);
            res?;
            load_prefs(db, state).await.ok();
        }
        PersistCmd::Ignore {
            identity_id,
            title,
            reply,
        } => {
            let id = identity_id.clone();
            let res = timed(async {
                db.upsert_ignored(&id, &title).await?;
                let actives = db.list_active().await.unwrap_or_default();
                let discard: Vec<String> = actives
                    .into_iter()
                    .filter(|s| s.identity_id == id)
                    .map(|s| s.id)
                    .collect();
                if !discard.is_empty() {
                    db.discard_active_sessions(&discard).await?;
                }
                Ok(())
            })
            .await;
            let out = persist_reply(&res);
            let _ = reply.send(out);
            res?;
            {
                let mut live = state.live.write().await;
                if live.identity_id() == Some(identity_id.as_str()) {
                    live.clear_identity();
                }
            }
            load_prefs(db, state).await.ok();
        }
        PersistCmd::Unignore {
            identity_id,
            reply,
        } => {
            let res = timed(db.remove_ignored(&identity_id)).await;
            let out = persist_reply(&res);
            let _ = reply.send(out);
            res?;
            load_prefs(db, state).await.ok();
        }
        PersistCmd::AddManual {
            game,
            identity_id,
            reply,
        } => {
            let res = timed(async {
                db.upsert_manual_game(&game).await?;
                let _ = db.remove_ignored(&identity_id).await;
                Ok(())
            })
            .await;
            let out = persist_reply(&res);
            let _ = reply.send(out);
            res?;
            load_prefs(db, state).await.ok();
        }
        PersistCmd::EnsureOpen { reply } => {
            let configured = state.config.read().await.resolved_db_path();
            if configured.as_path() != opened_path {
                let _ = reply.send(Ok(configured.display().to_string()));
                return Err(PersistError::PathChanged);
            }
            match timed(db.ping()).await {
                Ok(()) => {
                    let _ = reply.send(Ok(configured.display().to_string()));
                }
                Err(PersistError::Poison(e)) => {
                    let _ = reply.send(Err(e.clone()));
                    return Err(PersistError::Poison(e));
                }
                Err(PersistError::PathChanged) => return Err(PersistError::PathChanged),
            }
        }
    }
    refresh_and_store(state, db).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{Confidence, GameIdentity};
    use chrono::Duration as ChronoDuration;
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
    async fn timed_poison_on_timeout() {
        let err = timed(async {
            tokio::time::sleep(Duration::from_secs(30)).await;
            Ok::<(), String>(())
        })
        .await
        .unwrap_err();
        match err {
            PersistError::Poison(msg) => assert!(msg.contains("timeout"), "{msg}"),
            PersistError::PathChanged => panic!("expected timeout poison"),
        }
    }

    #[tokio::test]
    async fn recover_end_orphans_instead_of_discard() {
        let dir = tempdir().unwrap();
        let db = TursoDb::open(dir.path().join("rec.db")).await.unwrap();
        let apex = identity("steam:1172470", "Apex");
        let row = db
            .force_insert_active(&apex, Utc::now() - ChronoDuration::hours(1))
            .await
            .unwrap();
        let mut live = LiveSession {
            last_tick_at: Some(Utc::now()),
            ..Default::default()
        };
        flush_live(&db, &mut live).await.unwrap();
        assert!(db.list_active().await.unwrap().is_empty());
        let due = db.list_due_pushes().await.unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].id, row.id);
        assert!(db.get_session(&row.id).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn pending_end_without_db_row_inserts_and_ends() {
        let dir = tempdir().unwrap();
        let db = TursoDb::open(dir.path().join("mem.db")).await.unwrap();
        let apex = identity("steam:1", "RL");
        let t0 = Utc::now() - ChronoDuration::hours(1);
        let t1 = t0 + ChronoDuration::minutes(50);
        let mut live = LiveSession {
            pending_ends: vec![PendingEnd {
                identity: apex,
                db_session_id: None,
                started_at: t0,
                ended_at: t1,
            }],
            last_tick_at: Some(Utc::now()),
            ..Default::default()
        };
        flush_live(&db, &mut live).await.unwrap();
        assert!(db.list_active().await.unwrap().is_empty());
        let due = db.list_due_pushes().await.unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].started_at, t0);
        assert_eq!(due[0].ended_at, Some(t1));
    }

    #[tokio::test]
    async fn db_active_started_at_wins_over_memory() {
        let dir = tempdir().unwrap();
        let db = TursoDb::open(dir.path().join("win.db")).await.unwrap();
        let apex = identity("steam:1", "RL");
        let db_start = Utc::now() - ChronoDuration::minutes(20);
        let opened = db.force_insert_active(&apex, db_start).await.unwrap();
        let mut live = LiveSession {
            identity: Some(apex),
            started_at: Some(Utc::now()),
            last_seen_at: Some(Utc::now()),
            last_tick_at: Some(Utc::now()),
            ..Default::default()
        };
        flush_live(&db, &mut live).await.unwrap();
        assert_eq!(live.db_session_id.as_deref(), Some(opened.id.as_str()));
        assert_eq!(live.started_at, Some(db_start));
        assert_eq!(db.list_active().await.unwrap().len(), 1);
    }
}
