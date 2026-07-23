use archive_format::{ArchiveBatch, ArchiveError, decode_archive};
use observation_envelope::{CollectorSequence, ObservationBuilder, SourceSessionId};
use platform_proto::observation::CommittedObservation;

fn committed(sequence: u64, channel: &str) -> CommittedObservation {
    let session = SourceSessionId::try_from([0x42_u8; 16].as_slice()).expect("valid session");
    let observation = ObservationBuilder::new()
        .source_id("btc-observer-test-1")
        .source_session_id(session)
        .collector_sequence(CollectorSequence::new(sequence))
        .chain("bitcoin")
        .network("mainnet")
        .channel(channel)
        .source_message_type(channel)
        .observed_at_unix_ns(
            1_784_808_000_000_000_000 + i64::try_from(sequence).expect("test sequence fits i64"),
        )
        .observed_at_monotonic_ns(10_000 + sequence)
        .payload(format!("payload-{sequence}"))
        .build()
        .expect("valid observation");

    CommittedObservation {
        observation: Some(observation),
        durable_at_unix_ns: 1_784_808_000_000_000_000,
        wal_commit_hash: vec![0x11; 32],
    }
}

#[test]
fn framed_archive_round_trips_exact_committed_observations() {
    let records = vec![committed(7, "rawtx"), committed(8, "rawtx")];
    let archive = ArchiveBatch::try_new(records.clone())
        .expect("valid batch")
        .encode()
        .expect("archive encodes");

    assert_eq!(
        archive.object_key(),
        "raw/chain=bitcoin/network=mainnet/source=btc-observer-test-1/channel=rawtx/\
         date=2026-07-23/hour=12/part-42424242424242424242424242424242-7-8.bin.zst"
    );
    assert_eq!(archive.record_count(), 2);
    assert_eq!(
        decode_archive(archive.compressed_bytes()).expect("archive decodes"),
        records
    );
}

#[test]
fn archive_batch_rejects_non_contiguous_or_mixed_ranges() {
    assert!(matches!(
        ArchiveBatch::try_new(vec![committed(7, "rawtx"), committed(9, "rawtx")]),
        Err(ArchiveError::NonContiguousRange {
            expected: 8,
            actual: 9
        })
    ));

    assert!(matches!(
        ArchiveBatch::try_new(vec![committed(7, "rawtx"), committed(8, "sequence")]),
        Err(ArchiveError::MixedBatchField { field: "channel" })
    ));
}

#[test]
fn truncated_framed_archive_fails_closed() {
    let mut bytes = ArchiveBatch::try_new(vec![committed(7, "rawtx")])
        .expect("valid batch")
        .encode()
        .expect("archive encodes")
        .into_compressed_bytes();
    bytes.truncate(bytes.len() - 3);

    assert!(decode_archive(&bytes).is_err());
}
