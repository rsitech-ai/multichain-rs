mod common;

use bitcoin::{Block, OutPoint, consensus::deserialize};
use bitcoin_canonicality::{BitcoinState, BlockTransition, UtxoEvent};
use chain_domain::BitcoinNetwork;
use common::{coinbase, mine_block, regtest_genesis, spend};

#[test]
fn heavier_branch_reverses_old_utxos_before_applying_new_branch() {
    let genesis = regtest_genesis();
    let native: Block = deserialize(genesis.consensus_bytes()).expect("genesis");
    let genesis_outpoint = OutPoint {
        txid: native.txdata[0].compute_txid(),
        vout: 0,
    };
    let main_1 = mine_block(
        &genesis,
        1,
        vec![
            coinbase(1, 5_000_000_000),
            spend(genesis_outpoint, &[4_999_999_000]),
        ],
    );
    let alt_1 = mine_block(&genesis, 2, vec![coinbase(2, 5_000_000_000)]);
    let alt_2 = mine_block(&alt_1, 3, vec![coinbase(3, 5_000_000_000)]);
    let mut state = BitcoinState::new(BitcoinNetwork::Regtest);

    state.observe_block(genesis.clone()).expect("genesis");
    state.observe_block(main_1.clone()).expect("main branch");
    let main_hash = state.state_hash();
    assert!(
        state
            .observe_block(alt_1.clone())
            .expect("side branch")
            .is_empty()
    );
    assert_eq!(state.state_hash(), main_hash);

    let updates = state.observe_block(alt_2.clone()).expect("reorg");
    assert_eq!(
        updates
            .iter()
            .map(bitcoin_canonicality::StateUpdate::transition)
            .collect::<Vec<_>>(),
        vec![
            BlockTransition::disconnected(main_1.block_hash(), 1, 3),
            BlockTransition::connected(alt_1.block_hash(), 1, 4),
            BlockTransition::connected(alt_2.block_hash(), 2, 5),
        ]
    );
    assert!(matches!(
        updates[0].utxo_events().last(),
        Some(UtxoEvent::CreationReverted { .. })
    ));
    assert_eq!(state.canonical_tip(), Some(alt_2.block_hash()));
    assert_eq!(state.utxo_count(), 3);
}
