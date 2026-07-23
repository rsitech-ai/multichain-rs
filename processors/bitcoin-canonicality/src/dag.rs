use std::collections::{HashMap, HashSet};

use bitcoin_domain::{BitcoinBlock, BlockHash, BlockWork};
use chain_domain::BitcoinNetwork;

use crate::{BlockTransition, StateError};

#[derive(Clone, Debug)]
struct DagNode {
    block: BitcoinBlock,
    parent: Option<BlockHash>,
    height: u32,
    cumulative_work: BlockWork,
}

/// In-memory candidate DAG with deterministic heaviest-chain selection.
#[derive(Clone, Debug)]
pub struct CanonicalityState {
    network: BitcoinNetwork,
    nodes: HashMap<BlockHash, DagNode>,
    canonical_tip: Option<BlockHash>,
    next_revision: u64,
}

impl CanonicalityState {
    /// Creates empty state for one Bitcoin network.
    #[must_use]
    pub fn new(network: BitcoinNetwork) -> Self {
        Self {
            network,
            nodes: HashMap::new(),
            canonical_tip: None,
            next_revision: 1,
        }
    }

    /// Validates and adds a block, returning ordered canonical transitions.
    ///
    /// Equal-work side branches do not replace the current tip.
    ///
    /// # Errors
    ///
    /// Rejects invalid genesis blocks, missing parents, invalid proof of work,
    /// missing retarget boundaries, work overflow, or internal DAG corruption.
    pub fn observe_block(
        &mut self,
        block: BitcoinBlock,
    ) -> Result<Vec<BlockTransition>, StateError> {
        let hash = block.block_hash();
        if self.nodes.contains_key(&hash) {
            return Ok(Vec::new());
        }
        if !block.has_valid_pow_for(block.compact_target()) {
            return Err(StateError::InvalidProofOfWork { hash });
        }

        let (parent, height, parent_work, required_target) =
            self.validate_candidate_context(&block)?;
        if !block.has_valid_pow_for(required_target) {
            return Err(StateError::InvalidProofOfWork { hash });
        }
        let cumulative_work = parent_work
            .checked_add(block.work())
            .ok_or(StateError::WorkOverflow { hash })?;
        self.nodes.insert(
            hash,
            DagNode {
                block,
                parent,
                height,
                cumulative_work,
            },
        );

        let should_reorganize = match self.canonical_tip {
            None => true,
            Some(tip) => cumulative_work > self.node(tip)?.cumulative_work,
        };
        if !should_reorganize {
            return Ok(Vec::new());
        }

        self.reorganize_to(hash)
    }

    /// Returns the current canonical tip.
    #[must_use]
    pub const fn canonical_tip(&self) -> Option<BlockHash> {
        self.canonical_tip
    }

    /// Returns the number of accepted candidate blocks.
    #[must_use]
    pub fn block_count(&self) -> usize {
        self.nodes.len()
    }

    pub(crate) fn block(&self, hash: BlockHash) -> Result<BitcoinBlock, StateError> {
        Ok(self.node(hash)?.block.clone())
    }

    pub(crate) const fn revision(&self) -> u64 {
        self.next_revision.saturating_sub(1)
    }

    fn validate_candidate_context(
        &self,
        block: &BitcoinBlock,
    ) -> Result<(Option<BlockHash>, u32, BlockWork, u32), StateError> {
        let hash = block.block_hash();
        if hash == expected_genesis_hash(self.network) {
            return Ok((None, 0, BlockWork::zero(), block.compact_target()));
        }

        let parent_hash = block.previous_block_hash();
        if parent_hash == BlockHash::from_bytes([0; 32]) {
            return Err(StateError::InvalidGenesis { hash });
        }
        let parent = self
            .nodes
            .get(&parent_hash)
            .ok_or(StateError::UnknownParent {
                hash,
                parent: parent_hash,
            })?;
        let height = parent
            .height
            .checked_add(1)
            .ok_or(StateError::WorkOverflow { hash })?;
        let epoch_start = if needs_epoch_boundary(self.network, height) {
            let boundary_height = height - difficulty_interval(self.network);
            Some(
                self.ancestor_at_height(parent_hash, boundary_height)?
                    .block
                    .clone(),
            )
        } else {
            None
        };
        let required_target = parent
            .block
            .next_required_target(self.network, height, epoch_start.as_ref())
            .ok_or_else(|| StateError::MissingDifficultyBoundary {
                hash,
                height: height - difficulty_interval(self.network),
            })?;
        Ok((
            Some(parent_hash),
            height,
            parent.cumulative_work,
            required_target,
        ))
    }

    fn reorganize_to(&mut self, new_tip: BlockHash) -> Result<Vec<BlockTransition>, StateError> {
        let new_ancestors = self.ancestor_set(new_tip)?;
        let mut disconnect = Vec::new();
        let mut cursor = self.canonical_tip;
        while let Some(hash) = cursor {
            if new_ancestors.contains(&hash) {
                break;
            }
            let node = self.node(hash)?;
            disconnect.push((hash, node.height));
            cursor = node.parent;
        }
        let common_ancestor = cursor;

        let mut connect = Vec::new();
        let mut cursor = Some(new_tip);
        while cursor != common_ancestor {
            let hash = cursor.ok_or(StateError::InconsistentDag { hash: new_tip })?;
            let node = self.node(hash)?;
            connect.push((hash, node.height));
            cursor = node.parent;
        }
        connect.reverse();

        let mut transitions = Vec::with_capacity(disconnect.len() + connect.len());
        for (hash, height) in disconnect {
            transitions.push(BlockTransition::disconnected(
                hash,
                height,
                self.take_revision(),
            ));
        }
        for (hash, height) in connect {
            transitions.push(BlockTransition::connected(
                hash,
                height,
                self.take_revision(),
            ));
        }
        self.canonical_tip = Some(new_tip);
        Ok(transitions)
    }

    fn ancestor_set(&self, tip: BlockHash) -> Result<HashSet<BlockHash>, StateError> {
        let mut ancestors = HashSet::new();
        let mut cursor = Some(tip);
        while let Some(hash) = cursor {
            if !ancestors.insert(hash) {
                return Err(StateError::InconsistentDag { hash });
            }
            cursor = self.node(hash)?.parent;
        }
        Ok(ancestors)
    }

    fn ancestor_at_height(&self, tip: BlockHash, height: u32) -> Result<&DagNode, StateError> {
        let mut cursor = tip;
        loop {
            let node = self.node(cursor)?;
            if node.height == height {
                return Ok(node);
            }
            if node.height < height {
                return Err(StateError::InconsistentDag { hash: cursor });
            }
            cursor = node
                .parent
                .ok_or(StateError::InconsistentDag { hash: cursor })?;
        }
    }

    fn node(&self, hash: BlockHash) -> Result<&DagNode, StateError> {
        self.nodes
            .get(&hash)
            .ok_or(StateError::InconsistentDag { hash })
    }

    fn take_revision(&mut self) -> u64 {
        let revision = self.next_revision;
        self.next_revision = self.next_revision.saturating_add(1);
        revision
    }
}

fn expected_genesis_hash(network: BitcoinNetwork) -> BlockHash {
    use bitcoin::hashes::Hash as _;

    let network = match network {
        BitcoinNetwork::Mainnet => bitcoin::Network::Bitcoin,
        BitcoinNetwork::Testnet => bitcoin::Network::Testnet,
        BitcoinNetwork::Signet => bitcoin::Network::Signet,
        BitcoinNetwork::Regtest => bitcoin::Network::Regtest,
    };
    BlockHash::from_bytes(
        bitcoin::blockdata::constants::genesis_block(network)
            .block_hash()
            .to_byte_array(),
    )
}

const fn difficulty_interval(network: BitcoinNetwork) -> u32 {
    match network {
        BitcoinNetwork::Mainnet | BitcoinNetwork::Testnet | BitcoinNetwork::Signet => 2_016,
        BitcoinNetwork::Regtest => 144,
    }
}

const fn needs_epoch_boundary(network: BitcoinNetwork, height: u32) -> bool {
    !matches!(network, BitcoinNetwork::Regtest)
        && height.is_multiple_of(difficulty_interval(network))
}
