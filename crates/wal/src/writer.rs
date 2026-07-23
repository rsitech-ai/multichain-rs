use std::{
    ffi::OsString,
    fs::{File, OpenOptions},
    io::{self, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use observation_envelope::{Clock, CollectorSequence, SourceSessionId};
use platform_proto::observation::{CommittedObservation, WalCommit};
use prost::Message;

use crate::{
    CommittedRange, ObservationWal, PendingFrame, PendingObservation, UnframedObservation,
    WalError, WalOffset,
    format::{
        COMMIT_RESERVE_BYTES, GROUP_COMMIT_FRAME, OBSERVATION_FRAME, SEGMENT_SEAL_FRAME,
        calculate_commit_hash, encode_frame,
    },
    reader::hash_prefix,
    recovery::{RecoveryReport, recover},
    validate_session,
};

/// Capacity and session policy for one preallocated WAL segment.
#[derive(Clone, Debug)]
pub struct WalConfig {
    source_session_id: SourceSessionId,
    max_bytes: u64,
    group_commit_interval: Duration,
}

impl WalConfig {
    /// Creates a bounded single-segment WAL configuration.
    #[must_use]
    pub const fn new(
        source_session_id: SourceSessionId,
        max_bytes: u64,
        group_commit_interval: Duration,
    ) -> Self {
        Self {
            source_session_id,
            max_bytes,
            group_commit_interval,
        }
    }

    /// Returns the hard segment capacity.
    #[must_use]
    pub const fn max_bytes(&self) -> u64 {
        self.max_bytes
    }

    /// Returns the configured maximum group-commit interval.
    #[must_use]
    pub const fn group_commit_interval(&self) -> Duration {
        self.group_commit_interval
    }
}

/// File-backed, preallocated observation WAL.
pub struct FileWal {
    file: File,
    config: WalConfig,
    clock: Arc<dyn Clock>,
    logical_end: u64,
    next_sequence: u64,
    pending: Vec<PendingFrame>,
    sealed: bool,
}

impl FileWal {
    /// Opens or creates a segment, validates it, and removes any uncommitted or
    /// truncated tail before returning.
    ///
    /// # Errors
    ///
    /// Returns [`WalError`] when the configuration is invalid, an I/O
    /// operation fails, or recovery detects committed corruption.
    pub fn open(
        path: impl AsRef<Path>,
        config: WalConfig,
        clock: Arc<dyn Clock>,
    ) -> Result<(Self, RecoveryReport), WalError> {
        if config.max_bytes <= COMMIT_RESERVE_BYTES {
            return Err(WalError::InvalidCapacity {
                max_bytes: config.max_bytes,
            });
        }

        let path = path.as_ref();
        let existed = path.exists();
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(path)?;
        if !existed {
            file.set_len(config.max_bytes)?;
        } else if file.metadata()?.len() > config.max_bytes {
            return Err(WalError::CapacityExhausted {
                required: file.metadata()?.len(),
                available: config.max_bytes,
            });
        }

        let state = match recover(&file, config.source_session_id) {
            Ok(state) => state,
            Err(error) if error.requires_quarantine() => {
                drop(file);
                quarantine(path)?;
                return Err(error);
            }
            Err(error) => return Err(error),
        };
        if state.report.logical_end < file.metadata()?.len() {
            file.set_len(state.report.logical_end)?;
        }
        file.set_len(config.max_bytes)?;
        file.sync_data()?;

        let report = state.report.clone();
        Ok((
            Self {
                file,
                config,
                clock,
                logical_end: report.logical_end,
                next_sequence: state.next_sequence,
                pending: Vec::new(),
                sealed: state.sealed,
            },
            report,
        ))
    }

    /// Returns the next unwritten byte offset.
    #[must_use]
    pub const fn logical_end(&self) -> u64 {
        self.logical_end
    }

    /// Writes and durably flushes a terminal digest over every preceding frame.
    ///
    /// # Errors
    ///
    /// Returns [`WalError`] when observations are pending, the segment is
    /// already sealed, capacity is exhausted, or an I/O operation fails.
    pub fn seal(&mut self) -> Result<[u8; 32], WalError> {
        if self.sealed {
            return Err(WalError::Sealed);
        }
        if !self.pending.is_empty() {
            return Err(WalError::PendingObservations);
        }

        let seal_hash = hash_prefix(&self.file, self.logical_end)?;
        let frame = encode_frame(SEGMENT_SEAL_FRAME, &seal_hash)?;
        self.write_frame(&frame)?;
        self.file.sync_data()?;
        self.sealed = true;
        Ok(seal_hash)
    }

    fn remaining_capacity(&self) -> u64 {
        self.config.max_bytes.saturating_sub(self.logical_end)
    }

    fn write_frame(&mut self, frame: &[u8]) -> Result<u64, WalError> {
        let required = u64::try_from(frame.len()).map_err(|_| WalError::CapacityExhausted {
            required: u64::MAX,
            available: self.remaining_capacity(),
        })?;
        if required > self.remaining_capacity() {
            return Err(WalError::CapacityExhausted {
                required,
                available: self.remaining_capacity(),
            });
        }

        let offset = self.logical_end;
        self.file.seek(SeekFrom::Start(offset))?;
        self.file.write_all(frame)?;
        self.logical_end += required;
        Ok(offset)
    }
}

fn quarantine(path: &Path) -> Result<PathBuf, WalError> {
    let base = appended_path(path, ".quarantine");
    let destination = if base.exists() {
        (1_u32..=10_000)
            .map(|index| appended_path(path, &format!(".quarantine.{index}")))
            .find(|candidate| !candidate.exists())
            .ok_or_else(|| WalError::QuarantineFailed {
                path: base.clone(),
                source: io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "all quarantine suffixes from 1 through 10000 are occupied",
                ),
            })?
    } else {
        base
    };
    std::fs::rename(path, &destination).map_err(|source| WalError::QuarantineFailed {
        path: destination.clone(),
        source,
    })?;
    Ok(destination)
}

fn appended_path(path: &Path, suffix: &str) -> PathBuf {
    let mut value = OsString::from(path.as_os_str());
    value.push(suffix);
    PathBuf::from(value)
}

impl ObservationWal for FileWal {
    fn append(&mut self, input: UnframedObservation) -> Result<PendingObservation, WalError> {
        if self.sealed {
            return Err(WalError::Sealed);
        }
        validate_session(self.config.source_session_id, &input.observation)?;
        if input.observation.collector_sequence != self.next_sequence {
            return Err(WalError::SequenceMismatch {
                expected: self.next_sequence,
                actual: input.observation.collector_sequence,
            });
        }

        let payload = input.observation.encode_to_vec();
        let frame = encode_frame(OBSERVATION_FRAME, &payload)?;
        let frame_len = u64::try_from(frame.len()).map_err(|_| WalError::CapacityExhausted {
            required: u64::MAX,
            available: self.remaining_capacity(),
        })?;
        let required = frame_len.saturating_add(COMMIT_RESERVE_BYTES);
        if required > self.remaining_capacity() {
            return Err(WalError::CapacityExhausted {
                required,
                available: self.remaining_capacity(),
            });
        }

        let offset = self.write_frame(&frame)?;
        let end_offset = self.logical_end;
        self.pending.push(PendingFrame {
            observation: input.observation.clone(),
            offset: WalOffset::new(offset),
            end_offset,
            frame_bytes: frame,
        });
        self.next_sequence =
            self.next_sequence
                .checked_add(1)
                .ok_or(WalError::SequenceMismatch {
                    expected: u64::MAX,
                    actual: u64::MAX,
                })?;

        Ok(PendingObservation {
            observation: input.observation,
            wal_offset: WalOffset::new(offset),
        })
    }

    fn group_commit(&mut self) -> Result<CommittedRange, WalError> {
        if self.sealed {
            return Err(WalError::Sealed);
        }
        let first = self
            .pending
            .first()
            .ok_or(WalError::NoPendingObservations)?;
        let last = self
            .pending
            .last()
            .expect("first pending observation exists");
        self.file.sync_data()?;
        let durable_at_unix_ns = self.clock.wall_time_unix_ns();
        let frame_bytes = self
            .pending
            .iter()
            .map(|pending| pending.frame_bytes.clone())
            .collect::<Vec<_>>();
        let commit_hash = calculate_commit_hash(
            self.config.source_session_id,
            first.observation.collector_sequence,
            last.observation.collector_sequence,
            first.offset.get(),
            last.end_offset - 1,
            durable_at_unix_ns,
            &frame_bytes,
        );
        let commit = WalCommit {
            source_session_id: self.config.source_session_id.as_bytes().to_vec(),
            first_collector_sequence: first.observation.collector_sequence,
            last_collector_sequence: last.observation.collector_sequence,
            first_wal_offset: first.offset.get(),
            last_wal_offset: last.end_offset - 1,
            durable_at_unix_ns,
            commit_hash: commit_hash.to_vec(),
        };
        let commit_frame = encode_frame(GROUP_COMMIT_FRAME, &commit.encode_to_vec())?;

        self.write_frame(&commit_frame)?;
        self.file.sync_data()?;

        let range = CommittedRange {
            first_sequence: CollectorSequence::new(commit.first_collector_sequence),
            last_sequence: CollectorSequence::new(commit.last_collector_sequence),
            durable_at_unix_ns,
            commit_hash,
        };
        self.pending.clear();
        Ok(range)
    }

    fn committed(&self) -> Result<Box<dyn Iterator<Item = CommittedObservation>>, WalError> {
        let state = recover(&self.file, self.config.source_session_id)?;
        Ok(Box::new(state.committed.into_iter()))
    }
}
