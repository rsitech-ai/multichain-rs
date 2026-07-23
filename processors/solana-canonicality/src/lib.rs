#![doc = "Solana fork commitments and reversible selected-account projections."]

use std::collections::{BTreeMap, HashMap, HashSet};

use solana_domain::{AccountWrite, ForkId, Lamports, Pubkey, Signature, TransactionKey};
use thiserror::Error;

/// Chain-native Solana observation/commitment state.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Commitment {
    /// Observed before a validator reported processed.
    Received,
    /// Processed by the source validator.
    Processed,
    /// Confirmed by cluster voting evidence.
    Confirmed,
    /// Rooted/finalized by the source.
    Finalized,
    /// Fork was abandoned.
    Dead,
}

impl Commitment {
    const fn rank(self) -> Option<u8> {
        match self {
            Self::Received => Some(0),
            Self::Processed => Some(1),
            Self::Confirmed => Some(2),
            Self::Finalized => Some(3),
            Self::Dead => None,
        }
    }

    /// Stable storage/API encoding.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Received => "received",
            Self::Processed => "processed",
            Self::Confirmed => "confirmed",
            Self::Finalized => "finalized",
            Self::Dead => "dead",
        }
    }
}

/// Append-only commitment transition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitmentRevision {
    revision: u64,
    fork_id: ForkId,
    from: Commitment,
    to: Commitment,
}

impl CommitmentRevision {
    /// Global revision sequence.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Fork whose status changed.
    #[must_use]
    pub const fn fork_id(&self) -> &ForkId {
        &self.fork_id
    }

    /// Prior status.
    #[must_use]
    pub const fn from(&self) -> Commitment {
        self.from
    }

    /// New status.
    #[must_use]
    pub const fn to(&self) -> Commitment {
        self.to
    }
}

#[derive(Clone, Debug)]
struct ForkNode {
    parent: Option<ForkId>,
    commitment: Commitment,
    transactions: HashSet<TransactionKey>,
    writes: Vec<AccountWrite>,
    last_write_version: Option<u64>,
    applied_undo: Vec<AccountUndo>,
}

#[derive(Clone, Debug)]
struct AccountUndo {
    pubkey: [u8; 32],
    previous: Option<AccountState>,
}

/// Current selected-account value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountState {
    owner: Pubkey,
    lamports: Lamports,
    data: Vec<u8>,
    executable: bool,
    rent_epoch: u64,
    write_version: u64,
}

impl AccountState {
    fn from_write(write: &AccountWrite) -> Self {
        Self {
            owner: *write.owner(),
            lamports: write.lamports(),
            data: write.data().to_vec(),
            executable: write.executable(),
            rent_epoch: write.rent_epoch(),
            write_version: write.write_version(),
        }
    }

    /// Owner program.
    #[must_use]
    pub const fn owner(&self) -> &Pubkey {
        &self.owner
    }

    /// Exact lamports.
    #[must_use]
    pub const fn lamports(&self) -> Lamports {
        self.lamports
    }

    /// Exact account bytes.
    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Executable flag.
    #[must_use]
    pub const fn executable(&self) -> bool {
        self.executable
    }

    /// Rent epoch.
    #[must_use]
    pub const fn rent_epoch(&self) -> u64 {
        self.rent_epoch
    }

    /// Validator write version.
    #[must_use]
    pub const fn write_version(&self) -> u64 {
        self.write_version
    }
}

/// Fork graph, append-only commitment history, and one reversible current
/// selected-account projection.
#[derive(Clone, Debug, Default)]
pub struct SolanaCanonicality {
    nodes: HashMap<ForkId, ForkNode>,
    root: Option<ForkId>,
    active_tip: Option<ForkId>,
    accounts: BTreeMap<[u8; 32], AccountState>,
    revision: u64,
}

impl SolanaCanonicality {
    /// Creates empty state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a fork-qualified slot.
    ///
    /// # Errors
    ///
    /// Rejects inconsistent duplicates, unknown/dead parents, non-increasing
    /// child slots, and a second unrelated root.
    pub fn observe_slot(
        &mut self,
        fork_id: ForkId,
        parent: Option<ForkId>,
    ) -> Result<(), SolanaCanonicalityError> {
        if let Some(existing) = self.nodes.get(&fork_id) {
            return if existing.parent == parent {
                Ok(())
            } else {
                Err(SolanaCanonicalityError::ConflictingForkIdentity)
            };
        }
        if let Some(parent_id) = &parent {
            let parent_node = self
                .nodes
                .get(parent_id)
                .ok_or(SolanaCanonicalityError::UnknownParent)?;
            if parent_node.commitment == Commitment::Dead {
                return Err(SolanaCanonicalityError::DeadParent);
            }
            if fork_id.slot() <= parent_id.slot() {
                return Err(SolanaCanonicalityError::NonIncreasingSlot);
            }
        } else if self.root.is_some() {
            return Err(SolanaCanonicalityError::MultipleRoots);
        }
        if parent.is_none() {
            self.root = Some(fork_id.clone());
        }
        self.nodes.insert(
            fork_id,
            ForkNode {
                parent,
                commitment: Commitment::Received,
                transactions: HashSet::new(),
                writes: Vec::new(),
                last_write_version: None,
                applied_undo: Vec::new(),
            },
        );
        Ok(())
    }

    /// Applies a monotonic commitment transition.
    ///
    /// # Errors
    ///
    /// Rejects regression, unknown forks, dead-fork revival, and finalized
    /// fork death.
    pub fn observe_commitment(
        &mut self,
        fork_id: &ForkId,
        commitment: Commitment,
    ) -> Result<Option<CommitmentRevision>, SolanaCanonicalityError> {
        if commitment == Commitment::Dead {
            return self.mark_dead(fork_id);
        }
        let current = self
            .nodes
            .get(fork_id)
            .ok_or(SolanaCanonicalityError::UnknownFork)?
            .commitment;
        if current == commitment {
            return Ok(None);
        }
        if current == Commitment::Dead {
            return Err(SolanaCanonicalityError::DeadFork);
        }
        if commitment.rank() < current.rank() {
            return Err(SolanaCanonicalityError::CommitmentRegression);
        }
        self.next_revision()?;
        self.nodes
            .get_mut(fork_id)
            .ok_or(SolanaCanonicalityError::UnknownFork)?
            .commitment = commitment;
        Ok(Some(CommitmentRevision {
            revision: self.revision,
            fork_id: fork_id.clone(),
            from: current,
            to: commitment,
        }))
    }

    /// Marks a non-finalized fork and every descendant dead. If the active
    /// projection is within that subtree, it is first reverted to the dead
    /// fork's parent.
    ///
    /// # Errors
    ///
    /// Rejects unknown forks and finalized fork death.
    pub fn mark_dead(
        &mut self,
        fork_id: &ForkId,
    ) -> Result<Option<CommitmentRevision>, SolanaCanonicalityError> {
        let current = self
            .nodes
            .get(fork_id)
            .ok_or(SolanaCanonicalityError::UnknownFork)?
            .commitment;
        if current == Commitment::Dead {
            return Ok(None);
        }
        if current == Commitment::Finalized {
            return Err(SolanaCanonicalityError::FinalizedForkWouldDie);
        }
        let dead = self
            .nodes
            .keys()
            .filter(|candidate| self.is_ancestor(fork_id, candidate))
            .cloned()
            .collect::<Vec<_>>();
        if dead.iter().any(|candidate| {
            self.nodes
                .get(candidate)
                .is_some_and(|node| node.commitment == Commitment::Finalized)
        }) {
            return Err(SolanaCanonicalityError::FinalizedForkWouldDie);
        }
        let parent = self
            .nodes
            .get(fork_id)
            .ok_or(SolanaCanonicalityError::UnknownFork)?
            .parent
            .clone();
        if self
            .active_tip
            .as_ref()
            .is_some_and(|active| self.is_ancestor(fork_id, active))
        {
            self.activate_optional(parent.as_ref())?;
        }
        for candidate in dead {
            if let Some(node) = self.nodes.get_mut(&candidate) {
                node.commitment = Commitment::Dead;
            }
        }
        self.next_revision()?;
        Ok(Some(CommitmentRevision {
            revision: self.revision,
            fork_id: fork_id.clone(),
            from: current,
            to: Commitment::Dead,
        }))
    }

    /// Records an executed transaction under its fork-qualified identity.
    ///
    /// # Errors
    ///
    /// Rejects unknown/dead forks.
    pub fn record_transaction(
        &mut self,
        key: TransactionKey,
    ) -> Result<bool, SolanaCanonicalityError> {
        let node = self
            .nodes
            .get_mut(key.fork_id())
            .ok_or(SolanaCanonicalityError::UnknownFork)?;
        if node.commitment == Commitment::Dead {
            return Err(SolanaCanonicalityError::DeadFork);
        }
        Ok(node.transactions.insert(key))
    }

    /// Whether an exact fork-qualified execution is known.
    #[must_use]
    pub fn contains_transaction(&self, key: &TransactionKey) -> bool {
        self.nodes
            .get(key.fork_id())
            .is_some_and(|node| node.transactions.contains(key))
    }

    /// Records one selected account write and applies it immediately only when
    /// its fork is the active tip.
    ///
    /// # Errors
    ///
    /// Rejects unknown/dead forks and non-increasing write versions without
    /// mutating the journal or current state.
    pub fn record_account_write(
        &mut self,
        write: AccountWrite,
    ) -> Result<(), SolanaCanonicalityError> {
        let fork_id = write.fork_id().clone();
        {
            let node = self
                .nodes
                .get(&fork_id)
                .ok_or(SolanaCanonicalityError::UnknownFork)?;
            if node.commitment == Commitment::Dead {
                return Err(SolanaCanonicalityError::DeadFork);
            }
            if node
                .last_write_version
                .is_some_and(|version| write.write_version() <= version)
            {
                return Err(SolanaCanonicalityError::WriteVersionRegression);
            }
        }
        let active = self.active_tip.as_ref() == Some(&fork_id);
        if active {
            let undo = self.apply_write(&write);
            if let Some(node) = self.nodes.get_mut(&fork_id) {
                node.applied_undo.push(undo);
            }
        }
        let node = self
            .nodes
            .get_mut(&fork_id)
            .ok_or(SolanaCanonicalityError::UnknownFork)?;
        node.last_write_version = Some(write.write_version());
        node.writes.push(write);
        Ok(())
    }

    /// Switches the current selected-account projection to a live fork by
    /// reverting the old suffix and applying the new suffix.
    ///
    /// # Errors
    ///
    /// Rejects unknown or dead target forks.
    pub fn activate(&mut self, fork_id: &ForkId) -> Result<(), SolanaCanonicalityError> {
        self.activate_optional(Some(fork_id))
    }

    fn activate_optional(
        &mut self,
        target: Option<&ForkId>,
    ) -> Result<(), SolanaCanonicalityError> {
        if let Some(target) = target {
            let node = self
                .nodes
                .get(target)
                .ok_or(SolanaCanonicalityError::UnknownFork)?;
            if node.commitment == Commitment::Dead {
                return Err(SolanaCanonicalityError::DeadFork);
            }
        }
        if self.active_tip.as_ref() == target {
            return Ok(());
        }
        let current_path = self
            .active_tip
            .as_ref()
            .map_or_else(Vec::new, |tip| self.path_to_root(tip));
        let target_path = target.map_or_else(Vec::new, |tip| self.path_to_root(tip));
        let common = current_path
            .iter()
            .find(|candidate| target_path.contains(candidate))
            .cloned();

        for fork in current_path
            .iter()
            .take_while(|candidate| Some(*candidate) != common.as_ref())
        {
            self.revert_node(fork)?;
        }
        let mut connect = target_path
            .iter()
            .take_while(|candidate| Some(*candidate) != common.as_ref())
            .cloned()
            .collect::<Vec<_>>();
        connect.reverse();
        for fork in connect {
            self.apply_node(&fork)?;
        }
        self.active_tip = target.cloned();
        Ok(())
    }

    fn path_to_root(&self, tip: &ForkId) -> Vec<ForkId> {
        let mut path = Vec::new();
        let mut current = Some(tip.clone());
        while let Some(fork) = current {
            path.push(fork.clone());
            current = self.nodes.get(&fork).and_then(|node| node.parent.clone());
        }
        path
    }

    fn apply_node(&mut self, fork_id: &ForkId) -> Result<(), SolanaCanonicalityError> {
        let writes = self
            .nodes
            .get(fork_id)
            .ok_or(SolanaCanonicalityError::UnknownFork)?
            .writes
            .clone();
        let mut undo = Vec::with_capacity(writes.len());
        for write in writes {
            undo.push(self.apply_write(&write));
        }
        let node = self
            .nodes
            .get_mut(fork_id)
            .ok_or(SolanaCanonicalityError::UnknownFork)?;
        node.applied_undo = undo;
        Ok(())
    }

    fn apply_write(&mut self, write: &AccountWrite) -> AccountUndo {
        let pubkey = write.pubkey().to_bytes();
        let previous = self
            .accounts
            .insert(pubkey, AccountState::from_write(write));
        AccountUndo { pubkey, previous }
    }

    fn revert_node(&mut self, fork_id: &ForkId) -> Result<(), SolanaCanonicalityError> {
        let undo = std::mem::take(
            &mut self
                .nodes
                .get_mut(fork_id)
                .ok_or(SolanaCanonicalityError::UnknownFork)?
                .applied_undo,
        );
        for operation in undo.into_iter().rev() {
            match operation.previous {
                Some(previous) => {
                    self.accounts.insert(operation.pubkey, previous);
                }
                None => {
                    self.accounts.remove(&operation.pubkey);
                }
            }
        }
        Ok(())
    }

    fn is_ancestor(&self, ancestor: &ForkId, descendant: &ForkId) -> bool {
        self.path_to_root(descendant).contains(ancestor)
    }

    fn next_revision(&mut self) -> Result<(), SolanaCanonicalityError> {
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or(SolanaCanonicalityError::RevisionOverflow)?;
        Ok(())
    }

    /// Current active projection tip.
    #[must_use]
    pub const fn active_tip(&self) -> Option<&ForkId> {
        self.active_tip.as_ref()
    }

    /// Current commitment for a fork.
    #[must_use]
    pub fn commitment(&self, fork_id: &ForkId) -> Option<Commitment> {
        self.nodes.get(fork_id).map(|node| node.commitment)
    }

    /// Current selected-account state.
    #[must_use]
    pub fn account(&self, pubkey: &Pubkey) -> Option<&AccountState> {
        self.accounts.get(&pubkey.to_bytes())
    }

    /// Deterministic current projection digest.
    #[must_use]
    pub fn state_hash(&self) -> [u8; 32] {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"solana-account-state/v1");
        for (pubkey, state) in &self.accounts {
            hasher.update(pubkey);
            hasher.update(state.owner.as_ref());
            hasher.update(&state.lamports.value().to_le_bytes());
            hasher.update(&(state.data.len() as u64).to_le_bytes());
            hasher.update(&state.data);
            hasher.update(&[u8::from(state.executable)]);
            hasher.update(&state.rent_epoch.to_le_bytes());
            hasher.update(&state.write_version.to_le_bytes());
        }
        *hasher.finalize().as_bytes()
    }
}

/// Evidence that a missing fork/block was reconstructed from another source.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(clippy::struct_field_names)]
pub struct ReconstructionEvidence {
    missing_source_id: String,
    recovery_source_id: String,
    fork_id: ForkId,
    recovery_observation_id: [u8; 32],
}

impl ReconstructionEvidence {
    /// Creates explicit independent-source recovery evidence.
    ///
    /// # Errors
    ///
    /// Rejects blank/equal sources and a zero observation identity.
    pub fn new(
        missing_source_id: impl Into<String>,
        recovery_source_id: impl Into<String>,
        fork_id: ForkId,
        recovery_observation_id: [u8; 32],
    ) -> Result<Self, SolanaCanonicalityError> {
        let missing_source_id = missing_source_id.into();
        let recovery_source_id = recovery_source_id.into();
        if missing_source_id.trim().is_empty()
            || recovery_source_id.trim().is_empty()
            || missing_source_id == recovery_source_id
        {
            return Err(SolanaCanonicalityError::InvalidRecoverySources);
        }
        if recovery_observation_id == [0; 32] {
            return Err(SolanaCanonicalityError::InvalidRecoveryObservation);
        }
        Ok(Self {
            missing_source_id,
            recovery_source_id,
            fork_id,
            recovery_observation_id,
        })
    }

    /// Missing source.
    #[must_use]
    pub fn missing_source_id(&self) -> &str {
        &self.missing_source_id
    }

    /// Independent recovery source.
    #[must_use]
    pub fn recovery_source_id(&self) -> &str {
        &self.recovery_source_id
    }

    /// Reconstructed fork identity.
    #[must_use]
    pub const fn fork_id(&self) -> &ForkId {
        &self.fork_id
    }

    /// Observation proving the recovery bytes.
    #[must_use]
    pub const fn recovery_observation_id(&self) -> &[u8; 32] {
        &self.recovery_observation_id
    }
}

/// Pre-execution signal that remains non-executed until explicitly joined to
/// a fork-qualified runtime result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreExecutionObservation {
    source_id: String,
    signature: Signature,
    observed_at_unix_ns: i64,
    transaction_key: Option<TransactionKey>,
}

impl PreExecutionObservation {
    /// Creates a pre-execution signal.
    ///
    /// # Errors
    ///
    /// Rejects blank source IDs and negative receipt times.
    pub fn new(
        source_id: impl Into<String>,
        signature: Signature,
        observed_at_unix_ns: i64,
    ) -> Result<Self, SolanaCanonicalityError> {
        let source_id = source_id.into();
        if source_id.trim().is_empty() {
            return Err(SolanaCanonicalityError::InvalidPreExecutionSource);
        }
        if observed_at_unix_ns < 0 {
            return Err(SolanaCanonicalityError::InvalidObservedTime);
        }
        Ok(Self {
            source_id,
            signature,
            observed_at_unix_ns,
            transaction_key: None,
        })
    }

    /// Joins a runtime execution only when its signature matches.
    ///
    /// # Errors
    ///
    /// Rejects mismatches and a second join.
    pub fn join_execution(
        mut self,
        transaction_key: TransactionKey,
    ) -> Result<Self, SolanaCanonicalityError> {
        if self.transaction_key.is_some() {
            return Err(SolanaCanonicalityError::ExecutionAlreadyJoined);
        }
        if &self.signature != transaction_key.signature() {
            return Err(SolanaCanonicalityError::ExecutionSignatureMismatch);
        }
        self.transaction_key = Some(transaction_key);
        Ok(self)
    }

    /// Whether runtime execution evidence was joined.
    #[must_use]
    pub const fn is_executed(&self) -> bool {
        self.transaction_key.is_some()
    }

    /// Fork-qualified execution when joined.
    #[must_use]
    pub const fn transaction_key(&self) -> Option<&TransactionKey> {
        self.transaction_key.as_ref()
    }

    /// Exact source identity.
    #[must_use]
    pub fn source_id(&self) -> &str {
        &self.source_id
    }

    /// Local first-seen time.
    #[must_use]
    pub const fn observed_at_unix_ns(&self) -> i64 {
        self.observed_at_unix_ns
    }
}

/// Invalid Solana canonicality or account transition.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum SolanaCanonicalityError {
    /// Fork identity was reused with another parent.
    #[error("conflicting fork identity")]
    ConflictingForkIdentity,
    /// Parent fork is not known.
    #[error("unknown parent fork")]
    UnknownParent,
    /// Parent is already dead.
    #[error("cannot attach to a dead parent")]
    DeadParent,
    /// Child slot must be greater than parent slot.
    #[error("child slot is not greater than parent slot")]
    NonIncreasingSlot,
    /// Only one root is accepted per state instance.
    #[error("multiple unrelated roots")]
    MultipleRoots,
    /// Fork is not known.
    #[error("unknown fork")]
    UnknownFork,
    /// Commitment cannot regress.
    #[error("commitment regression")]
    CommitmentRegression,
    /// Dead fork cannot receive new facts.
    #[error("fork is dead")]
    DeadFork,
    /// Finalized fork death is a critical invariant failure.
    #[error("finalized fork would become dead")]
    FinalizedForkWouldDie,
    /// Account write version is duplicate or regressing.
    #[error("account write version is not increasing")]
    WriteVersionRegression,
    /// Revision counter exhausted.
    #[error("revision counter overflow")]
    RevisionOverflow,
    /// Reconstruction sources are not independent.
    #[error("invalid reconstruction sources")]
    InvalidRecoverySources,
    /// Reconstruction must cite a real observation.
    #[error("invalid reconstruction observation")]
    InvalidRecoveryObservation,
    /// Pre-execution source is blank.
    #[error("invalid pre-execution source")]
    InvalidPreExecutionSource,
    /// Observation time cannot be negative.
    #[error("invalid observed time")]
    InvalidObservedTime,
    /// Pre-execution signal already has execution evidence.
    #[error("execution already joined")]
    ExecutionAlreadyJoined,
    /// Joined runtime signature differs from pre-execution signature.
    #[error("execution signature mismatch")]
    ExecutionSignatureMismatch,
}
