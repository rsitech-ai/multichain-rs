use platform_proto::control::SourceHealth;

use crate::session::SourceSession;

/// Small explicit operational projection; interval history lives separately.
#[derive(Clone, Debug)]
pub struct HealthTracker {
    source_id: String,
    connection_state: String,
    sync_state: String,
    source_sequence: Option<u64>,
    open_incomplete_intervals: u64,
}

impl HealthTracker {
    #[must_use]
    pub fn new(source_id: impl Into<String>) -> Self {
        Self {
            source_id: source_id.into(),
            connection_state: "connecting".to_owned(),
            sync_state: "unknown".to_owned(),
            source_sequence: None,
            open_incomplete_intervals: 0,
        }
    }

    pub fn connected(&mut self) {
        "connected".clone_into(&mut self.connection_state);
        "healthy".clone_into(&mut self.sync_state);
    }

    pub fn gap_opened(&mut self) {
        "gapped".clone_into(&mut self.sync_state);
        self.open_incomplete_intervals = self.open_incomplete_intervals.saturating_add(1);
    }

    pub fn reconciled(&mut self, source_sequence: u64) {
        "healthy".clone_into(&mut self.sync_state);
        self.source_sequence = Some(source_sequence);
        self.open_incomplete_intervals = self.open_incomplete_intervals.saturating_sub(1);
    }

    #[must_use]
    pub fn snapshot(&self, session: &SourceSession, observed_at_unix_ns: i64) -> SourceHealth {
        SourceHealth {
            source_id: self.source_id.clone(),
            source_session_id: session.id().as_bytes().to_vec(),
            connection_state: self.connection_state.clone(),
            sync_state: self.sync_state.clone(),
            source_head: None,
            source_sequence: self.source_sequence,
            collector_sequence: session.next_sequence(),
            observed_at_unix_ns,
            clock_offset_ns: 0,
            wal_backlog_bytes: 0,
            broker_publication_lag: 0,
            open_incomplete_intervals: self.open_incomplete_intervals,
            quality_flags: if self.sync_state == "gapped" {
                vec!["known_incomplete".to_owned()]
            } else {
                Vec::new()
            },
        }
    }
}
