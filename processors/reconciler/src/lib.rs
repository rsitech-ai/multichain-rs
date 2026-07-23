#![doc = "Explicit source coverage recovery and bounded lineage traversal."]

use std::collections::{HashMap, HashSet};

use thiserror::Error;

/// Durable interpretation of one source-qualified coverage range.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoverageState {
    /// A gap is known and has not been proven repaired.
    KnownIncomplete,
    /// A prior gap was repaired with named evidence.
    Recovered,
}

/// One append-only coverage revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoverageRevision {
    /// Chain family.
    pub chain: String,
    /// Network identity.
    pub network: String,
    /// Dataset name.
    pub dataset: String,
    /// Exact source identity.
    pub source_id: String,
    /// Inclusive range start.
    pub range_start: u64,
    /// Inclusive repaired range end, absent while open.
    pub range_end: Option<u64>,
    /// Current state at this revision.
    pub state: CoverageState,
    /// Gap cause retained on every revision.
    pub cause: String,
    /// Evidence that proves recovery.
    pub evidence_ids: Vec<String>,
    /// Monotonic append-only revision.
    pub revision: u64,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct CoverageKey {
    chain: String,
    network: String,
    dataset: String,
    source_id: String,
}

/// In-memory coverage state machine used by deterministic reconciliation.
#[derive(Clone, Debug, Default)]
pub struct CoverageLedger {
    revisions: Vec<CoverageRevision>,
    current: HashMap<CoverageKey, usize>,
}

impl CoverageLedger {
    /// Creates an empty ledger.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Opens a durable incomplete interval for one exact source.
    ///
    /// # Errors
    ///
    /// Rejects invalid identity fields, zero/non-monotonic revisions, and an
    /// attempt to open another interval while one is already incomplete.
    #[allow(clippy::too_many_arguments)]
    pub fn open_gap(
        &mut self,
        chain: impl Into<String>,
        network: impl Into<String>,
        dataset: impl Into<String>,
        source_id: impl Into<String>,
        range_start: u64,
        cause: impl Into<String>,
        revision: u64,
    ) -> Result<(), CoverageError> {
        let key = coverage_key(chain, network, dataset, source_id)?;
        let cause = cause.into();
        if cause.trim().is_empty() {
            return Err(CoverageError::EmptyCause);
        }
        self.validate_next_revision(&key, revision)?;
        if self.current_revision(&key).is_some_and(|current| {
            current.state == CoverageState::KnownIncomplete && current.range_end.is_none()
        }) {
            return Err(CoverageError::GapAlreadyOpen);
        }
        self.append(
            key,
            range_start,
            None,
            CoverageState::KnownIncomplete,
            cause,
            Vec::new(),
            revision,
        );
        Ok(())
    }

    /// Closes the current gap with explicit recovery evidence.
    ///
    /// # Errors
    ///
    /// Rejects missing gaps/evidence, invalid ranges, and non-monotonic
    /// revisions. Current-state convergence alone cannot close a gap.
    #[allow(clippy::too_many_arguments)]
    pub fn close_gap<I, S>(
        &mut self,
        chain: impl Into<String>,
        network: impl Into<String>,
        dataset: impl Into<String>,
        source_id: impl Into<String>,
        range_end: u64,
        evidence_ids: I,
        revision: u64,
    ) -> Result<(), CoverageError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let key = coverage_key(chain, network, dataset, source_id)?;
        self.validate_next_revision(&key, revision)?;
        let current = self
            .current_revision(&key)
            .ok_or(CoverageError::NoOpenGap)?
            .clone();
        if current.state != CoverageState::KnownIncomplete || current.range_end.is_some() {
            return Err(CoverageError::NoOpenGap);
        }
        if range_end < current.range_start {
            return Err(CoverageError::InvalidRange);
        }
        let mut evidence_ids = evidence_ids.into_iter().map(Into::into).collect::<Vec<_>>();
        if evidence_ids.is_empty() || evidence_ids.iter().any(|value| value.trim().is_empty()) {
            return Err(CoverageError::MissingRecoveryEvidence);
        }
        evidence_ids.sort_unstable();
        evidence_ids.dedup();
        self.append(
            key,
            current.range_start,
            Some(range_end),
            CoverageState::Recovered,
            current.cause,
            evidence_ids,
            revision,
        );
        Ok(())
    }

    /// Returns the current state for one exact source/dataset key.
    #[must_use]
    pub fn current_state(
        &self,
        chain: &str,
        network: &str,
        dataset: &str,
        source_id: &str,
    ) -> Option<CoverageState> {
        let key = CoverageKey {
            chain: chain.to_owned(),
            network: network.to_owned(),
            dataset: dataset.to_owned(),
            source_id: source_id.to_owned(),
        };
        self.current_revision(&key).map(|revision| revision.state)
    }

    /// Returns all append-only revisions in arrival order.
    #[must_use]
    pub fn revisions(&self) -> &[CoverageRevision] {
        &self.revisions
    }

    fn validate_next_revision(
        &self,
        key: &CoverageKey,
        revision: u64,
    ) -> Result<(), CoverageError> {
        if revision == 0 {
            return Err(CoverageError::ZeroRevision);
        }
        if let Some(current) = self.current_revision(key)
            && revision <= current.revision
        {
            return Err(CoverageError::NonMonotonicRevision {
                current: current.revision,
                proposed: revision,
            });
        }
        Ok(())
    }

    fn current_revision(&self, key: &CoverageKey) -> Option<&CoverageRevision> {
        self.current
            .get(key)
            .and_then(|index| self.revisions.get(*index))
    }

    #[allow(clippy::too_many_arguments)]
    fn append(
        &mut self,
        key: CoverageKey,
        range_start: u64,
        range_end: Option<u64>,
        state: CoverageState,
        cause: String,
        evidence_ids: Vec<String>,
        revision: u64,
    ) {
        let index = self.revisions.len();
        self.revisions.push(CoverageRevision {
            chain: key.chain.clone(),
            network: key.network.clone(),
            dataset: key.dataset.clone(),
            source_id: key.source_id.clone(),
            range_start,
            range_end,
            state,
            cause,
            evidence_ids,
            revision,
        });
        self.current.insert(key, index);
    }
}

/// Node type in the replay lineage graph.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LineageNodeKind {
    /// Exact immutable source observation.
    Observation,
    /// Archive manifest that covers one or more observations.
    ArchiveManifest,
    /// Derived normalized fact.
    Fact,
}

/// One bounded lineage graph node.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LineageNode {
    /// Stable node identity.
    pub id: String,
    /// Layer identity.
    pub kind: LineageNodeKind,
    /// Immediate provenance parents.
    pub parent_ids: Vec<String>,
}

impl LineageNode {
    /// Constructs a node, normalizing parent order.
    #[must_use]
    pub fn new<I, S>(id: impl Into<String>, kind: LineageNodeKind, parent_ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut parent_ids = parent_ids.into_iter().map(Into::into).collect::<Vec<_>>();
        parent_ids.sort_unstable();
        parent_ids.dedup();
        Self {
            id: id.into(),
            kind,
            parent_ids,
        }
    }
}

/// Acyclic graph from facts back to exact source observations.
#[derive(Clone, Debug, Default)]
pub struct LineageGraph {
    nodes: HashMap<String, LineageNode>,
}

impl LineageGraph {
    /// Creates an empty lineage graph.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts a node after proving every parent already exists.
    ///
    /// # Errors
    ///
    /// Rejects blank/duplicate IDs, observations with parents, and unknown
    /// parents. Requiring parents first prevents lineage cycles.
    pub fn insert(&mut self, node: LineageNode) -> Result<(), CoverageError> {
        if node.id.trim().is_empty() {
            return Err(CoverageError::EmptyLineageId);
        }
        if self.nodes.contains_key(&node.id) {
            return Err(CoverageError::DuplicateLineageNode(node.id));
        }
        if node.kind == LineageNodeKind::Observation && !node.parent_ids.is_empty() {
            return Err(CoverageError::ObservationHasParents);
        }
        for parent_id in &node.parent_ids {
            if !self.nodes.contains_key(parent_id) {
                return Err(CoverageError::UnknownLineageNode(parent_id.clone()));
            }
        }
        self.nodes.insert(node.id.clone(), node);
        Ok(())
    }

    /// Traverses a fact to all reachable observations within explicit bounds.
    ///
    /// # Errors
    ///
    /// Rejects unknown roots, graphs outside the caller's depth/node limits,
    /// and branches that do not terminate in an observation.
    pub fn trace_to_observations(
        &self,
        root_id: &str,
        max_depth: usize,
        max_nodes: usize,
    ) -> Result<Vec<LineageNode>, CoverageError> {
        if max_nodes == 0 {
            return Err(CoverageError::LineageNodeLimitExceeded);
        }
        let mut trace = Vec::new();
        let mut visited = HashSet::new();
        let mut stack = vec![(root_id.to_owned(), 0_usize)];
        let mut found_observation = false;
        while let Some((id, depth)) = stack.pop() {
            if !visited.insert(id.clone()) {
                continue;
            }
            if depth > max_depth {
                return Err(CoverageError::LineageDepthExceeded);
            }
            let node = self
                .nodes
                .get(&id)
                .ok_or_else(|| CoverageError::UnknownLineageNode(id.clone()))?;
            if trace.len() >= max_nodes {
                return Err(CoverageError::LineageNodeLimitExceeded);
            }
            trace.push(node.clone());
            if node.kind == LineageNodeKind::Observation {
                found_observation = true;
            } else if node.parent_ids.is_empty() {
                return Err(CoverageError::LineageMissingObservation);
            }
            for parent_id in node.parent_ids.iter().rev() {
                stack.push((parent_id.clone(), depth.saturating_add(1)));
            }
        }
        if !found_observation {
            return Err(CoverageError::LineageMissingObservation);
        }
        Ok(trace)
    }
}

/// Coverage and lineage invariant failure.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum CoverageError {
    /// A required key field was blank or non-ASCII.
    #[error("coverage key fields must be non-empty ASCII")]
    InvalidKey,
    /// Gap cause was blank.
    #[error("coverage gap cause must not be empty")]
    EmptyCause,
    /// Revision zero is reserved.
    #[error("coverage revision must be positive")]
    ZeroRevision,
    /// Revision did not advance.
    #[error("coverage revision {proposed} does not advance current revision {current}")]
    NonMonotonicRevision {
        /// Current revision.
        current: u64,
        /// Proposed revision.
        proposed: u64,
    },
    /// A source/dataset already has an open interval.
    #[error("coverage gap is already open")]
    GapAlreadyOpen,
    /// No matching incomplete interval exists.
    #[error("no incomplete coverage gap is open")]
    NoOpenGap,
    /// Repaired range ended before it began.
    #[error("coverage range end precedes start")]
    InvalidRange,
    /// Closing a gap lacked concrete evidence IDs.
    #[error("recovery requires at least one evidence ID")]
    MissingRecoveryEvidence,
    /// Lineage node identity was blank.
    #[error("lineage node ID must not be empty")]
    EmptyLineageId,
    /// Lineage node identity already exists.
    #[error("duplicate lineage node {0}")]
    DuplicateLineageNode(String),
    /// A source observation incorrectly declared a parent.
    #[error("source observations cannot have lineage parents")]
    ObservationHasParents,
    /// A referenced lineage node was absent.
    #[error("unknown lineage node {0}")]
    UnknownLineageNode(String),
    /// Traversal exceeded the caller's depth bound.
    #[error("lineage depth limit exceeded")]
    LineageDepthExceeded,
    /// Traversal exceeded the caller's node bound.
    #[error("lineage node limit exceeded")]
    LineageNodeLimitExceeded,
    /// A derived branch did not terminate in a source observation.
    #[error("lineage did not terminate in an observation")]
    LineageMissingObservation,
}

fn coverage_key(
    chain: impl Into<String>,
    network: impl Into<String>,
    dataset: impl Into<String>,
    source_id: impl Into<String>,
) -> Result<CoverageKey, CoverageError> {
    let key = CoverageKey {
        chain: chain.into(),
        network: network.into(),
        dataset: dataset.into(),
        source_id: source_id.into(),
    };
    if [
        key.chain.as_str(),
        key.network.as_str(),
        key.dataset.as_str(),
        key.source_id.as_str(),
    ]
    .iter()
    .any(|value| value.trim().is_empty() || !value.is_ascii())
    {
        return Err(CoverageError::InvalidKey);
    }
    Ok(key)
}
