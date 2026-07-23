use bitcoin_domain::{OutPoint, ParseError, Txid, parse_block, parse_transaction};

#[test]
fn malformed_and_truncated_inputs_fail_closed_without_panicking() {
    for name in ["truncated_transaction.hex", "invalid_compact_size.hex"] {
        let bytes = fixture(name);
        let result = std::panic::catch_unwind(|| parse_transaction(&bytes));
        assert!(result.is_ok(), "{name} panicked");
        assert!(result.expect("no panic").is_err(), "{name} parsed");
    }
    assert!(matches!(
        parse_block(&fixture("invalid_merkle_block.hex")),
        Err(ParseError::MerkleRootMismatch)
    ));
}

#[test]
fn consensus_round_trips_and_outpoint_encoding_is_fixed() {
    for name in [
        "legacy_transaction.hex",
        "segwit_v0_transaction.hex",
        "taproot_transaction.hex",
        "large_witness.hex",
    ] {
        let bytes = fixture(name);
        let parsed = parse_transaction(&bytes).expect(name);
        assert_eq!(parsed.consensus_bytes(), bytes);
    }

    let outpoint = OutPoint {
        txid: Txid::from_bytes([0x42; 32]),
        vout: u32::MAX,
    };
    assert_eq!(
        OutPoint::from_consensus_bytes(&outpoint.consensus_bytes()).expect("outpoint"),
        outpoint
    );
    assert!(OutPoint::from_consensus_bytes(&[0; 35]).is_err());
}

#[test]
fn bounded_arbitrary_inputs_never_panic() {
    for length in 0..512 {
        let bytes = (0..length)
            .map(|index| u8::try_from((index * 31 + length) % 256).expect("byte"))
            .collect::<Vec<_>>();
        assert!(std::panic::catch_unwind(|| parse_transaction(&bytes)).is_ok());
        assert!(std::panic::catch_unwind(|| parse_block(&bytes)).is_ok());
    }
}

fn fixture(name: &str) -> Vec<u8> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/bitcoin/objects")
        .join(name);
    let text = std::fs::read_to_string(root).expect("fixture");
    let text = text.trim();
    (0..text.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&text[index..index + 2], 16).expect("fixture hex"))
        .collect()
}
