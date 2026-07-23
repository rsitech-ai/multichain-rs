use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
};

use crate::{
    WalError,
    format::{
        FRAME_CRC_BYTES, FRAME_HEADER_BYTES, MAX_FRAME_PAYLOAD_BYTES, RawFrame, TruncatedFrame,
    },
};

pub(crate) struct ScanResult {
    pub(crate) frames: Vec<RawFrame>,
    pub(crate) truncated: Option<TruncatedFrame>,
    pub(crate) scanned_end: u64,
}

pub(crate) fn scan(file: &File) -> Result<ScanResult, WalError> {
    let physical_len = file.metadata()?.len();
    let mut reader = file.try_clone()?;
    reader.seek(SeekFrom::Start(0))?;
    let mut frames = Vec::new();
    let mut offset = 0_u64;
    let mut truncated = None;

    while offset < physical_len {
        let mut frame_type = [0_u8; 1];
        reader.read_exact(&mut frame_type)?;
        if frame_type[0] == 0 {
            break;
        }

        if physical_len - offset < FRAME_HEADER_BYTES as u64 {
            truncated = Some(TruncatedFrame { offset });
            break;
        }

        let mut length_bytes = [0_u8; 4];
        reader.read_exact(&mut length_bytes)?;
        let payload_len = u64::from(u32::from_be_bytes(length_bytes));
        if payload_len > MAX_FRAME_PAYLOAD_BYTES {
            return Err(WalError::CorruptFrame {
                offset,
                reason: "frame payload exceeds the 64 MiB safety bound".to_owned(),
            });
        }
        let frame_len = FRAME_HEADER_BYTES as u64 + payload_len + FRAME_CRC_BYTES as u64;
        if frame_len > physical_len - offset {
            truncated = Some(TruncatedFrame { offset });
            break;
        }

        let frame_len_usize = usize::try_from(frame_len).map_err(|_| WalError::CorruptFrame {
            offset,
            reason: "frame length exceeds addressable memory".to_owned(),
        })?;
        let payload_len_usize =
            usize::try_from(payload_len).map_err(|_| WalError::CorruptFrame {
                offset,
                reason: "payload length exceeds addressable memory".to_owned(),
            })?;

        let mut bytes = Vec::with_capacity(frame_len_usize);
        bytes.push(frame_type[0]);
        bytes.extend_from_slice(&length_bytes);

        let mut payload = vec![0_u8; payload_len_usize];
        reader.read_exact(&mut payload)?;
        bytes.extend_from_slice(&payload);

        let mut crc_bytes = [0_u8; 4];
        reader.read_exact(&mut crc_bytes)?;
        let stored_crc = u32::from_be_bytes(crc_bytes);
        let crc_valid = crc32c::crc32c(&bytes) == stored_crc;
        bytes.extend_from_slice(&crc_bytes);

        let end_offset = offset + frame_len;
        frames.push(RawFrame {
            frame_type: frame_type[0],
            offset,
            end_offset,
            payload,
            bytes,
            crc_valid,
        });
        offset = end_offset;
    }

    Ok(ScanResult {
        frames,
        truncated,
        scanned_end: offset,
    })
}

pub(crate) fn hash_prefix(file: &File, end_offset: u64) -> Result<[u8; 32], WalError> {
    let mut reader = file.try_clone()?;
    reader.seek(SeekFrom::Start(0))?;
    let mut hasher = blake3::Hasher::new();
    let mut remaining = end_offset;
    let mut buffer = [0_u8; 8 * 1024];
    let buffer_len = u64::try_from(buffer.len()).expect("fixed buffer length fits in u64");
    while remaining > 0 {
        let chunk_len = usize::try_from(remaining.min(buffer_len))
            .expect("chunk length is bounded by the fixed buffer");
        reader.read_exact(&mut buffer[..chunk_len])?;
        hasher.update(&buffer[..chunk_len]);
        remaining -= u64::try_from(chunk_len).expect("chunk length fits in u64");
    }
    Ok(*hasher.finalize().as_bytes())
}
