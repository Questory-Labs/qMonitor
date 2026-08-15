//! In-memory play tracker. Detect always updates this; persist is best-effort.

use chrono::{DateTime, Duration, Utc};

use crate::identity::GameIdentity;

/// Wall-clock grace before a missing primary counts as quit.
pub const MISS_GRACE: Duration = Duration::seconds(8);
/// Unobserved gap treated as sleep/hang — split the session.
pub const SLEEP_SPLIT: Duration = Duration::seconds(30);

#[derive(Debug, Clone)]
pub struct DetectSample {
    pub observed_at: DateTime<Utc>,
    pub primary: Option<GameIdentity>,
}

impl DetectSample {
    pub fn empty() -> Self {
        Self {
            observed_at: Utc::now(),
            primary: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PendingEnd {
    pub identity: GameIdentity,
    pub db_session_id: Option<String>,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default)]
pub struct LiveSession {
    pub identity: Option<GameIdentity>,
    pub started_at: Option<DateTime<Utc>>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub db_session_id: Option<String>,
    pub pending_ends: Vec<PendingEnd>,
    pub last_tick_at: Option<DateTime<Utc>>,
}

impl LiveSession {
    pub fn identity_id(&self) -> Option<&str> {
        self.identity.as_ref().map(|i| i.id.as_str())
    }

    pub fn is_tracking(&self) -> bool {
        self.identity.is_some()
    }

    /// Apply a detect sample. Never waits on I/O.
    pub fn apply(&mut self, sample: &DetectSample) {
        let now = sample.observed_at;
        let gap = self
            .last_tick_at
            .map(|t| now.signed_duration_since(t));
        self.last_tick_at = Some(now);

        if gap.is_some_and(|g| g > SLEEP_SPLIT) && self.identity.is_some() {
            self.queue_end();
        }

        match &sample.primary {
            Some(primary) => {
                if self.identity.as_ref().is_some_and(|cur| cur.id == primary.id) {
                    self.last_seen_at = Some(now);
                    self.identity = Some(primary.clone());
                } else {
                    if self.identity.is_some() {
                        self.queue_end();
                    }
                    self.identity = Some(primary.clone());
                    self.started_at = Some(now);
                    self.last_seen_at = Some(now);
                    self.db_session_id = None;
                }
            }
            None => {
                if self.identity.is_some() {
                    let last = self.last_seen_at.unwrap_or(now);
                    if now.signed_duration_since(last) >= MISS_GRACE {
                        self.queue_end();
                    }
                }
            }
        }
    }

    pub fn clear_identity(&mut self) {
        self.identity = None;
        self.started_at = None;
        self.last_seen_at = None;
        self.db_session_id = None;
    }

    fn queue_end(&mut self) {
        let Some(identity) = self.identity.take() else {
            return;
        };
        let started_at = self
            .started_at
            .or(self.last_seen_at)
            .unwrap_or_else(Utc::now);
        let ended_at = self.last_seen_at.unwrap_or(started_at);
        let db_session_id = self.db_session_id.take();
        self.started_at = None;
        self.last_seen_at = None;
        self.pending_ends.push(PendingEnd {
            identity,
            db_session_id,
            started_at,
            ended_at,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{Confidence, GameIdentity};

    fn game(id: &str, title: &str) -> GameIdentity {
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

    fn sample_at(at: DateTime<Utc>, primary: Option<GameIdentity>) -> DetectSample {
        DetectSample {
            observed_at: at,
            primary,
        }
    }

    #[test]
    fn tracks_start_without_db() {
        let mut live = LiveSession::default();
        let t0 = Utc::now();
        live.apply(&sample_at(t0, Some(game("steam:1", "RL"))));
        assert_eq!(live.identity_id(), Some("steam:1"));
        assert_eq!(live.started_at, Some(t0));
        assert!(live.pending_ends.is_empty());
    }

    #[test]
    fn pending_end_after_miss_grace_even_without_db() {
        let mut live = LiveSession::default();
        let t0 = Utc::now();
        live.apply(&sample_at(t0, Some(game("steam:1", "RL"))));
        live.apply(&sample_at(t0 + Duration::seconds(3), None));
        assert!(live.is_tracking(), "still in grace");
        assert!(live.pending_ends.is_empty());

        live.apply(&sample_at(t0 + Duration::seconds(9), None));
        assert!(!live.is_tracking());
        assert_eq!(live.pending_ends.len(), 1);
        assert_eq!(live.pending_ends[0].ended_at, t0);
        assert_eq!(live.pending_ends[0].started_at, t0);
    }

    #[test]
    fn hang_with_game_gone_ends_immediately() {
        let mut live = LiveSession::default();
        let t0 = Utc::now();
        live.apply(&sample_at(t0, Some(game("steam:1", "RL"))));
        live.apply(&sample_at(t0 + Duration::hours(1), None));
        assert_eq!(live.pending_ends.len(), 1);
        assert_eq!(live.pending_ends[0].ended_at, t0);
        assert!(!live.is_tracking());
    }

    #[test]
    fn sleep_split_while_still_playing() {
        let mut live = LiveSession::default();
        let t0 = Utc::now();
        let rl = game("steam:1", "RL");
        live.apply(&sample_at(t0, Some(rl.clone())));
        live.db_session_id = Some("db-1".into());
        let t1 = t0 + Duration::hours(2);
        live.apply(&sample_at(t1, Some(rl)));
        assert_eq!(live.pending_ends.len(), 1);
        assert_eq!(live.pending_ends[0].db_session_id.as_deref(), Some("db-1"));
        assert_eq!(live.pending_ends[0].ended_at, t0);
        assert_eq!(live.identity_id(), Some("steam:1"));
        assert_eq!(live.started_at, Some(t1));
        assert!(live.db_session_id.is_none());
    }

    #[test]
    fn identity_switch_queues_end_then_starts() {
        let mut live = LiveSession::default();
        let t0 = Utc::now();
        live.apply(&sample_at(t0, Some(game("steam:1", "A"))));
        live.apply(&sample_at(t0 + Duration::seconds(3), Some(game("steam:2", "B"))));
        assert_eq!(live.pending_ends.len(), 1);
        assert_eq!(live.pending_ends[0].identity.id, "steam:1");
        assert_eq!(live.identity_id(), Some("steam:2"));
    }

    #[test]
    fn flicker_within_grace_does_not_end() {
        let mut live = LiveSession::default();
        let t0 = Utc::now();
        let rl = game("steam:1", "RL");
        live.apply(&sample_at(t0, Some(rl.clone())));
        live.apply(&sample_at(t0 + Duration::seconds(3), None));
        live.apply(&sample_at(t0 + Duration::seconds(6), Some(rl)));
        assert!(live.pending_ends.is_empty());
        assert_eq!(live.started_at, Some(t0));
        assert_eq!(live.last_seen_at, Some(t0 + Duration::seconds(6)));
    }
}
