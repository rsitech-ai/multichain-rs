use observation_envelope::{CollectorSequence, SourceSessionId};
use platform_proto::control::CoverageInterval;

/// Owns the only collector-sequence allocator for one source session.
#[derive(Clone, Debug)]
pub struct SourceSession {
    source_id: String,
    id: SourceSessionId,
    next_sequence: u64,
    started_at_unix_ns: i64,
}

impl SourceSession {
    /// Creates a session with an injected ID for deterministic tests.
    ///
    /// # Panics
    ///
    /// This cannot panic because the accepted array is exactly 16 bytes.
    pub fn with_id(source_id: impl Into<String>, id: [u8; 16], started_at_unix_ns: i64) -> Self {
        Self {
            source_id: source_id.into(),
            id: SourceSessionId::try_from(id.as_slice()).expect("fixed session ID"),
            next_sequence: 0,
            started_at_unix_ns,
        }
    }

    /// Creates a session identifier from operating-system randomness.
    ///
    /// # Errors
    ///
    /// Returns the platform entropy error without falling back to a weaker ID.
    pub fn new(
        source_id: impl Into<String>,
        started_at_unix_ns: i64,
    ) -> Result<Self, getrandom::Error> {
        let mut id = [0_u8; 16];
        getrandom::fill(&mut id)?;
        Ok(Self::with_id(source_id, id, started_at_unix_ns))
    }

    #[must_use]
    pub const fn id(&self) -> SourceSessionId {
        self.id
    }

    /// Allocates the next total-order position.
    pub fn allocate(&mut self) -> CollectorSequence {
        let sequence = CollectorSequence::new(self.next_sequence);
        self.next_sequence = self.next_sequence.saturating_add(1);
        sequence
    }

    #[must_use]
    pub const fn next_sequence(&self) -> u64 {
        self.next_sequence
    }

    /// Ends this session and starts a new independent sequence.
    #[must_use]
    pub fn reconnect(self, next_id: [u8; 16], ended_at_unix_ns: i64) -> (Self, CoverageInterval) {
        let interval = CoverageInterval {
            source_id: self.source_id.clone(),
            source_session_id: self.id.as_bytes().to_vec(),
            start_unix_ns: self.started_at_unix_ns,
            end_unix_ns: Some(ended_at_unix_ns),
            state: "closed".to_owned(),
            cause: "source_reconnect".to_owned(),
            repair_evidence_observation_ids: Vec::new(),
        };
        (
            Self::with_id(self.source_id, next_id, ended_at_unix_ns),
            interval,
        )
    }
}
