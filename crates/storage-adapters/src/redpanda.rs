use std::{collections::HashSet, future::Future, sync::Arc, time::Duration};

use platform_proto::observation::CommittedObservation;
use prost::Message;
use rdkafka::{
    ClientConfig,
    message::{Header, OwnedHeaders},
    producer::{FutureProducer, FutureRecord},
};
use storage_ports::{BrokerAck, BrokerError, BrokerPublisher};
use tokio::sync::Mutex;

/// Captured logical Kafka record used by deterministic tests.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublishedRecord {
    /// Destination topic.
    pub topic: String,
    /// Stable source ID used as the partitioning key.
    pub key: String,
    /// Replay-stable observation ID used for application deduplication.
    pub event_id: [u8; 32],
    /// Exact serialized committed observation.
    pub value: Vec<u8>,
}

/// In-memory broker with application-event-ID idempotence.
#[derive(Clone, Debug, Default)]
pub struct MemoryBroker {
    state: Arc<Mutex<MemoryBrokerState>>,
}

#[derive(Debug, Default)]
struct MemoryBrokerState {
    event_ids: HashSet<[u8; 32]>,
    records: Vec<PublishedRecord>,
}

/// Kafka-protocol producer configured for durable idempotent publication.
#[derive(Clone)]
pub struct RedpandaBroker {
    producer: FutureProducer,
    queue_timeout: Duration,
}

impl std::fmt::Debug for RedpandaBroker {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RedpandaBroker")
            .field("queue_timeout", &self.queue_timeout)
            .finish_non_exhaustive()
    }
}

impl RedpandaBroker {
    /// Creates a producer with `enable.idempotence=true`, `acks=all`, and a
    /// bounded delivery timeout.
    ///
    /// # Errors
    ///
    /// Returns [`BrokerError`] when librdkafka rejects the configuration.
    pub fn new(brokers: &str, delivery_timeout: Duration) -> Result<Self, BrokerError> {
        if brokers.trim().is_empty() {
            return Err(BrokerError::Delivery(
                "bootstrap broker list is empty".to_owned(),
            ));
        }
        let delivery_timeout_ms = delivery_timeout.as_millis().clamp(1, u128::from(u32::MAX));
        let producer = ClientConfig::new()
            .set("bootstrap.servers", brokers)
            .set("enable.idempotence", "true")
            .set("acks", "all")
            .set("delivery.timeout.ms", delivery_timeout_ms.to_string())
            .set("queue.buffering.max.messages", "100000")
            .create::<FutureProducer>()
            .map_err(|error| BrokerError::Delivery(error.to_string()))?;
        Ok(Self {
            producer,
            queue_timeout: delivery_timeout,
        })
    }
}

impl MemoryBroker {
    /// Returns the unique records durably acknowledged by this adapter.
    pub async fn records(&self) -> Vec<PublishedRecord> {
        self.state.lock().await.records.clone()
    }
}

impl BrokerPublisher for MemoryBroker {
    fn publish(
        &self,
        topic: &str,
        records: &[CommittedObservation],
    ) -> impl Future<Output = Result<BrokerAck, BrokerError>> + Send {
        let topic = topic.to_owned();
        let records = records.to_vec();
        let state = Arc::clone(&self.state);
        async move {
            let prepared = prepare_records(&topic, &records)?;
            let session = prepared.source_session_id;
            let last_collector_sequence = prepared.last_collector_sequence;
            let record_count = prepared.record_count;

            let mut guard = state.lock().await;
            for record in prepared.records {
                if guard.event_ids.insert(record.event_id) {
                    guard.records.push(record);
                }
            }
            Ok(BrokerAck::new(
                session,
                last_collector_sequence,
                record_count,
            ))
        }
    }
}

impl BrokerPublisher for RedpandaBroker {
    fn publish(
        &self,
        topic: &str,
        records: &[CommittedObservation],
    ) -> impl Future<Output = Result<BrokerAck, BrokerError>> + Send {
        let producer = self.producer.clone();
        let queue_timeout = self.queue_timeout;
        let topic = topic.to_owned();
        let records = records.to_vec();
        async move {
            let prepared = prepare_records(&topic, &records)?;
            for record in &prepared.records {
                let headers = OwnedHeaders::new().insert(Header {
                    key: "event_id",
                    value: Some(record.event_id.as_slice()),
                });
                producer
                    .send(
                        FutureRecord::to(&record.topic)
                            .key(&record.key)
                            .payload(&record.value)
                            .headers(headers),
                        queue_timeout,
                    )
                    .await
                    .map_err(|(error, _)| BrokerError::Delivery(error.to_string()))?;
            }
            Ok(BrokerAck::new(
                prepared.source_session_id,
                prepared.last_collector_sequence,
                prepared.record_count,
            ))
        }
    }
}

struct PreparedBatch {
    source_session_id: [u8; 16],
    last_collector_sequence: u64,
    record_count: u64,
    records: Vec<PublishedRecord>,
}

fn prepare_records(
    topic: &str,
    records: &[CommittedObservation],
) -> Result<PreparedBatch, BrokerError> {
    let first = records.first().ok_or(BrokerError::EmptyBatch)?;
    let first_observation = first
        .observation
        .as_ref()
        .ok_or(BrokerError::MissingObservation { index: 0 })?;
    let session = fixed_bytes::<16>(&first_observation.source_session_id, "source_session_id")?;
    let source_id = &first_observation.source_id;
    let mut prepared = Vec::with_capacity(records.len());

    for (index, record) in records.iter().enumerate() {
        let observation = record
            .observation
            .as_ref()
            .ok_or(BrokerError::MissingObservation { index })?;
        if observation.source_session_id.as_slice() != session
            || observation.source_id != *source_id
        {
            return Err(BrokerError::MixedSourceSession);
        }
        prepared.push(PublishedRecord {
            topic: topic.to_owned(),
            key: source_id.clone(),
            event_id: fixed_bytes::<32>(&observation.observation_id, "observation_id")?,
            value: record.encode_to_vec(),
        });
    }

    let last_collector_sequence = records
        .last()
        .and_then(|record| record.observation.as_ref())
        .ok_or(BrokerError::MissingObservation {
            index: records.len().saturating_sub(1),
        })?
        .collector_sequence;
    let record_count = u64::try_from(records.len())
        .map_err(|_| BrokerError::Delivery("record count exceeds u64".to_owned()))?;
    Ok(PreparedBatch {
        source_session_id: session,
        last_collector_sequence,
        record_count,
        records: prepared,
    })
}

fn fixed_bytes<const N: usize>(value: &[u8], field: &'static str) -> Result<[u8; N], BrokerError> {
    value.try_into().map_err(|_| BrokerError::InvalidLength {
        field,
        expected: N,
        actual: value.len(),
    })
}
