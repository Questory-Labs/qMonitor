//! Embedded Turso Database (https://github.com/tursodatabase/turso) session outbox.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::path::Path;
use turso::{Builder, Connection, Database, Value};
use uuid::Uuid;

use crate::identity::GameIdentity;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PushStatus {
    Active,
    Pending,
    Synced,
    Failed,
}

impl PushStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            PushStatus::Active => "active",
            PushStatus::Pending => "pending",
            PushStatus::Synced => "synced",
            PushStatus::Failed => "failed",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "active" => PushStatus::Active,
            "synced" => PushStatus::Synced,
            "failed" => PushStatus::Failed,
            _ => PushStatus::Pending,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRow {
    pub id: String,
    pub identity_id: String,
    pub title: String,
    pub steam_app_id: Option<u32>,
    pub exe: Option<String>,
    pub source: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub duration_secs: Option<i64>,
    pub push_status: PushStatus,
    pub acked_at: Option<DateTime<Utc>>,
    pub retry_count: i64,
    pub next_retry_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
}

pub struct TursoDb {
    /// Held so the connection is not dropped with the builder handle.
    _db: Database,
    conn: Connection,
}

impl TursoDb {
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let path_str = path.to_string_lossy().to_string();
        let db = Builder::new_local(&path_str)
            .build()
            .await
            .map_err(|e| format!("turso open: {e}"))?;
        let conn = db.connect().map_err(|e| format!("turso conn: {e}"))?;
        let this = Self { _db: db, conn };
        this.migrate().await?;
        Ok(this)
    }

    async fn migrate(&self) -> Result<(), String> {
        for stmt in [
            r#"CREATE TABLE IF NOT EXISTS sessions (
                  id TEXT PRIMARY KEY,
                  identity_id TEXT NOT NULL,
                  title TEXT NOT NULL,
                  steam_app_id INTEGER,
                  exe TEXT,
                  source TEXT NOT NULL,
                  started_at TEXT NOT NULL,
                  ended_at TEXT,
                  duration_secs INTEGER,
                  push_status TEXT NOT NULL,
                  acked_at TEXT,
                  retry_count INTEGER NOT NULL DEFAULT 0,
                  next_retry_at TEXT,
                  last_error TEXT
                )"#,
            r#"CREATE INDEX IF NOT EXISTS idx_sessions_status ON sessions(push_status)"#,
            r#"CREATE INDEX IF NOT EXISTS idx_sessions_started ON sessions(started_at)"#,
            r#"CREATE TABLE IF NOT EXISTS user_mappings (
                  fingerprint TEXT PRIMARY KEY,
                  title TEXT NOT NULL,
                  identity_id TEXT NOT NULL
                )"#,
            r#"CREATE TABLE IF NOT EXISTS ignored_identities (
                  identity_id TEXT PRIMARY KEY,
                  title TEXT NOT NULL,
                  updated_at TEXT NOT NULL
                )"#,
            r#"CREATE TABLE IF NOT EXISTS manual_games (
                  id TEXT PRIMARY KEY,
                  title TEXT NOT NULL,
                  exe_name TEXT NOT NULL,
                  path_hint TEXT,
                  steam_app_id INTEGER,
                  created_at TEXT NOT NULL
                )"#,
        ] {
            self.conn
                .execute(stmt, ())
                .await
                .map_err(|e| format!("migrate: {e}"))?;
        }
        Ok(())
    }

    /// Open (or reuse) an active row. Existing DB `started_at` wins over `started_at`.
    pub async fn open_session_at(
        &self,
        identity: &GameIdentity,
        started_at: DateTime<Utc>,
    ) -> Result<SessionRow, String> {
        if let Some(existing) = self
            .list_active()
            .await?
            .into_iter()
            .filter(|s| s.identity_id == identity.id)
            .min_by_key(|s| s.started_at)
        {
            return Ok(existing);
        }

        let row = SessionRow {
            id: Uuid::new_v4().to_string(),
            identity_id: identity.id.clone(),
            title: identity.title.clone(),
            steam_app_id: identity.steam_app_id,
            exe: identity.exe.clone(),
            source: identity.source.clone(),
            started_at,
            ended_at: None,
            duration_secs: None,
            push_status: PushStatus::Active,
            acked_at: None,
            retry_count: 0,
            next_retry_at: None,
            last_error: None,
        };
        self.conn
            .execute(
                r#"INSERT INTO sessions
                (id, identity_id, title, steam_app_id, exe, source, started_at, push_status, retry_count)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, 0)"#,
                (
                    row.id.as_str(),
                    row.identity_id.as_str(),
                    row.title.as_str(),
                    row.steam_app_id.map(|x| x as i64),
                    row.exe.as_deref(),
                    row.source.as_str(),
                    row.started_at.to_rfc3339(),
                    row.push_status.as_str(),
                ),
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok(row)
    }

    /// Test helper: insert an active row even if one already exists for the identity.
    #[cfg(test)]
    pub async fn force_insert_active(
        &self,
        identity: &GameIdentity,
        started_at: DateTime<Utc>,
    ) -> Result<SessionRow, String> {
        let row = SessionRow {
            id: Uuid::new_v4().to_string(),
            identity_id: identity.id.clone(),
            title: identity.title.clone(),
            steam_app_id: identity.steam_app_id,
            exe: identity.exe.clone(),
            source: identity.source.clone(),
            started_at,
            ended_at: None,
            duration_secs: None,
            push_status: PushStatus::Active,
            acked_at: None,
            retry_count: 0,
            next_retry_at: None,
            last_error: None,
        };
        self.conn
            .execute(
                r#"INSERT INTO sessions
                (id, identity_id, title, steam_app_id, exe, source, started_at, push_status, retry_count)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, 0)"#,
                (
                    row.id.as_str(),
                    row.identity_id.as_str(),
                    row.title.as_str(),
                    row.steam_app_id.map(|x| x as i64),
                    row.exe.as_deref(),
                    row.source.as_str(),
                    row.started_at.to_rfc3339(),
                    row.push_status.as_str(),
                ),
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok(row)
    }

    /// All active sessions, oldest first.
    pub async fn list_active(&self) -> Result<Vec<SessionRow>, String> {
        let mut rows = self
            .conn
            .query(
                r#"SELECT id, identity_id, title, steam_app_id, exe, source, started_at, ended_at,
                          duration_secs, push_status, acked_at, retry_count, next_retry_at, last_error
                   FROM sessions WHERE push_status='active' ORDER BY started_at ASC"#,
                (),
            )
            .await
            .map_err(|e| e.to_string())?;
        collect_sessions(&mut rows).await
    }

    /// Delete an active session without ending/pushing (restart orphan cleanup).
    pub async fn discard_session(&self, id: &str) -> Result<(), String> {
        self.conn
            .execute(
                r#"DELETE FROM sessions WHERE id=? AND push_status='active'"#,
                (id,),
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn discard_active_sessions(&self, ids: &[String]) -> Result<(), String> {
        for id in ids {
            self.discard_session(id).await?;
        }
        Ok(())
    }

    pub async fn end_session_at(
        &self,
        id: &str,
        ended_at: DateTime<Utc>,
    ) -> Result<SessionRow, String> {
        let mut row = self
            .get_session(id)
            .await?
            .ok_or_else(|| "missing".to_string())?;
        if row.push_status != PushStatus::Active {
            return Ok(row);
        }
        let ended = if ended_at < row.started_at {
            row.started_at
        } else {
            ended_at
        };
        let duration = (ended - row.started_at).num_seconds().max(0);
        row.ended_at = Some(ended);
        row.duration_secs = Some(duration);
        row.push_status = PushStatus::Pending;
        row.next_retry_at = Some(Utc::now());
        self.conn
            .execute(
                r#"UPDATE sessions SET ended_at=?, duration_secs=?, push_status=?, next_retry_at=? WHERE id=?"#,
                (
                    ended.to_rfc3339(),
                    duration,
                    PushStatus::Pending.as_str(),
                    Utc::now().to_rfc3339(),
                    id,
                ),
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok(row)
    }

    pub async fn mark_synced(&self, id: &str) -> Result<(), String> {
        let now = Utc::now().to_rfc3339();
        self.conn
            .execute(
                r#"UPDATE sessions SET push_status=?, acked_at=?, last_error=NULL, next_retry_at=NULL WHERE id=?"#,
                (PushStatus::Synced.as_str(), now.as_str(), id),
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn mark_push_failed(
        &self,
        id: &str,
        error: &str,
        retry_count: i64,
    ) -> Result<(), String> {
        let delay_secs = match retry_count {
            0 => 30,
            1 => 120,
            2 => 600,
            _ => 3600,
        };
        let next = (Utc::now() + Duration::seconds(delay_secs)).to_rfc3339();
        self.conn
            .execute(
                r#"UPDATE sessions SET push_status=?, last_error=?, retry_count=?, next_retry_at=? WHERE id=?"#,
                (
                    PushStatus::Failed.as_str(),
                    error,
                    retry_count,
                    next.as_str(),
                    id,
                ),
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn list_due_pushes(&self) -> Result<Vec<SessionRow>, String> {
        let now = Utc::now().to_rfc3339();
        let mut rows = self
            .conn
            .query(
                r#"SELECT id, identity_id, title, steam_app_id, exe, source, started_at, ended_at,
                          duration_secs, push_status, acked_at, retry_count, next_retry_at, last_error
                   FROM sessions
                   WHERE push_status IN ('pending','failed')
                     AND ended_at IS NOT NULL
                     AND (next_retry_at IS NULL OR next_retry_at <= ?)
                   ORDER BY ended_at ASC
                   LIMIT 20"#,
                (now.as_str(),),
            )
            .await
            .map_err(|e| e.to_string())?;
        collect_sessions(&mut rows).await
    }

    pub async fn list_sessions(&self, limit: i64) -> Result<Vec<SessionRow>, String> {
        let mut rows = self
            .conn
            .query(
                r#"SELECT id, identity_id, title, steam_app_id, exe, source, started_at, ended_at,
                          duration_secs, push_status, acked_at, retry_count, next_retry_at, last_error
                   FROM sessions
                   ORDER BY started_at DESC
                   LIMIT ?"#,
                (limit,),
            )
            .await
            .map_err(|e| e.to_string())?;
        collect_sessions(&mut rows).await
    }

    pub async fn get_active(&self) -> Result<Option<SessionRow>, String> {
        // Prefer oldest active (matches resume-across-restart keep-oldest policy).
        Ok(self.list_active().await?.into_iter().next())
    }

    pub async fn get_session(&self, id: &str) -> Result<Option<SessionRow>, String> {
        let mut rows = self
            .conn
            .query(
                r#"SELECT id, identity_id, title, steam_app_id, exe, source, started_at, ended_at,
                          duration_secs, push_status, acked_at, retry_count, next_retry_at, last_error
                   FROM sessions WHERE id=?"#,
                (id,),
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok(collect_sessions(&mut rows).await?.into_iter().next())
    }

    pub async fn count_pending(&self) -> Result<i64, String> {
        let mut rows = self
            .conn
            .query(
                r#"SELECT COUNT(*) FROM sessions WHERE push_status IN ('pending','failed')"#,
                (),
            )
            .await
            .map_err(|e| e.to_string())?;
        if let Some(row) = rows.next().await.map_err(|e| e.to_string())? {
            return Ok(value_i64(row.get_value(0).map_err(|e| e.to_string())?).unwrap_or(0));
        }
        Ok(0)
    }

    pub async fn purge_synced(&self, older_than_days: u32) -> Result<u64, String> {
        let cutoff = (Utc::now() - Duration::days(older_than_days as i64)).to_rfc3339();
        let n = self
            .conn
            .execute(
                r#"DELETE FROM sessions WHERE push_status='synced' AND acked_at IS NOT NULL AND acked_at < ?"#,
                (cutoff.as_str(),),
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok(n as u64)
    }

    pub async fn upsert_mapping(
        &self,
        fingerprint: &str,
        title: &str,
        identity_id: &str,
    ) -> Result<(), String> {
        self.conn
            .execute(
                r#"INSERT INTO user_mappings (fingerprint, title, identity_id) VALUES (?, ?, ?)
                   ON CONFLICT(fingerprint) DO UPDATE SET title=excluded.title, identity_id=excluded.identity_id"#,
                (fingerprint, title, identity_id),
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn list_mappings(
        &self,
    ) -> Result<Vec<crate::identity::resolver::UserMapping>, String> {
        let mut rows = self
            .conn
            .query(
                r#"SELECT fingerprint, title, identity_id FROM user_mappings"#,
                (),
            )
            .await
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await.map_err(|e| e.to_string())? {
            out.push(crate::identity::resolver::UserMapping {
                fingerprint: value_text(row.get_value(0).map_err(|e| e.to_string())?)
                    .ok_or("fingerprint")?,
                title: value_text(row.get_value(1).map_err(|e| e.to_string())?).ok_or("title")?,
                identity_id: value_text(row.get_value(2).map_err(|e| e.to_string())?)
                    .ok_or("identity_id")?,
            });
        }
        Ok(out)
    }

    pub async fn upsert_ignored(&self, identity_id: &str, title: &str) -> Result<(), String> {
        let now = Utc::now().to_rfc3339();
        self.conn
            .execute(
                r#"INSERT INTO ignored_identities (identity_id, title, updated_at) VALUES (?, ?, ?)
                   ON CONFLICT(identity_id) DO UPDATE SET title=excluded.title, updated_at=excluded.updated_at"#,
                (identity_id, title, now.as_str()),
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn remove_ignored(&self, identity_id: &str) -> Result<(), String> {
        self.conn
            .execute(
                r#"DELETE FROM ignored_identities WHERE identity_id=?"#,
                (identity_id,),
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn list_ignored(&self) -> Result<Vec<crate::identity::IgnoredIdentity>, String> {
        let mut rows = self
            .conn
            .query(
                r#"SELECT identity_id, title FROM ignored_identities ORDER BY title COLLATE NOCASE"#,
                (),
            )
            .await
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await.map_err(|e| e.to_string())? {
            out.push(crate::identity::IgnoredIdentity {
                identity_id: value_text(row.get_value(0).map_err(|e| e.to_string())?)
                    .ok_or("identity_id")?,
                title: value_text(row.get_value(1).map_err(|e| e.to_string())?).ok_or("title")?,
            });
        }
        Ok(out)
    }

    pub async fn upsert_manual_game(
        &self,
        game: &crate::identity::ManualGame,
    ) -> Result<(), String> {
        let now = Utc::now().to_rfc3339();
        self.conn
            .execute(
                r#"INSERT INTO manual_games (id, title, exe_name, path_hint, steam_app_id, created_at)
                   VALUES (?, ?, ?, ?, ?, ?)
                   ON CONFLICT(id) DO UPDATE SET
                     title=excluded.title,
                     exe_name=excluded.exe_name,
                     path_hint=excluded.path_hint,
                     steam_app_id=excluded.steam_app_id"#,
                (
                    game.id.as_str(),
                    game.title.as_str(),
                    game.exe_name.as_str(),
                    game.path_hint.as_deref(),
                    game.steam_app_id.map(|x| x as i64),
                    now.as_str(),
                ),
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn list_manual_games(&self) -> Result<Vec<crate::identity::ManualGame>, String> {
        let mut rows = self
            .conn
            .query(
                r#"SELECT id, title, exe_name, path_hint, steam_app_id
                   FROM manual_games ORDER BY title COLLATE NOCASE"#,
                (),
            )
            .await
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await.map_err(|e| e.to_string())? {
            out.push(crate::identity::ManualGame {
                id: value_text(row.get_value(0).map_err(|e| e.to_string())?).ok_or("id")?,
                title: value_text(row.get_value(1).map_err(|e| e.to_string())?).ok_or("title")?,
                exe_name: value_text(row.get_value(2).map_err(|e| e.to_string())?)
                    .ok_or("exe_name")?,
                path_hint: value_text(row.get_value(3).map_err(|e| e.to_string())?),
                steam_app_id: value_i64(row.get_value(4).map_err(|e| e.to_string())?)
                    .map(|x| x as u32),
            });
        }
        Ok(out)
    }

    pub async fn ping(&self) -> Result<(), String> {
        // Must use query — execute() rejects statements that return rows.
        let mut rows = self
            .conn
            .query("SELECT 1", ())
            .await
            .map_err(|e| e.to_string())?;
        let _ = rows
            .next()
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "db ping: empty".to_string())?;
        Ok(())
    }
}

async fn collect_sessions(rows: &mut turso::Rows) -> Result<Vec<SessionRow>, String> {
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.map_err(|e| e.to_string())? {
        out.push(session_from_row(&row)?);
    }
    Ok(out)
}

fn session_from_row(row: &turso::Row) -> Result<SessionRow, String> {
    Ok(SessionRow {
        id: value_text(row.get_value(0).map_err(|e| e.to_string())?).ok_or("id")?,
        identity_id: value_text(row.get_value(1).map_err(|e| e.to_string())?).ok_or("identity")?,
        title: value_text(row.get_value(2).map_err(|e| e.to_string())?).ok_or("title")?,
        steam_app_id: value_i64(row.get_value(3).map_err(|e| e.to_string())?).map(|x| x as u32),
        exe: value_text(row.get_value(4).map_err(|e| e.to_string())?),
        source: value_text(row.get_value(5).map_err(|e| e.to_string())?).ok_or("source")?,
        started_at: DateTime::parse_from_rfc3339(
            &value_text(row.get_value(6).map_err(|e| e.to_string())?).ok_or("started")?,
        )
        .map_err(|e| e.to_string())?
        .with_timezone(&Utc),
        ended_at: value_text(row.get_value(7).map_err(|e| e.to_string())?)
            .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
            .map(|d| d.with_timezone(&Utc)),
        duration_secs: value_i64(row.get_value(8).map_err(|e| e.to_string())?),
        push_status: PushStatus::parse(
            &value_text(row.get_value(9).map_err(|e| e.to_string())?).unwrap_or_default(),
        ),
        acked_at: value_text(row.get_value(10).map_err(|e| e.to_string())?)
            .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
            .map(|d| d.with_timezone(&Utc)),
        retry_count: value_i64(row.get_value(11).map_err(|e| e.to_string())?).unwrap_or(0),
        next_retry_at: value_text(row.get_value(12).map_err(|e| e.to_string())?)
            .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
            .map(|d| d.with_timezone(&Utc)),
        last_error: value_text(row.get_value(13).map_err(|e| e.to_string())?),
    })
}

fn value_text(v: Value) -> Option<String> {
    match v {
        Value::Text(s) => Some(s),
        Value::Null => None,
        other => Some(format!("{other:?}")),
    }
}

fn value_i64(v: Value) -> Option<i64> {
    match v {
        Value::Integer(i) => Some(i),
        Value::Null => None,
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{Confidence, GameIdentity};
    use tempfile::tempdir;

    fn sample_identity() -> GameIdentity {
        GameIdentity {
            id: "steam:570".into(),
            title: "Dota 2".into(),
            steam_app_id: Some(570),
            exe: Some("dota2.exe".into()),
            confidence: Confidence::High,
            source: "steam".into(),
            fingerprint: None,
        }
    }

    #[tokio::test]
    async fn embedded_turso_session_lifecycle_and_purge() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");
        let db = TursoDb::open(&path).await.expect("open");
        db.ping().await.expect("ping");
        let row = db.open_session_at(&sample_identity(), Utc::now()).await.unwrap();
        assert_eq!(row.push_status, PushStatus::Active);
        let ended = db.end_session_at(&row.id, Utc::now()).await.unwrap();
        assert_eq!(ended.push_status, PushStatus::Pending);
        db.mark_synced(&row.id).await.unwrap();
        // Force old ack via SQL
        db.conn
            .execute(
                "UPDATE sessions SET acked_at=? WHERE id=?",
                (
                    (Utc::now() - Duration::days(40)).to_rfc3339(),
                    row.id.as_str(),
                ),
            )
            .await
            .unwrap();
        assert_eq!(db.purge_synced(30).await.unwrap(), 1);
        assert!(db.list_sessions(10).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn open_session_reuses_existing_active_for_same_identity() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("dup.db");
        let db = TursoDb::open(&path).await.expect("open");
        let a = db.open_session_at(&sample_identity(), Utc::now()).await.unwrap();
        let b = db.open_session_at(&sample_identity(), Utc::now()).await.unwrap();
        assert_eq!(a.id, b.id);
        assert_eq!(db.list_active().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn discard_session_removes_active_without_pending_push() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("discard.db");
        let db = TursoDb::open(&path).await.expect("open");
        let row = db.open_session_at(&sample_identity(), Utc::now()).await.unwrap();
        db.discard_session(&row.id).await.unwrap();
        assert!(db.list_active().await.unwrap().is_empty());
        assert!(db.list_due_pushes().await.unwrap().is_empty());
        assert!(db.get_session(&row.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn list_active_oldest_first() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("order.db");
        let db = TursoDb::open(&path).await.expect("open");
        let id = sample_identity();
        let older = db
            .force_insert_active(&id, Utc::now() - Duration::minutes(10))
            .await
            .unwrap();
        let newer = db
            .force_insert_active(&id, Utc::now() - Duration::minutes(1))
            .await
            .unwrap();
        let actives = db.list_active().await.unwrap();
        assert_eq!(actives.len(), 2);
        assert_eq!(actives[0].id, older.id);
        assert_eq!(actives[1].id, newer.id);
        assert_eq!(db.get_active().await.unwrap().unwrap().id, older.id);
    }

    #[tokio::test]
    async fn ignored_identities_upsert_list_remove() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("ignore.db");
        let db = TursoDb::open(&path).await.expect("open");
        db.upsert_ignored("steam:570", "Dota 2").await.unwrap();
        db.upsert_ignored("steam:570", "Dota 2 Updated")
            .await
            .unwrap();
        let list = db.list_ignored().await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].identity_id, "steam:570");
        assert_eq!(list[0].title, "Dota 2 Updated");
        db.remove_ignored("steam:570").await.unwrap();
        assert!(db.list_ignored().await.unwrap().is_empty());
    }
}
