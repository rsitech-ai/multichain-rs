mod common;

use bitcoin::{Network, consensus::serialize};
use bitcoin_canonicality::{BlockTransition, CanonicalityState, StateError};
use bitcoin_domain::parse_block;
use chain_domain::BitcoinNetwork;
use common::{invalidate_pow, mine_child, regtest_genesis};

#[test]
fn heavier_branch_disconnects_old_tip_before_connecting_new_branch() {
    let genesis = regtest_genesis();
    let main_1 = mine_child(&genesis, 1);
    let alt_1 = mine_child(&genesis, 2);
    let alt_2 = mine_child(&alt_1, 3);
    let mut state = CanonicalityState::new(BitcoinNetwork::Regtest);

    assert_eq!(
        state.observe_block(genesis.clone()).expect("genesis"),
        vec![BlockTransition::connected(genesis.block_hash(), 0, 1)]
    );
    assert_eq!(
        state.observe_block(main_1.clone()).expect("main block"),
        vec![BlockTransition::connected(main_1.block_hash(), 1, 2)]
    );
    assert!(
        state
            .observe_block(alt_1.clone())
            .expect("side block")
            .is_empty()
    );
    assert_eq!(
        state.observe_block(alt_2.clone()).expect("heavier branch"),
        vec![
            BlockTransition::disconnected(main_1.block_hash(), 1, 3),
            BlockTransition::connected(alt_1.block_hash(), 1, 4),
            BlockTransition::connected(alt_2.block_hash(), 2, 5),
        ]
    );
    assert_eq!(state.canonical_tip(), Some(alt_2.block_hash()));
    assert!(state.observe_block(alt_2).expect("duplicate").is_empty());
}

#[test]
fn invalid_pow_and_unknown_parent_never_enter_candidate_dag() {
    let genesis = regtest_genesis();
    let invalid = invalidate_pow(&mine_child(&genesis, 1));
    let orphan_parent = mine_child(&genesis, 2);
    let orphan = mine_child(&orphan_parent, 3);
    let mut state = CanonicalityState::new(BitcoinNetwork::Regtest);

    assert!(matches!(
        state.observe_block(invalid),
        Err(StateError::InvalidProofOfWork { .. })
    ));
    assert!(matches!(
        state.observe_block(orphan),
        Err(StateError::UnknownParent { .. })
    ));
    assert_eq!(state.block_count(), 0);
}

#[test]
fn mainnet_genesis_is_network_bound_and_pow_validated() {
    let mainnet_genesis = parse_block(&serialize(&bitcoin::blockdata::constants::genesis_block(
        Network::Bitcoin,
    )))
    .expect("mainnet genesis");
    let mut mainnet = CanonicalityState::new(BitcoinNetwork::Mainnet);
    assert_eq!(
        mainnet
            .observe_block(mainnet_genesis.clone())
            .expect("mainnet genesis"),
        vec![BlockTransition::connected(
            mainnet_genesis.block_hash(),
            0,
            1
        )]
    );

    let mut regtest = CanonicalityState::new(BitcoinNetwork::Regtest);
    assert!(matches!(
        regtest.observe_block(mainnet_genesis),
        Err(StateError::InvalidGenesis { .. })
    ));
}
