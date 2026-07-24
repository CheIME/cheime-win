//! Pure state machine for the short post-commit learning rollback window.

use cheime_model::CommitToken;
use std::sync::OnceLock;
use std::time::Instant;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GuardEvent {
    TextInput,
    Navigation,
    FocusChanged,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ArmedRollback {
    token: CommitToken,
    context_identity: usize,
    deadline_ms: u64,
}

#[derive(Clone, Debug, Default)]
pub struct RollbackGuard {
    armed: Option<ArmedRollback>,
}

impl RollbackGuard {
    pub fn arm(&mut self, token: CommitToken, context_identity: usize, deadline_ms: u64) {
        self.armed = Some(ArmedRollback {
            token,
            context_identity,
            deadline_ms,
        });
    }

    pub fn is_armed(&self) -> bool {
        self.armed.is_some()
    }

    pub fn matching_token(&self, context_identity: usize, now_ms: u64) -> Option<CommitToken> {
        self.armed
            .filter(|armed| {
                armed.context_identity == context_identity && now_ms < armed.deadline_ms
            })
            .map(|armed| armed.token)
    }

    pub fn observe(&mut self, _event: GuardEvent) {
        self.disarm();
    }

    pub fn disarm(&mut self) {
        self.armed = None;
    }
}

pub fn monotonic_ms() -> u64 {
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    ORIGIN
        .get_or_init(Instant::now)
        .elapsed()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cheime_model::{ActionId, CommitToken, SessionEpoch, SessionId};

    fn token() -> CommitToken {
        CommitToken {
            session: SessionId::new(7),
            epoch: SessionEpoch::new(8),
            action_id: ActionId::new(9),
        }
    }

    #[test]
    fn matches_only_same_context_before_deadline() {
        let mut guard = RollbackGuard::default();
        guard.arm(token(), 42, 10_000);

        assert_eq!(guard.matching_token(42, 9_999), Some(token()));
        assert_eq!(guard.matching_token(41, 9_999), None);
        assert_eq!(guard.matching_token(42, 10_000), None);
    }

    #[test]
    fn intervening_edits_navigation_and_focus_changes_disarm() {
        for event in [
            GuardEvent::TextInput,
            GuardEvent::Navigation,
            GuardEvent::FocusChanged,
        ] {
            let mut guard = RollbackGuard::default();
            guard.arm(token(), 42, 10_000);
            guard.observe(event);
            assert!(!guard.is_armed());
        }
    }
}
