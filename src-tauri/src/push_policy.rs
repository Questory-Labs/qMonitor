//! Decide whether an ended session auto-pushes to Questory.

use std::collections::HashSet;

use crate::config::AppConfig;
use crate::identity::resolver::IdentityPipeline;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    TooShort,
    NotOnList,
}

impl SkipReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TooShort => "too_short",
            Self::NotOnList => "not_on_list",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Allowlist {
    pub identity_ids: HashSet<String>,
    pub steam_app_ids: HashSet<u32>,
}

impl Allowlist {
    pub fn from_pipeline(pipe: &IdentityPipeline) -> Self {
        let ignored = &pipe.ignored_identities;
        let mut identity_ids = HashSet::new();
        let mut steam_app_ids = HashSet::new();

        for g in pipe.steam.games.values() {
            let id = format!("steam:{}", g.app_id);
            if ignored.contains(&id) {
                continue;
            }
            identity_ids.insert(id);
            steam_app_ids.insert(g.app_id);
        }

        for mapping in pipe.user_mappings.values() {
            if ignored.contains(&mapping.identity_id) {
                continue;
            }
            identity_ids.insert(mapping.identity_id.clone());
        }

        for manual in &pipe.manual_games {
            let id = manual
                .steam_app_id
                .map(|sid| format!("steam:{sid}"))
                .unwrap_or_else(|| format!("manual:{}", manual.id));
            if ignored.contains(&id) {
                continue;
            }
            identity_ids.insert(id);
            if let Some(sid) = manual.steam_app_id {
                steam_app_ids.insert(sid);
            }
        }

        Self {
            identity_ids,
            steam_app_ids,
        }
    }

    pub fn contains(&self, identity_id: &str, steam_app_id: Option<u32>) -> bool {
        if self.identity_ids.contains(identity_id) {
            return true;
        }
        steam_app_id.is_some_and(|sid| self.steam_app_ids.contains(&sid))
    }
}

#[derive(Debug, Clone)]
pub struct PushPolicy {
    pub min_duration_secs: i64,
    pub allowlist: Option<Allowlist>,
}

impl PushPolicy {
    pub fn push_all() -> Self {
        Self {
            min_duration_secs: 0,
            allowlist: None,
        }
    }

    pub fn from_config(cfg: &AppConfig, pipe: &IdentityPipeline) -> Self {
        Self {
            min_duration_secs: i64::from(cfg.min_push_duration_mins).saturating_mul(60),
            allowlist: cfg
                .push_from_list_only
                .then(|| Allowlist::from_pipeline(pipe)),
        }
    }

    pub fn skip_reason(
        &self,
        identity_id: &str,
        steam_app_id: Option<u32>,
        duration_secs: i64,
    ) -> Option<SkipReason> {
        if self.min_duration_secs > 0 && duration_secs < self.min_duration_secs {
            return Some(SkipReason::TooShort);
        }
        if let Some(list) = &self.allowlist {
            if !list.contains(identity_id, steam_app_id) {
                return Some(SkipReason::NotOnList);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::steam_library::SteamGame;
    use crate::identity::ManualGame;
    use std::path::PathBuf;

    fn pipe_with_dota() -> IdentityPipeline {
        let mut pipe = IdentityPipeline::new(None, None, Default::default(), None);
        pipe.steam.games.insert(
            570,
            SteamGame {
                app_id: 570,
                title: "Dota 2".into(),
                install_path: PathBuf::from("/dota"),
            },
        );
        pipe
    }

    #[test]
    fn zero_minutes_does_not_gate_duration() {
        let policy = PushPolicy::push_all();
        assert!(policy.skip_reason("steam:570", Some(570), 1).is_none());
        assert!(policy.skip_reason("steam:570", Some(570), 0).is_none());
    }

    #[test]
    fn duration_floor_skips_strictly_under() {
        let policy = PushPolicy {
            min_duration_secs: 300,
            allowlist: None,
        };
        assert_eq!(
            policy.skip_reason("steam:570", Some(570), 299),
            Some(SkipReason::TooShort)
        );
        assert!(policy
            .skip_reason("steam:570", Some(570), 300)
            .is_none());
    }

    #[test]
    fn allowlist_matches_discord_row_by_steam_sku() {
        let policy = PushPolicy {
            min_duration_secs: 0,
            allowlist: Some(Allowlist::from_pipeline(&pipe_with_dota())),
        };
        assert!(policy
            .skip_reason("discord:111", Some(570), 600)
            .is_none());
        assert_eq!(
            policy.skip_reason("discord:111", None, 600),
            Some(SkipReason::NotOnList)
        );
        assert_eq!(
            policy.skip_reason("catalog:hades", None, 600),
            Some(SkipReason::NotOnList)
        );
    }

    #[test]
    fn ignored_steam_sku_is_not_on_allowlist() {
        let mut pipe = pipe_with_dota();
        pipe.ignored_identities.insert("steam:570".into());
        let policy = PushPolicy {
            min_duration_secs: 0,
            allowlist: Some(Allowlist::from_pipeline(&pipe)),
        };
        assert_eq!(
            policy.skip_reason("discord:111", Some(570), 600),
            Some(SkipReason::NotOnList)
        );
    }

    #[test]
    fn manual_with_steam_app_id_joins_sku_allowlist() {
        let mut pipe = IdentityPipeline::new(None, None, Default::default(), None);
        pipe.manual_games.push(ManualGame {
            id: "m1".into(),
            title: "Dota 2".into(),
            exe_name: "dota2.exe".into(),
            path_hint: None,
            steam_app_id: Some(570),
        });
        let policy = PushPolicy {
            min_duration_secs: 0,
            allowlist: Some(Allowlist::from_pipeline(&pipe)),
        };
        assert!(policy
            .skip_reason("discord:111", Some(570), 60)
            .is_none());
    }

    #[test]
    fn from_config_respects_defaults() {
        let cfg = AppConfig::default();
        let pipe = pipe_with_dota();
        let policy = PushPolicy::from_config(&cfg, &pipe);
        assert_eq!(policy.min_duration_secs, 0);
        assert!(policy.allowlist.is_none());
        assert!(policy.skip_reason("catalog:x", None, 5).is_none());
    }
}
