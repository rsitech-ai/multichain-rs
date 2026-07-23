#![doc = "Coordinates broker publication, verified raw archival, and coverage."]

use archive_format::{ArchiveBatch, ArchiveManifest};
use platform_proto::observation::CommittedObservation;
use storage_ports::{
    ArchiveError, BrokerError, BrokerPublisher, CheckpointError, CheckpointKind, CheckpointStore,
    DurableCheckpoint, ManifestAck, RAW_BITCOIN_OBSERVATION_TOPIC, RawArchive,
};
use thiserror::Error;

/// Successful durable publication and archive coverage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArchiveOutcome {
    /// Committed manifest acknowledgement.
    pub manifest: ManifestAck,
    /// Inclusive sequence covered by both sinks.
    pub last_collector_sequence: u64,
}

/// Orchestrates the ordered visibility transitions for WAL-committed records.
pub struct ArchiveCoordinator<B, A, C> {
    broker: B,
    archive: A,
    checkpoints: C,
}

impl<B, A, C> ArchiveCoordinator<B, A, C>
where
    B: BrokerPublisher,
    A: RawArchive,
    C: CheckpointStore,
{
    /// Constructs an archive coordinator from external storage ports.
    #[must_use]
    pub const fn new(broker: B, archive: A, checkpoints: C) -> Self {
        Self {
            broker,
            archive,
            checkpoints,
        }
    }

    /// Publishes committed observations, verifies their exact archive object,
    /// commits the manifest, and advances each checkpoint only after its own
    /// durable acknowledgement.
    ///
    /// # Errors
    ///
    /// Returns [`ArchiveWriterError`] at the first failed visibility boundary.
    /// A broker checkpoint may therefore exist without archive coverage, which
    /// deliberately keeps the WAL segment non-reclaimable.
    pub async fn publish_and_archive(
        &self,
        records: Vec<CommittedObservation>,
    ) -> Result<ArchiveOutcome, ArchiveWriterError> {
        let encoded = ArchiveBatch::try_new(records.clone())?.encode()?;
        let source_id = encoded.source_id().to_owned();
        let source_session_id = encoded.source_session_id();

        let broker_ack = self
            .broker
            .publish(RAW_BITCOIN_OBSERVATION_TOPIC, &records)
            .await?;
        if broker_ack.source_session_id() != source_session_id
            || broker_ack.last_collector_sequence() != encoded.last_collector_sequence()
        {
            return Err(ArchiveWriterError::BrokerAckMismatch);
        }
        self.checkpoints
            .advance(
                CheckpointKind::Broker,
                &source_id,
                DurableCheckpoint::new(source_session_id, broker_ack.last_collector_sequence()),
            )
            .await?;

        let previous_manifest_hash = self.archive.latest_manifest_hash(source_session_id).await?;
        let manifest = ArchiveManifest::from_encoded(&encoded, previous_manifest_hash)?;
        let staged = self.archive.stage(encoded).await?;
        self.archive.verify(&staged).await?;
        let manifest_ack = self.archive.commit_manifest(manifest).await?;
        self.checkpoints
            .advance(
                CheckpointKind::Archive,
                &source_id,
                DurableCheckpoint::new(source_session_id, manifest_ack.last_collector_sequence()),
            )
            .await?;

        Ok(ArchiveOutcome {
            manifest: manifest_ack,
            last_collector_sequence: broker_ack.last_collector_sequence(),
        })
    }
}

/// Archive workflow failures with source boundary context preserved.
#[derive(Debug, Error)]
pub enum ArchiveWriterError {
    /// Batch validation or framed archive encoding failed.
    #[error("archive format failed: {0}")]
    Format(#[from] archive_format::ArchiveError),
    /// Manifest construction failed.
    #[error("archive manifest failed: {0}")]
    Manifest(#[from] archive_format::ManifestError),
    /// Broker publication failed.
    #[error("broker publication failed: {0}")]
    Broker(#[from] BrokerError),
    /// Broker acknowledgement did not cover the encoded range.
    #[error("broker acknowledgement did not match the archive range")]
    BrokerAckMismatch,
    /// Raw object or manifest handling failed.
    #[error("raw archive failed: {0}")]
    Archive(#[from] ArchiveError),
    /// Durable coverage persistence failed.
    #[error("checkpoint failed: {0}")]
    Checkpoint(#[from] CheckpointError),
}
