use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

use bitcoin_domain::{BitcoinTransaction, OutPoint, Txid};

use crate::MempoolError;

/// Replacement evidence available when a transaction is added.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplacementEvidence {
    /// No direct source evidence; conflicts remain inferred.
    None,
    /// Source directly named the replaced transaction.
    Direct {
        /// Replaced transaction.
        replaced_txid: Txid,
    },
    /// Snapshot reconciliation plus confirmed conflicting spend.
    Reconciled {
        /// Replaced transaction.
        replaced_txid: Txid,
    },
}

/// Strength of a replacement relationship.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplacementClassification {
    /// Direct source replacement evidence.
    Observed,
    /// Snapshot diff plus confirmed conflict.
    Reconciled,
    /// Conflict and ordering evidence only.
    Inferred,
    /// Removal exists without sufficient replacement evidence.
    Unknown,
}

/// Source-qualified conflicting-spend relationship.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConflictEdge {
    source_id: String,
    spent_outpoint: OutPoint,
    txid: Txid,
    conflicting_txid: Txid,
}

impl ConflictEdge {
    /// Returns source identity.
    #[must_use]
    pub fn source_id(&self) -> &str {
        &self.source_id
    }

    /// Returns the multiply spent outpoint.
    #[must_use]
    pub const fn spent_outpoint(&self) -> OutPoint {
        self.spent_outpoint
    }

    /// Returns the newly added transaction.
    #[must_use]
    pub const fn txid(&self) -> Txid {
        self.txid
    }

    /// Returns the previously known conflicting transaction.
    #[must_use]
    pub const fn conflicting_txid(&self) -> Txid {
        self.conflicting_txid
    }
}

/// Result of adding one transaction to the source graph.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GraphUpdate {
    conflicts: Vec<ConflictEdge>,
    replacement_classification: Option<ReplacementClassification>,
}

impl GraphUpdate {
    /// Returns deterministic conflict edges created by the addition.
    #[must_use]
    pub fn conflicts(&self) -> &[ConflictEdge] {
        &self.conflicts
    }

    /// Returns replacement confidence when conflicts exist.
    #[must_use]
    pub const fn replacement_classification(&self) -> Option<ReplacementClassification> {
        self.replacement_classification
    }
}

/// Exact integer fee-rate fraction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FeeRate {
    fee_sats: u64,
    vsize: u64,
}

impl FeeRate {
    /// Returns numerator satoshis.
    #[must_use]
    pub const fn fee_sats(self) -> u64 {
        self.fee_sats
    }

    /// Returns denominator virtual bytes.
    #[must_use]
    pub const fn vsize(self) -> u64 {
        self.vsize
    }

    /// Returns the integer floor in sat/vbyte.
    #[must_use]
    pub const fn sats_per_vbyte_floor(self) -> u64 {
        self.fee_sats / self.vsize
    }
}

/// Deterministic current package/cluster economics.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PackageSnapshot {
    member_txids: Vec<Txid>,
    total_fee_sats: u64,
    total_vsize: u64,
    effective_fee_rate: FeeRate,
}

impl PackageSnapshot {
    /// Returns lexicographically sorted current members.
    #[must_use]
    pub fn member_txids(&self) -> Vec<Txid> {
        self.member_txids.clone()
    }

    /// Returns checked total fees.
    #[must_use]
    pub const fn total_fee_sats(&self) -> u64 {
        self.total_fee_sats
    }

    /// Returns checked total virtual size.
    #[must_use]
    pub const fn total_vsize(&self) -> u64 {
        self.total_vsize
    }

    /// Returns the exact effective fee-rate fraction.
    #[must_use]
    pub const fn effective_fee_rate(&self) -> FeeRate {
        self.effective_fee_rate
    }
}

#[derive(Clone, Debug)]
struct GraphTransaction {
    transaction: BitcoinTransaction,
    fee_sats: u64,
    vsize: u64,
}

/// Source-local transaction history, conflicts, and current package graph.
#[derive(Clone, Debug)]
pub struct MempoolGraph {
    source_id: String,
    transactions: HashMap<Txid, GraphTransaction>,
    present: HashSet<Txid>,
    spent_by_history: HashMap<OutPoint, BTreeSet<Txid>>,
}

impl MempoolGraph {
    /// Creates an empty source-qualified graph.
    ///
    /// # Errors
    ///
    /// Rejects a blank source identity.
    pub fn new(source_id: impl Into<String>) -> Result<Self, MempoolError> {
        let source_id = source_id.into();
        if source_id.trim().is_empty() {
            return Err(MempoolError::EmptySourceId);
        }
        Ok(Self {
            source_id,
            transactions: HashMap::new(),
            present: HashSet::new(),
            spent_by_history: HashMap::new(),
        })
    }

    /// Adds or reaccepts one transaction and returns new conflict evidence.
    ///
    /// # Errors
    ///
    /// Rejects oversized transactions and direct evidence that does not name a
    /// real conflicting spend.
    pub fn add(
        &mut self,
        transaction: BitcoinTransaction,
        fee_sats: u64,
        evidence: ReplacementEvidence,
    ) -> Result<GraphUpdate, MempoolError> {
        let txid = transaction.txid();
        if self.present.contains(&txid) {
            return Ok(GraphUpdate::default());
        }
        let vsize = u64::try_from(transaction.virtual_size())
            .map_err(|_| MempoolError::TransactionTooLarge { txid })?;
        let mut conflicts = BTreeMap::<(OutPoint, Txid), ConflictEdge>::new();
        for input in transaction.inputs() {
            if let Some(existing) = self.spent_by_history.get(&input.previous_output) {
                for conflicting_txid in existing.iter().copied().filter(|known| *known != txid) {
                    conflicts.insert(
                        (input.previous_output, conflicting_txid),
                        ConflictEdge {
                            source_id: self.source_id.clone(),
                            spent_outpoint: input.previous_output,
                            txid,
                            conflicting_txid,
                        },
                    );
                }
            }
        }
        let named_replacement = match evidence {
            ReplacementEvidence::None => None,
            ReplacementEvidence::Direct { replaced_txid }
            | ReplacementEvidence::Reconciled { replaced_txid } => Some(replaced_txid),
        };
        if named_replacement.is_some_and(|named| {
            !conflicts
                .values()
                .any(|edge| edge.conflicting_txid == named)
        }) {
            return Err(MempoolError::InvalidReplacementEvidence { txid });
        }
        let replacement_classification = if conflicts.is_empty() {
            None
        } else {
            Some(match evidence {
                ReplacementEvidence::None => ReplacementClassification::Inferred,
                ReplacementEvidence::Direct { .. } => ReplacementClassification::Observed,
                ReplacementEvidence::Reconciled { .. } => ReplacementClassification::Reconciled,
            })
        };

        for input in transaction.inputs() {
            self.spent_by_history
                .entry(input.previous_output)
                .or_default()
                .insert(txid);
        }
        self.transactions.insert(
            txid,
            GraphTransaction {
                transaction,
                fee_sats,
                vsize,
            },
        );
        self.present.insert(txid);
        Ok(GraphUpdate {
            conflicts: conflicts.into_values().collect(),
            replacement_classification,
        })
    }

    /// Marks one known transaction absent while retaining graph history.
    pub fn remove(&mut self, txid: Txid) {
        self.present.remove(&txid);
    }

    /// Computes the current connected parent/child package containing `txid`.
    ///
    /// # Errors
    ///
    /// Rejects absent roots, checked fee/vsize overflow, and zero size.
    pub fn package(&self, txid: Txid) -> Result<PackageSnapshot, MempoolError> {
        if !self.present.contains(&txid) {
            return Err(MempoolError::UnknownTransaction { txid });
        }
        let adjacency = self.current_adjacency();
        let mut members = BTreeSet::new();
        let mut queue = VecDeque::from([txid]);
        while let Some(member) = queue.pop_front() {
            if !members.insert(member) {
                continue;
            }
            if let Some(neighbors) = adjacency.get(&member) {
                queue.extend(neighbors.iter().copied());
            }
        }

        let (total_fee_sats, total_vsize) =
            members
                .iter()
                .try_fold((0_u64, 0_u64), |(fees, vsize), member| {
                    let transaction = self
                        .transactions
                        .get(member)
                        .ok_or(MempoolError::UnknownTransaction { txid: *member })?;
                    Ok((
                        fees.checked_add(transaction.fee_sats)
                            .ok_or(MempoolError::FeeOverflow)?,
                        vsize
                            .checked_add(transaction.vsize)
                            .ok_or(MempoolError::VirtualSizeOverflow)?,
                    ))
                })?;
        if total_vsize == 0 {
            return Err(MempoolError::ZeroVirtualSize);
        }
        Ok(PackageSnapshot {
            member_txids: members.into_iter().collect(),
            total_fee_sats,
            total_vsize,
            effective_fee_rate: FeeRate {
                fee_sats: total_fee_sats,
                vsize: total_vsize,
            },
        })
    }

    fn current_adjacency(&self) -> HashMap<Txid, BTreeSet<Txid>> {
        let mut adjacency = HashMap::<Txid, BTreeSet<Txid>>::new();
        for child in &self.present {
            let Some(transaction) = self.transactions.get(child) else {
                continue;
            };
            for input in transaction.transaction.inputs() {
                let parent = input.previous_output.txid;
                if self.present.contains(&parent) {
                    adjacency.entry(*child).or_default().insert(parent);
                    adjacency.entry(parent).or_default().insert(*child);
                }
            }
        }
        adjacency
    }
}
