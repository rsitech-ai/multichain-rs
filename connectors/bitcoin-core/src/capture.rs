use std::sync::Arc;

use observation_envelope::{Clock, ObservationBuilder};
use platform_proto::observation::CommittedObservation;
use wal::{ObservationWal, UnframedObservation};

use crate::{
    config::BitcoinCoreNetwork, error::CaptureError, session::SourceSession, zmq::ZmqNotification,
};

/// Single-writer durable capture engine.
pub struct CaptureEngine<W> {
    source_id: String,
    network: BitcoinCoreNetwork,
    session: SourceSession,
    clock: Arc<dyn Clock>,
    wal: W,
}

impl<W: ObservationWal> CaptureEngine<W> {
    #[must_use]
    pub fn new(
        source_id: impl Into<String>,
        network: BitcoinCoreNetwork,
        session: SourceSession,
        clock: Arc<dyn Clock>,
        wal: W,
    ) -> Self {
        Self {
            source_id: source_id.into(),
            network,
            session,
            clock,
            wal,
        }
    }

    /// Durably captures exactly one validated notification.
    ///
    /// # Errors
    ///
    /// Returns identity, WAL, or committed-read failures.
    pub fn capture(
        &mut self,
        notification: ZmqNotification,
    ) -> Result<CommittedObservation, CaptureError> {
        self.capture_payload(
            &notification.topic,
            &notification.topic,
            notification.body,
            u64::from(notification.transport_sequence),
            Some(notification.transport_sequence),
            false,
        )
    }

    /// Persists an atomic RPC snapshot as recovery evidence.
    ///
    /// # Errors
    ///
    /// Returns identity, WAL, or committed-read failures.
    pub fn capture_recovered_mempool_snapshot(
        &mut self,
        payload: Vec<u8>,
        mempool_sequence: u64,
    ) -> Result<CommittedObservation, CaptureError> {
        self.capture_payload(
            "rpc",
            "getrawmempool_snapshot",
            payload,
            mempool_sequence,
            None,
            true,
        )
    }

    fn capture_payload(
        &mut self,
        channel: &str,
        source_message_type: &str,
        payload: Vec<u8>,
        source_sequence: u64,
        zmq_transport_sequence: Option<u32>,
        recovered_by_rpc: bool,
    ) -> Result<CommittedObservation, CaptureError> {
        let collector_sequence = self.session.allocate();
        let mut observation = ObservationBuilder::new()
            .source_id(&self.source_id)
            .source_session_id(self.session.id())
            .collector_sequence(collector_sequence)
            .chain("bitcoin")
            .network(self.network.as_str())
            .channel(channel)
            .source_message_type(source_message_type)
            .source_sequence(source_sequence)
            .observed_at_unix_ns(self.clock.wall_time_unix_ns())
            .observed_at_monotonic_ns(self.clock.monotonic_ns())
            .payload(payload)
            .build()?;
        if let Some(sequence) = zmq_transport_sequence {
            observation
                .attributes
                .insert("zmq_transport_sequence".to_owned(), sequence.to_string());
        }
        if recovered_by_rpc {
            observation
                .quality_flags
                .push("recovered_by_rpc".to_owned());
        }
        self.wal.append(UnframedObservation::new(observation))?;
        self.wal.group_commit()?;
        self.wal
            .committed()?
            .find(|record| {
                record
                    .observation
                    .as_ref()
                    .is_some_and(|value| value.collector_sequence == collector_sequence.get())
            })
            .ok_or(CaptureError::MissingCommittedObservation(
                collector_sequence.get(),
            ))
    }

    #[must_use]
    pub const fn session(&self) -> &SourceSession {
        &self.session
    }

    pub fn into_parts(self) -> (SourceSession, W) {
        (self.session, self.wal)
    }
}
