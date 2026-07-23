use std::str::FromStr as _;

use solana_domain::{
    Blockhash, ForkId, Lamports, Pubkey, Signature, Slot, SolanaNetwork, TransactionKey,
};

#[test]
fn mainnet_identity_uses_official_fixed_width_primitives() {
    assert_eq!(SolanaNetwork::MainnetBeta.as_str(), "solana-mainnet-beta");

    let pubkey = Pubkey::from_str("11111111111111111111111111111111").expect("system program");
    let signature = Signature::from_str(&"1".repeat(64)).expect("zero signature");
    let blockhash = Blockhash::from_str("11111111111111111111111111111111").expect("zero hash");

    assert_eq!(pubkey.to_bytes().len(), 32);
    assert_eq!(signature.as_ref().len(), 64);
    assert_eq!(blockhash.to_bytes().len(), 32);
    assert!(Pubkey::from_str("too-short").is_err());
    assert!(Signature::from_str("111").is_err());
    assert!(Blockhash::from_str("111").is_err());
}

#[test]
fn transaction_identity_is_signature_plus_fork_context() {
    let signature = Signature::from([7_u8; 64]);
    let first = TransactionKey::new(
        signature,
        ForkId::new(Slot::new(42), Blockhash::new_from_array([1; 32])),
    );
    let competing = TransactionKey::new(
        signature,
        ForkId::new(Slot::new(42), Blockhash::new_from_array([2; 32])),
    );

    assert_ne!(first, competing);
    assert_eq!(first.signature(), competing.signature());
    assert_ne!(first.fork_id(), competing.fork_id());
}

#[test]
fn lamports_are_exact_decimal_strings() {
    let maximum = Lamports::new(u64::MAX);
    assert_eq!(
        serde_json::to_string(&maximum).expect("serialize"),
        format!("\"{}\"", u64::MAX)
    );
    assert_eq!(
        serde_json::from_str::<Lamports>(&format!("\"{}\"", u64::MAX))
            .expect("deserialize")
            .value(),
        u64::MAX
    );
    assert!(serde_json::from_str::<Lamports>(&u64::MAX.to_string()).is_err());
    assert!(serde_json::from_str::<Lamports>("\"18446744073709551616\"").is_err());
    assert!(serde_json::from_str::<Lamports>("\"-1\"").is_err());
}
