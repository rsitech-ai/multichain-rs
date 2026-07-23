use observation_envelope::SourceSessionId;
use prost::Message as _;
use solana_yellowstone_connector::{
    CapturedUpdate, SessionCursor, UpdateKind, YellowstoneConnectorError,
};
use yellowstone_grpc_proto::prelude::{
    SlotStatus, SubscribeUpdate, SubscribeUpdateSlot, subscribe_update::UpdateOneof,
};

#[test]
fn exact_yellowstone_protobuf_is_retained_before_interpretation() {
    let wire = SubscribeUpdate {
        filters: vec!["commitment-transitions".to_owned()],
        update_oneof: Some(UpdateOneof::Slot(SubscribeUpdateSlot {
            slot: 42,
            parent: Some(41),
            status: SlotStatus::SlotProcessed as i32,
            dead_error: None,
        })),
        created_at: None,
    }
    .encode_to_vec();
    let session_id = SourceSessionId::try_from([3_u8; 16].as_slice()).expect("session");
    let captured = CapturedUpdate::decode("solana-eu-a", session_id, 7, 1_000, &wire)
        .expect("captured update");

    assert_eq!(captured.exact_protobuf(), wire);
    assert_eq!(captured.source_id(), "solana-eu-a");
    assert_eq!(captured.source_sequence(), 7);
    assert_eq!(captured.observed_at_unix_ns(), 1_000);
    assert_eq!(captured.kind(), UpdateKind::Slot);
    assert_eq!(captured.slot(), Some(42));
}

#[test]
fn malformed_empty_and_oversized_messages_fail_closed() {
    let session_id = SourceSessionId::try_from([4_u8; 16].as_slice()).expect("session");
    assert!(CapturedUpdate::decode("source", session_id, 1, 1, &[0xff, 0xff]).is_err());
    assert!(
        CapturedUpdate::decode(
            "source",
            session_id,
            1,
            1,
            &SubscribeUpdate::default().encode_to_vec()
        )
        .is_err()
    );
    assert!(matches!(
        CapturedUpdate::decode("source", session_id, 1, 1, &vec![0; 33_554_433]),
        Err(YellowstoneConnectorError::MessageTooLarge(_))
    ));
}

#[test]
fn cursor_detects_gaps_and_reconnects_under_a_new_session() {
    let first = SourceSessionId::try_from([5_u8; 16].as_slice()).expect("session");
    let second = SourceSessionId::try_from([6_u8; 16].as_slice()).expect("session");
    let mut cursor = SessionCursor::start("solana-eu-a", first).expect("cursor");

    assert!(cursor.observe(1).expect("first sequence").is_none());
    let gap = cursor.observe(3).expect("gap accepted").expect("gap");
    assert_eq!(gap.expected_sequence(), 2);
    assert_eq!(gap.observed_sequence(), 3);
    assert!(cursor.observe(3).is_err());

    cursor.reconnect(second).expect("new session");
    assert!(cursor.observe(1).expect("new first sequence").is_none());
    assert_eq!(cursor.source_session_id(), second);
    assert!(cursor.reconnect(second).is_err());
}
