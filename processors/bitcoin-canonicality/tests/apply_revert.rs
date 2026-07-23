mod common;

use bitcoin::{Block, OutPoint as NativeOutPoint, consensus::deserialize, hashes::Hash as _};
use bitcoin_canonicality::{StateError, UtxoEvent, UtxoState};
use bitcoin_domain::OutPoint;
use common::{coinbase, mine_block, regtest_genesis, spend};

#[test]
fn apply_then_reverse_restores_exact_state_hash() {
    let genesis = regtest_genesis();
    let genesis_native: Block = deserialize(genesis.consensus_bytes()).expect("genesis");
    let genesis_outpoint = NativeOutPoint {
        txid: genesis_native.txdata[0].compute_txid(),
        vout: 0,
    };
    let payment = spend(genesis_outpoint, &[4_999_999_000]);
    let payment_txid = payment.compute_txid();
    let child = mine_block(&genesis, 1, vec![coinbase(1, 5_000_000_000), payment]);
    let mut state = UtxoState::new();
    let empty_hash = state.state_hash();

    state.apply_block(&genesis).expect("apply genesis");
    let after_genesis = state.state_hash();
    let events = state.apply_block(&child).expect("apply child");

    assert_eq!(events.len(), 3);
    assert!(events.iter().any(|event| matches!(
        event,
        UtxoEvent::Spent {
            spending_txid,
            input_index: 0,
            ..
        } if spending_txid.as_bytes() == payment_txid.as_byte_array()
    )));
    assert_eq!(state.utxo_count(), 2);

    let reverted = state
        .revert_block(child.block_hash())
        .expect("reverse child");
    assert_eq!(reverted.len(), events.len());
    assert_eq!(state.state_hash(), after_genesis);
    state
        .revert_block(genesis.block_hash())
        .expect("reverse genesis");
    assert_eq!(state.state_hash(), empty_hash);
}

#[test]
fn invalid_spends_are_atomic_and_out_of_order_disconnects_fail() {
    let genesis = regtest_genesis();
    let genesis_native: Block = deserialize(genesis.consensus_bytes()).expect("genesis");
    let genesis_outpoint = NativeOutPoint {
        txid: genesis_native.txdata[0].compute_txid(),
        vout: 0,
    };
    let mut state = UtxoState::new();
    state.apply_block(&genesis).expect("apply genesis");
    let before = state.state_hash();

    let missing = spend(
        NativeOutPoint {
            txid: bitcoin::Txid::from_byte_array([7; 32]),
            vout: 0,
        },
        &[1],
    );
    let missing_block = mine_block(&genesis, 2, vec![coinbase(2, 1), missing]);
    assert!(matches!(
        state.apply_block(&missing_block),
        Err(StateError::MissingPrevout { .. })
    ));
    assert_eq!(state.state_hash(), before);

    let double_spend = mine_block(
        &genesis,
        3,
        vec![
            coinbase(3, 1),
            spend(genesis_outpoint, &[4_999_999_000]),
            spend(genesis_outpoint, &[4_999_998_000]),
        ],
    );
    assert!(matches!(
        state.apply_block(&double_spend),
        Err(StateError::MissingPrevout { .. })
    ));
    assert_eq!(state.state_hash(), before);

    let negative_fee = mine_block(
        &genesis,
        4,
        vec![coinbase(4, 1), spend(genesis_outpoint, &[5_000_000_001])],
    );
    assert!(matches!(
        state.apply_block(&negative_fee),
        Err(StateError::NegativeFee { .. })
    ));
    assert_eq!(state.state_hash(), before);

    let child = mine_block(
        &genesis,
        5,
        vec![coinbase(5, 1), spend(genesis_outpoint, &[4_999_999_000])],
    );
    let grandchild = mine_block(&child, 6, vec![coinbase(6, 1)]);
    state.apply_block(&child).expect("apply child");
    state.apply_block(&grandchild).expect("apply grandchild");
    assert!(matches!(
        state.revert_block(child.block_hash()),
        Err(StateError::OutOfOrderDisconnect { .. })
    ));
}

#[test]
fn public_outpoints_preserve_consensus_identity() {
    let native = NativeOutPoint {
        txid: bitcoin::Txid::from_byte_array([9; 32]),
        vout: 42,
    };
    let domain = OutPoint::from_consensus_bytes(&bitcoin::consensus::serialize(&native))
        .expect("fixed outpoint");
    assert_eq!(
        domain.consensus_bytes(),
        bitcoin::consensus::serialize(&native).as_slice()
    );
}
