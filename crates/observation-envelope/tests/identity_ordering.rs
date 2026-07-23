use std::{fmt::Write as _, fs, path::PathBuf};

use observation_envelope::{
    CollectorSequence, ObservationBuilder, ObservationError, SourceSessionId,
};

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/observation/interleaved-zmq.jsonl")
}

fn lowercase_hex(bytes: &[u8]) -> String {
    bytes.iter().fold(
        String::with_capacity(bytes.len() * 2),
        |mut output, byte| {
            write!(output, "{byte:02x}").expect("writing to a String cannot fail");
            output
        },
    )
}

fn build_fixture() -> Vec<platform_proto::observation::Observation> {
    let session = SourceSessionId::try_from(
        [
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
            0xee, 0xff,
        ]
        .as_slice(),
    )
    .expect("fixed session ID");

    fs::read_to_string(fixture_path())
        .expect("fixture is readable")
        .lines()
        .enumerate()
        .map(|(index, line)| {
            let value: serde_json::Value = serde_json::from_str(line).expect("valid JSONL row");
            ObservationBuilder::new()
                .source_id("btc-observer-waw-1")
                .source_session_id(session)
                .collector_sequence(CollectorSequence::new(index as u64))
                .chain("bitcoin")
                .network("mainnet")
                .channel(value["channel"].as_str().expect("channel"))
                .source_message_type(value["source_message_type"].as_str().expect("message type"))
                .observed_at_unix_ns(1_784_808_000_000_000_000)
                .observed_at_monotonic_ns(50_000 + index as u64)
                .payload(value["payload"].as_str().expect("payload").as_bytes())
                .build()
                .expect("valid fixture observation")
        })
        .collect()
}

#[test]
fn interleaved_channels_have_one_replay_stable_total_order() {
    let first_run = build_fixture();
    let replay = build_fixture();

    assert_eq!(
        first_run
            .iter()
            .map(|record| record.collector_sequence)
            .collect::<Vec<_>>(),
        vec![0, 1, 2, 3]
    );
    assert_eq!(
        first_run
            .iter()
            .map(|record| record.observation_id.clone())
            .collect::<Vec<_>>(),
        replay
            .iter()
            .map(|record| record.observation_id.clone())
            .collect::<Vec<_>>()
    );
    assert_ne!(first_run[0].observation_id, first_run[1].observation_id);
    assert!(
        first_run
            .iter()
            .all(|record| record.observation_id.len() == 32)
    );
    assert!(
        first_run
            .iter()
            .all(|record| record.payload_hash.len() == 32)
    );
    assert_eq!(
        lowercase_hex(&first_run[0].payload_hash),
        "1111aedacc2694db8a0ad9cdcffb6960b808b9d83b4b2259235c735435843127"
    );
    assert_eq!(
        lowercase_hex(&first_run[0].observation_id),
        "aac819b5661872cc63649a8d91ae527b2373a0dfc7f02c66cb71dc7b502f78c2"
    );
}

#[test]
fn builder_rejects_empty_required_text_fields() {
    let session = SourceSessionId::try_from([7_u8; 16].as_slice()).expect("session ID");

    for (field, builder) in [
        (
            "source_id",
            ObservationBuilder::new()
                .source_id("")
                .source_session_id(session)
                .collector_sequence(CollectorSequence::new(0))
                .chain("bitcoin")
                .network("mainnet")
                .channel("rawtx")
                .source_message_type("rawtx")
                .observed_at_unix_ns(1)
                .observed_at_monotonic_ns(1)
                .payload(b"payload"),
        ),
        (
            "network",
            ObservationBuilder::new()
                .source_id("observer")
                .source_session_id(session)
                .collector_sequence(CollectorSequence::new(0))
                .chain("bitcoin")
                .network("")
                .channel("rawtx")
                .source_message_type("rawtx")
                .observed_at_unix_ns(1)
                .observed_at_monotonic_ns(1)
                .payload(b"payload"),
        ),
        (
            "channel",
            ObservationBuilder::new()
                .source_id("observer")
                .source_session_id(session)
                .collector_sequence(CollectorSequence::new(0))
                .chain("bitcoin")
                .network("mainnet")
                .channel("")
                .source_message_type("rawtx")
                .observed_at_unix_ns(1)
                .observed_at_monotonic_ns(1)
                .payload(b"payload"),
        ),
    ] {
        assert_eq!(
            builder.build(),
            Err(ObservationError::EmptyField(field)),
            "{field} should be required"
        );
    }
}

#[test]
fn source_session_id_rejects_invalid_lengths() {
    assert!(SourceSessionId::try_from([0_u8; 15].as_slice()).is_err());
    assert!(SourceSessionId::try_from([0_u8; 17].as_slice()).is_err());
}
