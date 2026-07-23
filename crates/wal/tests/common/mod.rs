use observation_envelope::{CollectorSequence, ObservationBuilder, SourceSessionId};

pub fn session() -> SourceSessionId {
    SourceSessionId::try_from([0x42_u8; 16].as_slice()).expect("valid session")
}

pub fn observation(
    sequence: u64,
    payload: impl AsRef<[u8]>,
) -> platform_proto::observation::Observation {
    ObservationBuilder::new()
        .source_id("btc-observer-test-1")
        .source_session_id(session())
        .collector_sequence(CollectorSequence::new(sequence))
        .chain("bitcoin")
        .network("mainnet")
        .channel("rawtx")
        .source_message_type("rawtx")
        .observed_at_unix_ns(1_784_808_000_000_000_000)
        .observed_at_monotonic_ns(10_000 + sequence)
        .payload(payload)
        .build()
        .expect("valid observation")
}
