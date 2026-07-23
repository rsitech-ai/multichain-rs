use std::str::FromStr as _;

use solana_decoder::{DecodeStatus, DecoderDeployment, DecoderRegistry};
use solana_domain::Pubkey;

#[test]
fn token_and_token_2022_deployments_are_program_and_slot_qualified() {
    let registry = DecoderRegistry::standard_v1().expect("standard registry");
    let token = Pubkey::from_str("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA").expect("token");
    let token_2022 =
        Pubkey::from_str("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb").expect("token-2022");

    assert_eq!(
        registry
            .deployment_for(&token, 100)
            .expect("token")
            .decoder_version(),
        "spl-token/v1"
    );
    assert_eq!(
        registry
            .deployment_for(&token_2022, 100)
            .expect("token-2022")
            .decoder_version(),
        "spl-token-2022/v1"
    );
}

#[test]
fn overlaps_fail_and_decoder_failure_remains_an_append_only_revision() {
    let program = Pubkey::new_from_array([7; 32]);
    let mut registry = DecoderRegistry::new();
    registry
        .register(DecoderDeployment::new(program, "custom/v1", 10, Some(20)).expect("deployment"))
        .expect("register");
    assert!(
        registry
            .register(DecoderDeployment::new(program, "custom/v2", 20, None).expect("overlap"))
            .is_err()
    );
    let revision = registry
        .decode(
            &program,
            15,
            "0:inner:none",
            &[0xde, 0xad],
            [9; 32],
            1_000,
            |_| Err("decoder crashed".to_owned()),
        )
        .expect("failure revision");

    assert_eq!(revision.revision(), 1);
    assert_eq!(revision.status(), DecodeStatus::Failed);
    assert_eq!(revision.raw_data(), &[0xde, 0xad]);
    assert_eq!(revision.native_fact_id(), &[9; 32]);
    assert_eq!(revision.error(), Some("decoder crashed"));
    let row = revision.fact_row();
    assert_eq!(row.slot, 15);
    assert_eq!(row.decode_status, "failed");
    assert_eq!(row.error.as_deref(), Some("decoder crashed"));
    assert_eq!(row.raw_data_hex, "dead");
    assert_eq!(row.native_fact_id, "09".repeat(32));
}

#[test]
fn unknown_program_is_retained_without_fabricated_decode_and_can_replay() {
    let program = Pubkey::new_from_array([8; 32]);
    let mut registry = DecoderRegistry::new();
    let unknown = registry
        .decode(&program, 1, "0:inner:none", &[1, 2, 3], [7; 32], 1, |_| {
            Ok(serde_json::json!({"must_not": "run"}))
        })
        .expect("unknown revision");
    assert_eq!(unknown.status(), DecodeStatus::Unknown);
    assert!(unknown.decoded_json().is_none());

    registry
        .register(DecoderDeployment::new(program, "custom/v1", 0, None).expect("deployment"))
        .expect("register");
    let decoded = registry
        .decode(&program, 1, "0:inner:none", &[1, 2, 3], [7; 32], 2, |_| {
            Ok(serde_json::json!({"kind": "custom"}))
        })
        .expect("decoded revision");
    assert_eq!(decoded.revision(), 2);
    assert_eq!(decoded.status(), DecodeStatus::Decoded);
    assert_eq!(decoded.decoded_json(), Some(r#"{"kind":"custom"}"#));
}
