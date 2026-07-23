use std::io::{Cursor, Read};

use platform_proto::observation::CommittedObservation;
use prost::Message;
use sha2::{Digest, Sha256};
use thiserror::Error;
use time::OffsetDateTime;

const FRAME_LENGTH_BYTES: usize = size_of::<u32>();

/// Failures while validating, encoding, or decoding a raw archive batch.
#[derive(Debug, Error)]
pub enum ArchiveError {
    /// A batch must contain at least one durable observation.
    #[error("archive batch is empty")]
    EmptyBatch,
    /// A committed record did not contain its observation.
    #[error("committed record at index {index} has no observation")]
    MissingObservation {
        /// Zero-based record index.
        index: usize,
    },
    /// A fixed-width observation field was malformed.
    #[error("field `{field}` must be {expected} bytes, got {actual}")]
    InvalidLength {
        /// Field name.
        field: &'static str,
        /// Expected byte length.
        expected: usize,
        /// Supplied byte length.
        actual: usize,
    },
    /// Records in a single archive object disagreed on partition identity.
    #[error("archive records have mixed `{field}` values")]
    MixedBatchField {
        /// Field that differed.
        field: &'static str,
    },
    /// Collector sequences within one batch were not contiguous.
    #[error("collector sequence is not contiguous: expected {expected}, got {actual}")]
    NonContiguousRange {
        /// Required next sequence.
        expected: u64,
        /// Observed sequence.
        actual: u64,
    },
    /// An archive path component was unsafe or empty.
    #[error("field `{field}` is not a safe archive path component")]
    InvalidPathField {
        /// Field name.
        field: &'static str,
    },
    /// A timestamp could not be represented.
    #[error("observation timestamp is outside the supported UTC range")]
    InvalidTimestamp,
    /// A serialized record exceeded the frame limit.
    #[error("serialized archive record is larger than 4 GiB")]
    RecordTooLarge,
    /// A frame ended before its declared length.
    #[error("archive frame is truncated")]
    TruncatedFrame,
    /// A protobuf record was invalid.
    #[error("archive record decode failed: {0}")]
    Decode(#[from] prost::DecodeError),
    /// Compression or decompression failed.
    #[error("archive compression failed: {0}")]
    Io(#[from] std::io::Error),
}

/// A validated, homogeneous range of durable observations.
#[derive(Clone, Debug)]
pub struct ArchiveBatch {
    records: Vec<CommittedObservation>,
    source_id: String,
    source_session_id: [u8; 16],
    first_collector_sequence: u64,
    last_collector_sequence: u64,
    object_key: String,
}

impl ArchiveBatch {
    /// Validates an archive range and derives its deterministic object key.
    ///
    /// # Errors
    ///
    /// Returns [`ArchiveError`] for empty, malformed, non-contiguous, or mixed
    /// batches.
    pub fn try_new(records: Vec<CommittedObservation>) -> Result<Self, ArchiveError> {
        let first = observation_at(&records, 0)?;
        validate_path_component(&first.source_id, "source_id")?;
        validate_path_component(&first.chain, "chain")?;
        validate_path_component(&first.network, "network")?;
        validate_path_component(&first.channel, "channel")?;
        let source_session_id = fixed_bytes::<16>(&first.source_session_id, "source_session_id")?;
        let first_collector_sequence = first.collector_sequence;
        let archive_partition = partition(first.observed_at_unix_ns)?;

        let mut expected_sequence = first_collector_sequence;
        for (index, record) in records.iter().enumerate() {
            let observation = record
                .observation
                .as_ref()
                .ok_or(ArchiveError::MissingObservation { index })?;
            if observation.source_id != first.source_id {
                return Err(ArchiveError::MixedBatchField { field: "source_id" });
            }
            if observation.source_session_id != first.source_session_id {
                return Err(ArchiveError::MixedBatchField {
                    field: "source_session_id",
                });
            }
            if observation.chain != first.chain {
                return Err(ArchiveError::MixedBatchField { field: "chain" });
            }
            if observation.network != first.network {
                return Err(ArchiveError::MixedBatchField { field: "network" });
            }
            if observation.channel != first.channel {
                return Err(ArchiveError::MixedBatchField { field: "channel" });
            }
            if partition(observation.observed_at_unix_ns)? != archive_partition {
                return Err(ArchiveError::MixedBatchField {
                    field: "partition_hour",
                });
            }
            if observation.collector_sequence != expected_sequence {
                return Err(ArchiveError::NonContiguousRange {
                    expected: expected_sequence,
                    actual: observation.collector_sequence,
                });
            }
            expected_sequence =
                expected_sequence
                    .checked_add(1)
                    .ok_or(ArchiveError::NonContiguousRange {
                        expected: u64::MAX,
                        actual: u64::MAX,
                    })?;
        }

        let last_collector_sequence = expected_sequence - 1;
        let session_hex = hex(&source_session_id);
        let source_id = first.source_id.clone();
        let object_key = format!(
            "raw/chain={}/network={}/source={}/channel={}/date={}/hour={}/\
             part-{session_hex}-{first_collector_sequence}-{last_collector_sequence}.bin.zst",
            first.chain,
            first.network,
            source_id,
            first.channel,
            archive_partition.date,
            archive_partition.hour
        );

        Ok(Self {
            records,
            source_id,
            source_session_id,
            first_collector_sequence,
            last_collector_sequence,
            object_key,
        })
    }

    /// Serializes length-delimited protobuf records and compresses the complete
    /// byte stream with Zstandard.
    ///
    /// # Errors
    ///
    /// Returns [`ArchiveError`] when a frame is too large or compression fails.
    pub fn encode(self) -> Result<EncodedArchive, ArchiveError> {
        let mut framed = Vec::new();
        for record in &self.records {
            let bytes = record.encode_to_vec();
            let length = u32::try_from(bytes.len()).map_err(|_| ArchiveError::RecordTooLarge)?;
            framed.extend_from_slice(&length.to_be_bytes());
            framed.extend_from_slice(&bytes);
        }
        let compressed_bytes = zstd::stream::encode_all(Cursor::new(framed), 3)?;
        let object_sha256 = Sha256::digest(&compressed_bytes).into();
        let record_count =
            u64::try_from(self.records.len()).map_err(|_| ArchiveError::RecordTooLarge)?;
        Ok(EncodedArchive {
            compressed_bytes,
            object_sha256,
            object_key: self.object_key,
            source_id: self.source_id,
            source_session_id: self.source_session_id,
            first_collector_sequence: self.first_collector_sequence,
            last_collector_sequence: self.last_collector_sequence,
            record_count,
        })
    }
}

/// Complete immutable bytes and range metadata for one archive object.
#[derive(Clone, Debug)]
pub struct EncodedArchive {
    compressed_bytes: Vec<u8>,
    object_sha256: [u8; 32],
    object_key: String,
    source_id: String,
    source_session_id: [u8; 16],
    first_collector_sequence: u64,
    last_collector_sequence: u64,
    record_count: u64,
}

impl EncodedArchive {
    /// Returns the deterministic object key.
    #[must_use]
    pub fn object_key(&self) -> &str {
        &self.object_key
    }

    /// Returns the stable source identifier.
    #[must_use]
    pub fn source_id(&self) -> &str {
        &self.source_id
    }

    /// Returns the source session.
    #[must_use]
    pub const fn source_session_id(&self) -> [u8; 16] {
        self.source_session_id
    }

    /// Returns the inclusive first collector sequence.
    #[must_use]
    pub const fn first_collector_sequence(&self) -> u64 {
        self.first_collector_sequence
    }

    /// Returns the inclusive last collector sequence.
    #[must_use]
    pub const fn last_collector_sequence(&self) -> u64 {
        self.last_collector_sequence
    }

    /// Returns the SHA-256 digest over the exact compressed bytes.
    #[must_use]
    pub const fn object_sha256(&self) -> [u8; 32] {
        self.object_sha256
    }

    /// Returns the number of framed observations.
    #[must_use]
    pub const fn record_count(&self) -> u64 {
        self.record_count
    }

    /// Borrows the exact compressed bytes.
    #[must_use]
    pub fn compressed_bytes(&self) -> &[u8] {
        &self.compressed_bytes
    }

    /// Consumes the object and returns its exact compressed bytes.
    #[must_use]
    pub fn into_compressed_bytes(self) -> Vec<u8> {
        self.compressed_bytes
    }
}

/// Decompresses and validates every framed committed observation.
///
/// # Errors
///
/// Returns [`ArchiveError`] for invalid compression, framing, or protobuf
/// payloads.
pub fn decode_archive(bytes: &[u8]) -> Result<Vec<CommittedObservation>, ArchiveError> {
    let decoded = zstd::stream::decode_all(Cursor::new(bytes))?;
    let mut cursor = Cursor::new(decoded);
    let mut records = Vec::new();

    while usize::try_from(cursor.position()).unwrap_or(usize::MAX) < cursor.get_ref().len() {
        let mut length_bytes = [0_u8; FRAME_LENGTH_BYTES];
        cursor
            .read_exact(&mut length_bytes)
            .map_err(|_| ArchiveError::TruncatedFrame)?;
        let length = usize::try_from(u32::from_be_bytes(length_bytes))
            .map_err(|_| ArchiveError::RecordTooLarge)?;
        let mut record = vec![0_u8; length];
        cursor
            .read_exact(&mut record)
            .map_err(|_| ArchiveError::TruncatedFrame)?;
        records.push(CommittedObservation::decode(record.as_slice())?);
    }

    Ok(records)
}

fn observation_at(
    records: &[CommittedObservation],
    index: usize,
) -> Result<&platform_proto::observation::Observation, ArchiveError> {
    records
        .get(index)
        .ok_or(ArchiveError::EmptyBatch)?
        .observation
        .as_ref()
        .ok_or(ArchiveError::MissingObservation { index })
}

fn fixed_bytes<const N: usize>(value: &[u8], field: &'static str) -> Result<[u8; N], ArchiveError> {
    value.try_into().map_err(|_| ArchiveError::InvalidLength {
        field,
        expected: N,
        actual: value.len(),
    })
}

fn validate_path_component(value: &str, field: &'static str) -> Result<(), ArchiveError> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\\')
        || value.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err(ArchiveError::InvalidPathField { field });
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Partition {
    date: String,
    hour: u8,
}

fn partition(unix_ns: i64) -> Result<Partition, ArchiveError> {
    let timestamp = OffsetDateTime::from_unix_timestamp_nanos(i128::from(unix_ns))
        .map_err(|_| ArchiveError::InvalidTimestamp)?;
    Ok(Partition {
        date: format!(
            "{:04}-{:02}-{:02}",
            timestamp.year(),
            u8::from(timestamp.month()),
            timestamp.day()
        ),
        hour: timestamp.hour(),
    })
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    output
}
