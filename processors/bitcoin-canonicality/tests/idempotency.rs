mod common;

use bitcoin::{OutPoint, hashes::Hash as _};
use bitcoin_canonicality::{BitcoinState, StateError};
use chain_domain::BitcoinNetwork;
use common::{coinbase, mine_block, regtest_genesis, spend};

#[test]
fn duplicate_delivery_is_a_noop_and_invalid_block_is_atomic() {
    let genesis = regtest_genesis();
    let mut state = BitcoinState::new(BitcoinNetwork::Regtest);
    let initial = state.observe_block(genesis.clone()).expect("genesis");
    assert_eq!(initial.len(), 1);
    let after_genesis = state.state_hash();

    assert!(
        state
            .observe_block(genesis.clone())
            .expect("duplicate")
            .is_empty()
    );
    assert_eq!(state.state_hash(), after_genesis);
    assert_eq!(state.candidate_block_count(), 1);

    let invalid = mine_block(
        &genesis,
        1,
        vec![
            coinbase(1, 1),
            spend(
                OutPoint {
                    txid: bitcoin::Txid::from_byte_array([8; 32]),
                    vout: 0,
                },
                &[1],
            ),
        ],
    );
    assert!(matches!(
        state.observe_block(invalid),
        Err(StateError::MissingPrevout { .. })
    ));
    assert_eq!(state.state_hash(), after_genesis);
    assert_eq!(state.candidate_block_count(), 1);
}

#[test]
fn checkpoint_binds_offset_tip_revision_and_state_hash() {
    let genesis = regtest_genesis();
    let mut state = BitcoinState::new(BitcoinNetwork::Regtest);
    state.observe_block(genesis.clone()).expect("genesis");

    let checkpoint = state.checkpoint(42);
    assert_eq!(checkpoint.consumer_offset(), 42);
    assert_eq!(checkpoint.canonical_tip(), Some(genesis.block_hash()));
    assert_eq!(checkpoint.revision(), 1);
    assert_eq!(checkpoint.state_hash(), state.state_hash());
}
