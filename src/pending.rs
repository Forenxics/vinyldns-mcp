//! In-memory store of planned (not yet applied) write operations.
//!
//! Every write goes through two steps: a `plan_*` tool validates the input,
//! looks up current state, and stores a [`PendingChange`] under a random
//! token; `confirm_change` later applies it. Tokens are single-use and expire.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

/// Upper bound on outstanding plans, so a looping client cannot grow memory without limit.
pub const MAX_PENDING: usize = 100;

/// The API call a confirmed change will make.
#[derive(Debug, Clone, PartialEq)]
pub enum PlannedAction {
    CreateRecordSet {
        zone_id: String,
        body: Value,
    },
    UpdateRecordSet {
        zone_id: String,
        record_set_id: String,
        body: Value,
    },
    DeleteRecordSet {
        zone_id: String,
        record_set_id: String,
    },
    SubmitBatchChange {
        body: Value,
        allow_manual_review: bool,
    },
    CancelBatchChange {
        batch_change_id: String,
    },
}

impl PlannedAction {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::CreateRecordSet { .. } => "create_record_set",
            Self::UpdateRecordSet { .. } => "update_record_set",
            Self::DeleteRecordSet { .. } => "delete_record_set",
            Self::SubmitBatchChange { .. } => "submit_batch_change",
            Self::CancelBatchChange { .. } => "cancel_batch_change",
        }
    }
}

#[derive(Debug, Clone)]
pub struct PendingChange {
    pub token: String,
    pub action: PlannedAction,
    /// One-line human-readable description shown in confirmations.
    pub summary: String,
    /// Structured preview (request body, current state, diff, warnings).
    pub preview: Value,
    pub created: Instant,
}

/// Serializable view of a pending change for `list_pending_changes`.
#[derive(Debug, Serialize)]
pub struct PendingSummary {
    pub token: String,
    pub kind: &'static str,
    pub summary: String,
    pub expires_in_seconds: u64,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PendingError {
    #[error("no pending change with token '{0}' (it may have been applied, discarded, or never existed)")]
    NotFound(String),
    #[error("pending change '{0}' expired; plan it again")]
    Expired(String),
    #[error("too many pending changes ({MAX_PENDING}); confirm or discard some first")]
    Full,
}

#[derive(Debug)]
pub struct PendingStore {
    ttl: Duration,
    inner: Mutex<HashMap<String, PendingChange>>,
}

impl PendingStore {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            inner: Mutex::new(HashMap::new()),
        }
    }

    pub fn ttl(&self) -> Duration {
        self.ttl
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, PendingChange>> {
        // A panic while holding this lock cannot leave the map half-updated,
        // so recovering from poisoning is safe.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn is_expired(&self, change: &PendingChange, now: Instant) -> bool {
        now.saturating_duration_since(change.created) >= self.ttl
    }

    /// Stores a new plan and returns it (including its token).
    pub fn insert(
        &self,
        action: PlannedAction,
        summary: String,
        preview: Value,
    ) -> Result<PendingChange, PendingError> {
        let now = Instant::now();
        let mut map = self.lock();
        map.retain(|_, c| !self.is_expired(c, now));
        if map.len() >= MAX_PENDING {
            return Err(PendingError::Full);
        }
        let change = PendingChange {
            token: uuid::Uuid::new_v4().to_string(),
            action,
            summary,
            preview,
            created: now,
        };
        map.insert(change.token.clone(), change.clone());
        Ok(change)
    }

    /// Returns a copy of a live plan without consuming it.
    pub fn peek(&self, token: &str) -> Result<PendingChange, PendingError> {
        let map = self.lock();
        let change = map.get(token).ok_or_else(|| PendingError::NotFound(token.into()))?;
        if self.is_expired(change, Instant::now()) {
            return Err(PendingError::Expired(token.into()));
        }
        Ok(change.clone())
    }

    /// Removes and returns a live plan. Each token can be taken only once.
    pub fn take(&self, token: &str) -> Result<PendingChange, PendingError> {
        let change = self
            .lock()
            .remove(token)
            .ok_or_else(|| PendingError::NotFound(token.into()))?;
        if self.is_expired(&change, Instant::now()) {
            return Err(PendingError::Expired(token.into()));
        }
        Ok(change)
    }

    /// Discards a plan; returns whether it existed.
    pub fn discard(&self, token: &str) -> bool {
        self.lock().remove(token).is_some()
    }

    pub fn list(&self) -> Vec<PendingSummary> {
        let now = Instant::now();
        let mut map = self.lock();
        map.retain(|_, c| !self.is_expired(c, now));
        let mut out: Vec<_> = map
            .values()
            .map(|c| PendingSummary {
                token: c.token.clone(),
                kind: c.action.kind(),
                summary: c.summary.clone(),
                expires_in_seconds: self
                    .ttl
                    .saturating_sub(now.saturating_duration_since(c.created))
                    .as_secs(),
            })
            .collect();
        out.sort_by_key(|s| std::cmp::Reverse(s.expires_in_seconds));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn action() -> PlannedAction {
        PlannedAction::CancelBatchChange {
            batch_change_id: "b1".into(),
        }
    }

    #[test]
    fn tokens_are_single_use() {
        let store = PendingStore::new(Duration::from_secs(60));
        let c = store.insert(action(), "cancel b1".into(), json!({})).unwrap();
        assert_eq!(store.peek(&c.token).unwrap().action, action());
        assert_eq!(store.take(&c.token).unwrap().summary, "cancel b1");
        assert_eq!(
            store.take(&c.token).unwrap_err(),
            PendingError::NotFound(c.token.clone())
        );
    }

    #[test]
    fn expired_plans_cannot_be_applied() {
        let store = PendingStore::new(Duration::ZERO);
        let c = store.insert(action(), "x".into(), json!({})).unwrap();
        assert_eq!(
            store.peek(&c.token).unwrap_err(),
            PendingError::Expired(c.token.clone())
        );
        assert_eq!(
            store.take(&c.token).unwrap_err(),
            PendingError::Expired(c.token.clone())
        );
        assert!(store.list().is_empty());
    }

    #[test]
    fn discard_and_list() {
        let store = PendingStore::new(Duration::from_secs(60));
        let a = store.insert(action(), "a".into(), json!({})).unwrap();
        let _b = store.insert(action(), "b".into(), json!({})).unwrap();
        assert_eq!(store.list().len(), 2);
        assert!(store.discard(&a.token));
        assert!(!store.discard(&a.token));
        assert_eq!(store.list().len(), 1);
    }

    #[test]
    fn store_is_bounded() {
        let store = PendingStore::new(Duration::from_secs(60));
        for _ in 0..MAX_PENDING {
            store.insert(action(), "x".into(), json!({})).unwrap();
        }
        assert_eq!(
            store.insert(action(), "x".into(), json!({})).unwrap_err(),
            PendingError::Full
        );
    }
}
