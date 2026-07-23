use fact_envelope::FixtureFactView;
use serde::Serialize;

/// Bounded server-to-client Phase 0 frame.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StreamFrame {
    /// Frame semantic.
    #[serde(rename = "type")]
    pub frame_type: &'static str,
    /// Current logical fixture projections.
    pub facts: Vec<FixtureFactView>,
}

impl StreamFrame {
    /// Constructs a full initial-state snapshot.
    #[must_use]
    pub const fn snapshot(facts: Vec<FixtureFactView>) -> Self {
        Self {
            frame_type: "snapshot",
            facts,
        }
    }
}
