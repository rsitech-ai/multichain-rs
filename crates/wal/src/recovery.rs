use std::fs::File;

use observation_envelope::SourceSessionId;
use platform_proto::observation::{CommittedObservation, Observation, WalCommit};
use prost::Message;

use crate::{
    WalError,
    format::{
        GROUP_COMMIT_FRAME, OBSERVATION_FRAME, RawFrame, SEGMENT_SEAL_FRAME, calculate_commit_hash,
    },
    reader::{hash_prefix, scan},
};

/// Explicit evidence emitted when crash recovery discards an unpublishable
/// tail.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecoveryIncident {
    /// The final frame ended before its declared boundary.
    TruncatedTail {
        /// First invalid byte offset.
        at_offset: u64,
    },
    /// Valid observation frames had no durable commit record.
    UncommittedTail {
        /// First uncommitted frame offset.
        from_offset: u64,
        /// Number of complete observations discarded.
        observations: usize,
    },
}

/// Result of validating and repairing a WAL segment.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RecoveryReport {
    /// Visible recovery incidents.
    pub incidents: Vec<RecoveryIncident>,
    /// Logical append boundary after repair.
    pub logical_end: u64,
}

pub(crate) struct RecoveryState {
    pub(crate) report: RecoveryReport,
    pub(crate) committed: Vec<CommittedObservation>,
    pub(crate) next_sequence: u64,
    pub(crate) sealed: bool,
}

#[derive(Clone)]
struct CandidateObservation {
    observation: Observation,
    offset: u64,
    end_offset: u64,
    frame_bytes: Vec<u8>,
}

#[derive(Default)]
struct RecoveryAccumulator {
    committed: Vec<CommittedObservation>,
    candidates: Vec<CandidateObservation>,
    invalid_observation_offset: Option<u64>,
    last_commit_end: u64,
    next_sequence: u64,
    sealed: bool,
}

pub(crate) fn recover(
    file: &File,
    expected_session: SourceSessionId,
) -> Result<RecoveryState, WalError> {
    let scan = scan(file)?;
    let mut accumulator = RecoveryAccumulator::default();

    for frame in &scan.frames {
        accumulator.process_frame(file, expected_session, frame)?;
    }

    let report = finish_report(
        scan.scanned_end,
        scan.truncated.map(|frame| frame.offset),
        accumulator.last_commit_end,
        &accumulator.candidates,
        accumulator.invalid_observation_offset,
    );

    Ok(RecoveryState {
        report,
        committed: accumulator.committed,
        next_sequence: accumulator.next_sequence,
        sealed: accumulator.sealed,
    })
}

impl RecoveryAccumulator {
    fn process_frame(
        &mut self,
        file: &File,
        expected_session: SourceSessionId,
        frame: &RawFrame,
    ) -> Result<(), WalError> {
        if self.sealed {
            return Err(WalError::CorruptFrame {
                offset: frame.offset,
                reason: "frame found after segment seal".to_owned(),
            });
        }
        match frame.frame_type {
            OBSERVATION_FRAME => self.process_observation(frame),
            GROUP_COMMIT_FRAME => self.process_commit(expected_session, frame),
            SEGMENT_SEAL_FRAME => self.process_seal(file, frame),
            other => Err(WalError::CorruptFrame {
                offset: frame.offset,
                reason: format!("unknown frame type 0x{other:02x}"),
            }),
        }
    }

    fn process_observation(&mut self, frame: &RawFrame) -> Result<(), WalError> {
        if !frame.crc_valid {
            self.invalid_observation_offset.get_or_insert(frame.offset);
            return Ok(());
        }
        let observation =
            Observation::decode(frame.payload.as_slice()).map_err(|error| WalError::Decode {
                offset: frame.offset,
                reason: error.to_string(),
            })?;
        self.candidates.push(CandidateObservation {
            observation,
            offset: frame.offset,
            end_offset: frame.end_offset,
            frame_bytes: frame.bytes.clone(),
        });
        Ok(())
    }

    fn process_commit(
        &mut self,
        expected_session: SourceSessionId,
        frame: &RawFrame,
    ) -> Result<(), WalError> {
        if !frame.crc_valid {
            return Err(WalError::CorruptFrame {
                offset: frame.offset,
                reason: "group-commit CRC mismatch".to_owned(),
            });
        }
        if let Some(offset) = self.invalid_observation_offset {
            return Err(WalError::CommittedCorruption { offset });
        }
        let commit =
            WalCommit::decode(frame.payload.as_slice()).map_err(|error| WalError::Decode {
                offset: frame.offset,
                reason: error.to_string(),
            })?;
        validate_commit(expected_session, &commit, &self.candidates, frame.offset)?;
        let commit_hash: [u8; 32] =
            commit
                .commit_hash
                .as_slice()
                .try_into()
                .map_err(|_| WalError::CorruptFrame {
                    offset: frame.offset,
                    reason: "commit hash is not 32 bytes".to_owned(),
                })?;
        for candidate in self.candidates.drain(..) {
            self.committed.push(CommittedObservation {
                observation: Some(candidate.observation),
                durable_at_unix_ns: commit.durable_at_unix_ns,
                wal_commit_hash: commit_hash.to_vec(),
            });
        }
        self.next_sequence = commit
            .last_collector_sequence
            .checked_add(1)
            .ok_or_else(|| WalError::CorruptFrame {
                offset: frame.offset,
                reason: "collector sequence overflow".to_owned(),
            })?;
        self.last_commit_end = frame.end_offset;
        Ok(())
    }

    fn process_seal(&mut self, file: &File, frame: &RawFrame) -> Result<(), WalError> {
        if !frame.crc_valid {
            return Err(WalError::CorruptFrame {
                offset: frame.offset,
                reason: "segment-seal CRC mismatch".to_owned(),
            });
        }
        if !self.candidates.is_empty() || self.invalid_observation_offset.is_some() {
            return Err(WalError::CorruptFrame {
                offset: frame.offset,
                reason: "segment sealed with uncommitted observations".to_owned(),
            });
        }
        let expected_hash = hash_prefix(file, frame.offset)?;
        if frame.payload.as_slice() != expected_hash {
            return Err(WalError::CorruptFrame {
                offset: frame.offset,
                reason: "segment-seal hash mismatch".to_owned(),
            });
        }
        self.sealed = true;
        Ok(())
    }
}

fn finish_report(
    scanned_end: u64,
    truncated_offset: Option<u64>,
    last_commit_end: u64,
    candidates: &[CandidateObservation],
    invalid_observation_offset: Option<u64>,
) -> RecoveryReport {
    let mut report = RecoveryReport {
        incidents: Vec::new(),
        logical_end: scanned_end,
    };

    if let Some(at_offset) = truncated_offset {
        report
            .incidents
            .push(RecoveryIncident::TruncatedTail { at_offset });
        report.logical_end = last_commit_end;
    } else if !candidates.is_empty() || invalid_observation_offset.is_some() {
        let from_offset = candidates.first().map_or_else(
            || invalid_observation_offset.unwrap_or(last_commit_end),
            |item| item.offset,
        );
        report.incidents.push(RecoveryIncident::UncommittedTail {
            from_offset,
            observations: candidates.len(),
        });
        report.logical_end = last_commit_end;
    }
    report
}

fn validate_commit(
    expected_session: SourceSessionId,
    commit: &WalCommit,
    candidates: &[CandidateObservation],
    commit_offset: u64,
) -> Result<(), WalError> {
    let Some(first) = candidates.first() else {
        return Err(WalError::CorruptFrame {
            offset: commit_offset,
            reason: "commit covers no observation frames".to_owned(),
        });
    };
    let last = candidates.last().expect("first candidate exists");

    if commit.source_session_id.as_slice() != expected_session.as_bytes() {
        return Err(WalError::CorruptFrame {
            offset: commit_offset,
            reason: "commit source session mismatch".to_owned(),
        });
    }
    if commit.first_collector_sequence != first.observation.collector_sequence
        || commit.last_collector_sequence != last.observation.collector_sequence
        || commit.first_wal_offset != first.offset
        || commit.last_wal_offset != last.end_offset - 1
    {
        return Err(WalError::CorruptFrame {
            offset: commit_offset,
            reason: "commit range does not match preceding observations".to_owned(),
        });
    }
    for pair in candidates.windows(2) {
        if pair[1].observation.collector_sequence != pair[0].observation.collector_sequence + 1 {
            return Err(WalError::CorruptFrame {
                offset: pair[1].offset,
                reason: "collector sequence is not contiguous".to_owned(),
            });
        }
    }

    let frame_bytes = candidates
        .iter()
        .map(|candidate| candidate.frame_bytes.clone())
        .collect::<Vec<_>>();
    let expected_hash = calculate_commit_hash(
        expected_session,
        commit.first_collector_sequence,
        commit.last_collector_sequence,
        commit.first_wal_offset,
        commit.last_wal_offset,
        commit.durable_at_unix_ns,
        &frame_bytes,
    );
    if commit.commit_hash.as_slice() != expected_hash {
        return Err(WalError::CorruptFrame {
            offset: commit_offset,
            reason: "commit hash mismatch".to_owned(),
        });
    }
    Ok(())
}
