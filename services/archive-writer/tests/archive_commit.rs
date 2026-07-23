use archive_format::{ArchiveBatch, ArchiveManifest};
use archive_writer::ArchiveCoordinator;
use observation_envelope::{CollectorSequence, ObservationBuilder, SourceSessionId};
use platform_proto::observation::CommittedObservation;
use storage_adapters::{MemoryBroker, MemoryCheckpointStore, MemoryRawArchive};
use storage_ports::{
    CheckpointKind, DurableCheckpoint, ReclaimBlocker, SealedWalSegment, ensure_reclaimable,
};

fn committed(sequence: u64) -> CommittedObservation {
    let session = SourceSessionId::try_from([0x42_u8; 16].as_slice()).expect("valid session");
    let observation = ObservationBuilder::new()
        .source_id("btc-observer-test-1")
        .source_session_id(session)
        .collector_sequence(CollectorSequence::new(sequence))
        .chain("bitcoin")
        .network("mainnet")
        .channel("rawtx")
        .source_message_type("rawtx")
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

#[tokio::test]
async fn staged_object_without_manifest_is_not_replayable() {
    let archive = MemoryRawArchive::default();
    let encoded = ArchiveBatch::try_new(vec![committed(7)])
        .expect("valid batch")
        .encode()
        .expect("archive encodes");
    let staged = archive.stage_encoded(encoded).await.expect("object stages");

    assert!(
        archive
            .replay_by_object_key(staged.object_key())
            .await
            .is_none()
    );
}

#[tokio::test]
async fn checksum_mismatch_prevents_manifest_commit() {
    let archive = MemoryRawArchive::default();
    let encoded = ArchiveBatch::try_new(vec![committed(7)])
        .expect("valid batch")
        .encode()
        .expect("archive encodes");
    let staged = archive
        .stage_encoded(encoded.clone())
        .await
        .expect("object stages");
    archive
        .replace_staged_bytes(staged.object_key(), b"corrupt".to_vec())
        .await;
    let manifest = ArchiveManifest::from_encoded(&encoded, None).expect("valid manifest");

    assert!(archive.verify(&staged).await.is_err());
    assert!(archive.commit_manifest(manifest).await.is_err());
}

#[tokio::test]
async fn manifest_commit_is_idempotent_and_overlap_safe() {
    let archive = MemoryRawArchive::default();
    let encoded = ArchiveBatch::try_new(vec![committed(7), committed(8)])
        .expect("valid batch")
        .encode()
        .expect("archive encodes");
    let staged = archive
        .stage_encoded(encoded.clone())
        .await
        .expect("object stages");
    archive.verify(&staged).await.expect("object verifies");
    let manifest = ArchiveManifest::from_encoded(&encoded, None).expect("valid manifest");

    let first = archive
        .commit_manifest(manifest.clone())
        .await
        .expect("first commit succeeds");
    let repeated = archive
        .commit_manifest(manifest)
        .await
        .expect("same manifest is idempotent");
    assert_eq!(first, repeated);

    let overlap = ArchiveBatch::try_new(vec![committed(8), committed(9)])
        .expect("valid batch")
        .encode()
        .expect("archive encodes");
    let overlap_staged = archive
        .stage_encoded(overlap.clone())
        .await
        .expect("overlap stages");
    archive
        .verify(&overlap_staged)
        .await
        .expect("overlap verifies");
    let overlap_manifest =
        ArchiveManifest::from_encoded(&overlap, Some(first.manifest_hash())).expect("manifest");

    assert!(archive.commit_manifest(overlap_manifest).await.is_err());
}

#[tokio::test]
async fn coordinator_full_replay_reuses_committed_manifest() {
    let archive = MemoryRawArchive::default();
    let checkpoints = MemoryCheckpointStore::default();
    let coordinator =
        ArchiveCoordinator::new(MemoryBroker::default(), archive, checkpoints.clone());
    let records = vec![committed(7), committed(8)];

    let first = coordinator
        .publish_and_archive(records.clone())
        .await
        .expect("first publish");
    let replay = coordinator
        .publish_and_archive(records)
        .await
        .expect("full replay is idempotent");

    assert_eq!(first.manifest, replay.manifest);
    assert_eq!(
        checkpoints
            .load(CheckpointKind::Archive, "btc-observer-test-1", [0x42; 16])
            .await
            .expect("checkpoint"),
        Some(DurableCheckpoint::new([0x42; 16], 8))
    );
}

#[tokio::test]
async fn broker_ack_without_archive_coverage_does_not_permit_reclaim() {
    let broker = MemoryBroker::default();
    let archive = MemoryRawArchive::withhold_manifest_commits();
    let checkpoints = MemoryCheckpointStore::default();
    let coordinator = ArchiveCoordinator::new(broker, archive, checkpoints.clone());

    assert!(
        coordinator
            .publish_and_archive(vec![committed(7), committed(8)])
            .await
            .is_err()
    );

    let broker_checkpoint = checkpoints
        .load(CheckpointKind::Broker, "btc-observer-test-1", [0x42; 16])
        .await
        .expect("checkpoint load");
    let archive_checkpoint = checkpoints
        .load(CheckpointKind::Archive, "btc-observer-test-1", [0x42; 16])
        .await
        .expect("checkpoint load");
    let segment = SealedWalSegment::new([0x42; 16], 8);

    assert_eq!(
        ensure_reclaimable(
            &segment,
            broker_checkpoint.as_ref(),
            archive_checkpoint.as_ref()
        ),
        Err(ReclaimBlocker::MissingArchiveCheckpoint)
    );
}

#[tokio::test]
async fn checkpoint_coverage_is_independent_across_source_sessions() {
    let checkpoints = MemoryCheckpointStore::default();
    checkpoints
        .advance(
            CheckpointKind::Broker,
            "btc-observer-test-1",
            DurableCheckpoint::new([0x42; 16], 8),
        )
        .await
        .expect("first session advances");
    checkpoints
        .advance(
            CheckpointKind::Broker,
            "btc-observer-test-1",
            DurableCheckpoint::new([0x43; 16], 0),
        )
        .await
        .expect("new source session has independent coverage");
}

#[test]
fn checkpoint_regression_and_session_mismatch_fail_closed() {
    let segment = SealedWalSegment::new([0x42; 16], 8);
    let broker = DurableCheckpoint::new([0x42; 16], 8);
    let wrong_archive = DurableCheckpoint::new([0x24; 16], 9);

    assert_eq!(
        ensure_reclaimable(&segment, Some(&broker), Some(&wrong_archive)),
        Err(ReclaimBlocker::SourceSessionMismatch {
            checkpoint: CheckpointKind::Archive
        })
    );

    assert!(
        DurableCheckpoint::advance(Some(&broker), DurableCheckpoint::new([0x42; 16], 7)).is_err()
    );
}
