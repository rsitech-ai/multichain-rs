use evm_decoder::{
    DecodeStatus, DecoderDeployment, DecoderError, DecoderRegistry, NativeDecodeSubject,
};
use evm_domain::Address;

#[test]
fn historical_decoder_ranges_are_non_overlapping_and_chain_qualified() {
    let address = Address::from([1; 20]);
    let mut registry = DecoderRegistry::new();
    registry
        .register(DecoderDeployment::new(1, address, 0, Some(99), "erc20-v1").expect("deployment"))
        .expect("register");
    registry
        .register(DecoderDeployment::new(1, address, 100, None, "erc20-v2").expect("deployment"))
        .expect("register");

    assert_eq!(
        registry.resolve(1, address, 50).expect("v1").version(),
        "erc20-v1"
    );
    assert_eq!(
        registry.resolve(1, address, 100).expect("v2").version(),
        "erc20-v2"
    );
    assert!(registry.resolve(56, address, 100).is_none());
    assert!(matches!(
        registry.register(
            DecoderDeployment::new(1, address, 90, Some(110), "overlap").expect("deployment"),
        ),
        Err(DecoderError::OverlappingDeployment)
    ));
}

#[test]
fn decoder_failure_retains_native_bytes_and_does_not_block_replay() {
    let address = Address::from([1; 20]);
    let mut registry = DecoderRegistry::new();
    registry
        .register(DecoderDeployment::new(1, address, 0, None, "erc20-v1").expect("deployment"))
        .expect("register");
    let subject = NativeDecodeSubject::new(
        1,
        address,
        10,
        "native-fact-1",
        [0xa9, 0x05, 0x9c, 0xbb],
        vec![0xde, 0xad],
    )
    .expect("subject");
    let failed = registry
        .record_outcome(&subject, Err("malformed ABI word".to_owned()), 1)
        .expect("failure revision");
    assert_eq!(failed.status, DecodeStatus::Failed);
    assert_eq!(failed.raw_data_hex, "dead");
    assert_eq!(failed.native_fact_id, "native-fact-1");

    let decoded = registry
        .record_outcome(&subject, Ok(serde_json::json!({"amount":"42"})), 2)
        .expect("replay revision");
    assert_eq!(decoded.status, DecodeStatus::Decoded);
    assert_eq!(decoded.revision, 2);
    assert_eq!(decoded.replay_key, failed.replay_key);
}

#[test]
fn unknown_contracts_remain_queryable_without_fabricated_decode() {
    let registry = DecoderRegistry::new();
    let subject = NativeDecodeSubject::new(
        56,
        Address::from([2; 20]),
        1,
        "native-fact-2",
        [1, 2, 3, 4],
        vec![5, 6],
    )
    .expect("subject");
    let unknown = registry
        .record_unknown(&subject, 1)
        .expect("unknown revision");
    assert_eq!(unknown.status, DecodeStatus::Unknown);
    assert_eq!(unknown.raw_data_hex, "0506");
}
