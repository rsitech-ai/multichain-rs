use observation_envelope::SourceSessionId;

use crate::WalError;

pub(crate) const OBSERVATION_FRAME: u8 = 0x01;
pub(crate) const GROUP_COMMIT_FRAME: u8 = 0x02;
pub(crate) const SEGMENT_SEAL_FRAME: u8 = 0x03;
pub(crate) const FRAME_HEADER_BYTES: usize = 5;
pub(crate) const FRAME_CRC_BYTES: usize = 4;
pub(crate) const COMMIT_RESERVE_BYTES: u64 = 512;
pub(crate) const MAX_FRAME_PAYLOAD_BYTES: u64 = 64 * 1024 * 1024;
const COMMIT_DOMAIN: &[u8] = b"wal-commit/v1";

#[derive(Clone, Debug)]
pub(crate) struct RawFrame {
    pub(crate) frame_type: u8,
    pub(crate) offset: u64,
    pub(crate) end_offset: u64,
    pub(crate) payload: Vec<u8>,
    pub(crate) bytes: Vec<u8>,
    pub(crate) crc_valid: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TruncatedFrame {
    pub(crate) offset: u64,
}

pub(crate) fn encode_frame(frame_type: u8, payload: &[u8]) -> Result<Vec<u8>, WalError> {
    let payload_bytes = u64::try_from(payload.len()).unwrap_or(u64::MAX);
    if payload_bytes > MAX_FRAME_PAYLOAD_BYTES {
        return Err(WalError::CapacityExhausted {
            required: payload_bytes,
            available: MAX_FRAME_PAYLOAD_BYTES,
        });
    }
    let payload_len = u32::try_from(payload.len()).map_err(|_| WalError::CapacityExhausted {
        required: u64::MAX,
        available: 0,
    })?;
    let mut frame = Vec::with_capacity(FRAME_HEADER_BYTES + payload.len() + FRAME_CRC_BYTES);
    frame.push(frame_type);
    frame.extend_from_slice(&payload_len.to_be_bytes());
    frame.extend_from_slice(payload);
    let crc = crc32c::crc32c(&frame);
    frame.extend_from_slice(&crc.to_be_bytes());
    Ok(frame)
}

pub(crate) fn calculate_commit_hash(
    source_session_id: SourceSessionId,
    first_sequence: u64,
    last_sequence: u64,
    first_wal_offset: u64,
    last_wal_offset: u64,
    durable_at_unix_ns: i64,
    observation_frames: &[Vec<u8>],
) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(COMMIT_DOMAIN);
    hasher.update(source_session_id.as_bytes());
    hasher.update(&first_sequence.to_be_bytes());
    hasher.update(&last_sequence.to_be_bytes());
    hasher.update(&first_wal_offset.to_be_bytes());
    hasher.update(&last_wal_offset.to_be_bytes());
    hasher.update(&durable_at_unix_ns.to_be_bytes());
    for frame in observation_frames {
        hasher.update(frame);
    }
    *hasher.finalize().as_bytes()
}
