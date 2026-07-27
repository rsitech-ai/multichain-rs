use std::fmt::Write as _;

use api_contract::Completeness;
use serde::Serialize;
use thiserror::Error;

const MAX_SOURCE_COUNT: usize = 32;

/// How known-incomplete source coverage affects alert evaluation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DegradedPolicy {
    /// Never trigger while any configured source is unavailable or incomplete.
    Suppress,
    /// Evaluate when the healthy source count still satisfies the quorum.
    EvaluateHealthyQuorum,
}

/// Provenance for one aggregate fee-band revision.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotCause {
    /// Derived from the normal ordered observation path.
    Observed,
    /// Recovered by reconciliation while retaining gap provenance.
    Recovered,
    /// Corrects a previously materialized aggregate revision.
    Correction,
}

/// Validated definition for the first Bitcoin product alert.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct QuorumVbytesAboveDefinition {
    alert_id: String,
    min_fee_rate_sat_vb: u64,
    threshold_vbytes: u64,
    quorum_required: u16,
    for_evaluations: u16,
    cooldown_seconds: u64,
    degraded_policy: DegradedPolicy,
}

impl QuorumVbytesAboveDefinition {
    /// Creates a bounded, deterministic alert definition.
    ///
    /// # Errors
    ///
    /// Rejects blank identities, zero thresholds/quorum/persistence, and a
    /// cooldown that cannot be represented by the evaluation clock.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        alert_id: impl Into<String>,
        min_fee_rate_sat_vb: u64,
        threshold_vbytes: u64,
        quorum_required: u16,
        for_evaluations: u16,
        cooldown_seconds: u64,
        degraded_policy: DegradedPolicy,
    ) -> Result<Self, MempoolAlertError> {
        let alert_id = alert_id.into();
        if !valid_identity(&alert_id) {
            return Err(MempoolAlertError::InvalidAlertId);
        }
        if threshold_vbytes == 0 {
            return Err(MempoolAlertError::ZeroThreshold);
        }
        if quorum_required == 0 {
            return Err(MempoolAlertError::ZeroQuorum);
        }
        if for_evaluations == 0 {
            return Err(MempoolAlertError::ZeroPersistence);
        }
        if i64::try_from(cooldown_seconds).is_err() {
            return Err(MempoolAlertError::CooldownOutOfRange);
        }
        Ok(Self {
            alert_id,
            min_fee_rate_sat_vb,
            threshold_vbytes,
            quorum_required,
            for_evaluations,
            cooldown_seconds,
            degraded_policy,
        })
    }

    /// Stable alert-definition identity.
    #[must_use]
    pub fn alert_id(&self) -> &str {
        &self.alert_id
    }

    /// Inclusive fee-band floor in sat/vB.
    #[must_use]
    pub const fn min_fee_rate_sat_vb(&self) -> u64 {
        self.min_fee_rate_sat_vb
    }

    /// Strict aggregate virtual-byte threshold.
    #[must_use]
    pub const fn threshold_vbytes(&self) -> u64 {
        self.threshold_vbytes
    }

    /// Minimum healthy sources required by the aggregate view.
    #[must_use]
    pub const fn quorum_required(&self) -> u16 {
        self.quorum_required
    }

    /// Consecutive true revisions required before triggering.
    #[must_use]
    pub const fn for_evaluations(&self) -> u16 {
        self.for_evaluations
    }

    /// Minimum time between trigger deliveries.
    #[must_use]
    pub const fn cooldown_seconds(&self) -> u64 {
        self.cooldown_seconds
    }

    /// Configured incomplete-source policy.
    #[must_use]
    pub const fn degraded_policy(&self) -> DegradedPolicy {
        self.degraded_policy
    }
}

/// One source-qualified quorum fee-band materialization.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct QuorumFeeBandSnapshot {
    network: String,
    revision: u64,
    observed_at_unix_seconds: i64,
    min_fee_rate_sat_vb: u64,
    vbytes: u64,
    quorum_required: u16,
    eligible_sources: Vec<String>,
    unavailable_sources: Vec<String>,
    completeness: Completeness,
    cause: SnapshotCause,
}

impl QuorumFeeBandSnapshot {
    /// Creates a normalized aggregate revision without inventing global truth.
    ///
    /// # Errors
    ///
    /// Rejects invalid identities, zero revision/quorum, overlapping source
    /// sets, and a complete claim that still names unavailable sources.
    #[allow(clippy::too_many_arguments)]
    pub fn new<E, ES, U, US>(
        network: impl Into<String>,
        revision: u64,
        observed_at_unix_seconds: i64,
        min_fee_rate_sat_vb: u64,
        vbytes: u64,
        quorum_required: u16,
        eligible_sources: E,
        unavailable_sources: U,
        completeness: Completeness,
        cause: SnapshotCause,
    ) -> Result<Self, MempoolAlertError>
    where
        E: IntoIterator<Item = ES>,
        ES: Into<String>,
        U: IntoIterator<Item = US>,
        US: Into<String>,
    {
        let network = network.into();
        if !valid_identity(&network) {
            return Err(MempoolAlertError::InvalidNetwork);
        }
        if revision == 0 {
            return Err(MempoolAlertError::ZeroRevision);
        }
        if observed_at_unix_seconds < 0 {
            return Err(MempoolAlertError::NegativeEvaluationTime);
        }
        if quorum_required == 0 {
            return Err(MempoolAlertError::ZeroQuorum);
        }
        let eligible_sources = normalized_sources(eligible_sources)?;
        let unavailable_sources = normalized_sources(unavailable_sources)?;
        if eligible_sources.is_empty() && unavailable_sources.is_empty() {
            return Err(MempoolAlertError::MissingSources);
        }
        if eligible_sources
            .iter()
            .any(|source| unavailable_sources.binary_search(source).is_ok())
        {
            return Err(MempoolAlertError::OverlappingSources);
        }
        if completeness == Completeness::Complete && !unavailable_sources.is_empty() {
            return Err(MempoolAlertError::ContradictoryCompleteness);
        }
        if eligible_sources.len() + unavailable_sources.len() > MAX_SOURCE_COUNT {
            return Err(MempoolAlertError::TooManySources);
        }
        Ok(Self {
            network,
            revision,
            observed_at_unix_seconds,
            min_fee_rate_sat_vb,
            vbytes,
            quorum_required,
            eligible_sources,
            unavailable_sources,
            completeness,
            cause,
        })
    }

    /// Bitcoin network identity.
    #[must_use]
    pub fn network(&self) -> &str {
        &self.network
    }

    /// Monotonic materialization revision.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Evaluation clock supplied by the ordered input.
    #[must_use]
    pub const fn observed_at_unix_seconds(&self) -> i64 {
        self.observed_at_unix_seconds
    }

    /// Aggregate virtual bytes at or above the fee-band floor.
    #[must_use]
    pub const fn vbytes(&self) -> u64 {
        self.vbytes
    }

    /// Sorted healthy sources eligible for this revision.
    #[must_use]
    pub fn eligible_sources(&self) -> &[String] {
        &self.eligible_sources
    }

    /// Sorted configured sources unavailable to this revision.
    #[must_use]
    pub fn unavailable_sources(&self) -> &[String] {
        &self.unavailable_sources
    }

    /// Reader-facing completeness state.
    #[must_use]
    pub const fn completeness(&self) -> Completeness {
        self.completeness
    }

    /// Revision provenance.
    #[must_use]
    pub const fn cause(&self) -> SnapshotCause {
        self.cause
    }
}

/// State transition produced by one ordered input revision.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertTransition {
    /// Threshold is true but has not persisted long enough.
    Pending,
    /// Alert became active and requires one delivery.
    Triggered,
    /// Alert remains active without another delivery.
    Confirmed,
    /// A correction invalidated an active alert.
    Corrected,
    /// A normal revision moved an active alert below threshold.
    Retracted,
    /// Source evidence cannot safely satisfy the configured policy.
    DegradedSource,
    /// Alert is inactive and the threshold is false.
    BelowThreshold,
    /// A trigger was withheld inside its cooldown window.
    CooldownSuppressed,
    /// An identical revision was replayed and produced no new side effect.
    DuplicateRevision,
}

/// Fully evidenced result of evaluating one alert revision.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MempoolAlertEvaluation {
    /// Stable hash of definition, normalized evidence, and transition.
    pub evaluation_id: String,
    /// Definition identity.
    pub alert_id: String,
    /// Stable alert kind.
    pub kind: &'static str,
    /// Current transition.
    pub transition: AlertTransition,
    /// Whether this result must enter the transactional outbox.
    pub delivery_required: bool,
    /// Idempotency key for a required outbox delivery.
    pub outbox_idempotency_key: Option<String>,
    /// Whether the alert is active after evaluation.
    pub active: bool,
    /// Bitcoin network identity.
    pub network: String,
    /// Input materialization revision.
    pub revision: u64,
    /// Inclusive fee-band floor in sat/vB.
    pub min_fee_rate_sat_vb: u64,
    /// Strict aggregate vbyte threshold.
    pub threshold_vbytes: u64,
    /// Healthy-source quorum required by the definition and snapshot.
    pub quorum_required: u16,
    /// Consecutive threshold-true revision count.
    pub consecutive_true_evaluations: u16,
    /// Aggregate vbytes in the configured band.
    pub observed_vbytes: u64,
    /// Sorted healthy source evidence.
    pub contributing_sources: Vec<String>,
    /// Sorted unavailable source evidence.
    pub unavailable_sources: Vec<String>,
    /// Retained completeness state.
    pub completeness: Completeness,
    /// Retained observation or correction provenance.
    pub cause: SnapshotCause,
}

/// Stateful deterministic evaluator for one definition and ordered revision stream.
#[derive(Clone, Debug)]
pub struct MempoolAlertEvaluator {
    definition: QuorumVbytesAboveDefinition,
    consecutive_true: u16,
    active: bool,
    last_revision: Option<u64>,
    last_observed_at_unix_seconds: Option<i64>,
    last_snapshot: Option<QuorumFeeBandSnapshot>,
    last_evaluation_id: Option<String>,
    last_trigger_delivery_at_unix_seconds: Option<i64>,
}

impl MempoolAlertEvaluator {
    /// Starts an empty evaluator for one immutable definition version.
    #[must_use]
    pub const fn new(definition: QuorumVbytesAboveDefinition) -> Self {
        Self {
            definition,
            consecutive_true: 0,
            active: false,
            last_revision: None,
            last_observed_at_unix_seconds: None,
            last_snapshot: None,
            last_evaluation_id: None,
            last_trigger_delivery_at_unix_seconds: None,
        }
    }

    /// Evaluates the next ordered snapshot.
    ///
    /// # Errors
    ///
    /// Rejects incompatible definitions, conflicting or regressing revisions,
    /// and evaluation-clock regression.
    pub fn evaluate(
        &mut self,
        snapshot: &QuorumFeeBandSnapshot,
    ) -> Result<MempoolAlertEvaluation, MempoolAlertError> {
        self.validate_order_and_contract(snapshot)?;
        if self.last_snapshot.as_ref() == Some(snapshot) {
            return Ok(self.duplicate_evaluation(snapshot));
        }

        let insufficient_quorum =
            snapshot.eligible_sources.len() < usize::from(self.definition.quorum_required);
        let incomplete = snapshot.completeness == Completeness::KnownIncomplete
            || !snapshot.unavailable_sources.is_empty();
        let policy_suppresses =
            incomplete && self.definition.degraded_policy == DegradedPolicy::Suppress;

        let transition = if insufficient_quorum || policy_suppresses {
            self.consecutive_true = 0;
            AlertTransition::DegradedSource
        } else if snapshot.vbytes <= self.definition.threshold_vbytes {
            self.consecutive_true = 0;
            if self.active {
                self.active = false;
                if snapshot.cause == SnapshotCause::Correction {
                    AlertTransition::Corrected
                } else {
                    AlertTransition::Retracted
                }
            } else {
                AlertTransition::BelowThreshold
            }
        } else {
            self.consecutive_true = self.consecutive_true.saturating_add(1);
            if self.active {
                AlertTransition::Confirmed
            } else if self.consecutive_true < self.definition.for_evaluations {
                AlertTransition::Pending
            } else if self.inside_cooldown(snapshot.observed_at_unix_seconds) {
                AlertTransition::CooldownSuppressed
            } else {
                self.active = true;
                self.last_trigger_delivery_at_unix_seconds =
                    Some(snapshot.observed_at_unix_seconds);
                AlertTransition::Triggered
            }
        };

        let delivery_required = matches!(
            transition,
            AlertTransition::Triggered | AlertTransition::Corrected | AlertTransition::Retracted
        );
        let evaluation_id = evaluation_id(&self.definition, snapshot, transition);
        let outbox_idempotency_key = delivery_required
            .then(|| digest_hex(&[b"multichain.alert.outbox.v1", evaluation_id.as_bytes()]));
        let evaluation = MempoolAlertEvaluation {
            evaluation_id: evaluation_id.clone(),
            alert_id: self.definition.alert_id.clone(),
            kind: "bitcoin_mempool_quorum_vbytes_above",
            transition,
            delivery_required,
            outbox_idempotency_key,
            active: self.active,
            network: snapshot.network.clone(),
            revision: snapshot.revision,
            min_fee_rate_sat_vb: snapshot.min_fee_rate_sat_vb,
            threshold_vbytes: self.definition.threshold_vbytes,
            quorum_required: snapshot.quorum_required,
            consecutive_true_evaluations: self.consecutive_true,
            observed_vbytes: snapshot.vbytes,
            contributing_sources: snapshot.eligible_sources.clone(),
            unavailable_sources: snapshot.unavailable_sources.clone(),
            completeness: snapshot.completeness,
            cause: snapshot.cause,
        };
        self.last_revision = Some(snapshot.revision);
        self.last_observed_at_unix_seconds = Some(snapshot.observed_at_unix_seconds);
        self.last_snapshot = Some(snapshot.clone());
        self.last_evaluation_id = Some(evaluation_id);
        Ok(evaluation)
    }

    fn validate_order_and_contract(
        &self,
        snapshot: &QuorumFeeBandSnapshot,
    ) -> Result<(), MempoolAlertError> {
        if snapshot.min_fee_rate_sat_vb != self.definition.min_fee_rate_sat_vb {
            return Err(MempoolAlertError::FeeBandMismatch);
        }
        if snapshot.quorum_required != self.definition.quorum_required {
            return Err(MempoolAlertError::QuorumMismatch);
        }
        if let Some(last_revision) = self.last_revision {
            if snapshot.revision < last_revision {
                return Err(MempoolAlertError::RevisionRegression {
                    current: last_revision,
                    attempted: snapshot.revision,
                });
            }
            if snapshot.revision == last_revision && self.last_snapshot.as_ref() != Some(snapshot) {
                return Err(MempoolAlertError::ConflictingRevision {
                    revision: snapshot.revision,
                });
            }
        }
        if self
            .last_observed_at_unix_seconds
            .is_some_and(|last| snapshot.observed_at_unix_seconds < last)
        {
            return Err(MempoolAlertError::EvaluationTimeRegression);
        }
        Ok(())
    }

    fn duplicate_evaluation(&self, snapshot: &QuorumFeeBandSnapshot) -> MempoolAlertEvaluation {
        MempoolAlertEvaluation {
            evaluation_id: self
                .last_evaluation_id
                .clone()
                .expect("a duplicate snapshot always has a prior evaluation"),
            alert_id: self.definition.alert_id.clone(),
            kind: "bitcoin_mempool_quorum_vbytes_above",
            transition: AlertTransition::DuplicateRevision,
            delivery_required: false,
            outbox_idempotency_key: None,
            active: self.active,
            network: snapshot.network.clone(),
            revision: snapshot.revision,
            min_fee_rate_sat_vb: snapshot.min_fee_rate_sat_vb,
            threshold_vbytes: self.definition.threshold_vbytes,
            quorum_required: snapshot.quorum_required,
            consecutive_true_evaluations: self.consecutive_true,
            observed_vbytes: snapshot.vbytes,
            contributing_sources: snapshot.eligible_sources.clone(),
            unavailable_sources: snapshot.unavailable_sources.clone(),
            completeness: snapshot.completeness,
            cause: snapshot.cause,
        }
    }

    fn inside_cooldown(&self, observed_at_unix_seconds: i64) -> bool {
        let cooldown = i64::try_from(self.definition.cooldown_seconds)
            .expect("definition validation bounds cooldown");
        self.last_trigger_delivery_at_unix_seconds
            .is_some_and(|last| observed_at_unix_seconds < last.saturating_add(cooldown))
    }
}

/// Alert definition, evidence, or ordering failure.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum MempoolAlertError {
    /// Definition identity is blank or non-ASCII.
    #[error("mempool alert ID must be non-empty ASCII")]
    InvalidAlertId,
    /// Network identity is blank or non-ASCII.
    #[error("mempool alert network must be non-empty ASCII")]
    InvalidNetwork,
    /// Aggregate vbyte threshold is zero.
    #[error("mempool alert vbyte threshold must be positive")]
    ZeroThreshold,
    /// Quorum zero cannot support a source-qualified decision.
    #[error("mempool alert quorum must be positive")]
    ZeroQuorum,
    /// Persistence window zero can never be evaluated.
    #[error("mempool alert persistence count must be positive")]
    ZeroPersistence,
    /// Cooldown exceeds the signed evaluation clock range.
    #[error("mempool alert cooldown is out of range")]
    CooldownOutOfRange,
    /// Snapshot revision zero is reserved.
    #[error("mempool alert snapshot revision must be positive")]
    ZeroRevision,
    /// Bitcoin observations cannot predate the Unix epoch.
    #[error("mempool alert evaluation time must not be negative")]
    NegativeEvaluationTime,
    /// Snapshot contains no configured source evidence.
    #[error("mempool alert snapshot requires source evidence")]
    MissingSources,
    /// Source identity is blank or non-ASCII.
    #[error("mempool alert source identity must be non-empty ASCII")]
    InvalidSource,
    /// One source appears in healthy and unavailable sets.
    #[error("mempool alert source sets must not overlap")]
    OverlappingSources,
    /// Source count exceeds the bounded API contract.
    #[error("mempool alert source count exceeds 32")]
    TooManySources,
    /// Complete evidence cannot name unavailable sources.
    #[error("complete mempool alert evidence cannot include unavailable sources")]
    ContradictoryCompleteness,
    /// Definition and snapshot select different fee bands.
    #[error("mempool alert fee band does not match definition")]
    FeeBandMismatch,
    /// Definition and snapshot use different source quorum.
    #[error("mempool alert quorum does not match definition")]
    QuorumMismatch,
    /// Input revision moved backwards.
    #[error("mempool alert revision regressed from {current} to {attempted}")]
    RevisionRegression {
        /// Last accepted revision.
        current: u64,
        /// Regressing revision.
        attempted: u64,
    },
    /// One revision was reused for different evidence.
    #[error("mempool alert revision {revision} has conflicting evidence")]
    ConflictingRevision {
        /// Conflicting revision.
        revision: u64,
    },
    /// Supplied evaluation time moved backwards.
    #[error("mempool alert evaluation time regressed")]
    EvaluationTimeRegression,
}

fn normalized_sources<I, S>(sources: I) -> Result<Vec<String>, MempoolAlertError>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut sources = sources.into_iter().map(Into::into).collect::<Vec<_>>();
    if sources.iter().any(|source| !valid_identity(source)) {
        return Err(MempoolAlertError::InvalidSource);
    }
    sources.sort_unstable();
    sources.dedup();
    Ok(sources)
}

fn valid_identity(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_graphic())
}

fn evaluation_id(
    definition: &QuorumVbytesAboveDefinition,
    snapshot: &QuorumFeeBandSnapshot,
    transition: AlertTransition,
) -> String {
    let revision = snapshot.revision.to_le_bytes();
    let observed_at = snapshot.observed_at_unix_seconds.to_le_bytes();
    let min_fee_rate = snapshot.min_fee_rate_sat_vb.to_le_bytes();
    let vbytes = snapshot.vbytes.to_le_bytes();
    let quorum = snapshot.quorum_required.to_le_bytes();
    let threshold = definition.threshold_vbytes.to_le_bytes();
    let persistence = definition.for_evaluations.to_le_bytes();
    let cooldown = definition.cooldown_seconds.to_le_bytes();
    let degraded_policy = [definition.degraded_policy as u8];
    let transition = [transition as u8];
    let completeness = [snapshot.completeness as u8];
    let cause = [snapshot.cause as u8];
    let mut fields = vec![
        b"multichain.alert.bitcoin_mempool_quorum_vbytes_above.v1".as_slice(),
        definition.alert_id.as_bytes(),
        snapshot.network.as_bytes(),
        &revision,
        &observed_at,
        &min_fee_rate,
        &vbytes,
        &quorum,
        &threshold,
        &persistence,
        &cooldown,
        &degraded_policy,
        &transition,
    ];
    for source in &snapshot.eligible_sources {
        fields.push(source.as_bytes());
    }
    fields.push(b"unavailable");
    for source in &snapshot.unavailable_sources {
        fields.push(source.as_bytes());
    }
    fields.push(&completeness);
    fields.push(&cause);
    digest_hex(&fields)
}

fn digest_hex(fields: &[&[u8]]) -> String {
    let mut hasher = blake3::Hasher::new();
    for field in fields {
        hasher.update(&u64::try_from(field.len()).unwrap_or(u64::MAX).to_le_bytes());
        hasher.update(field);
    }
    hasher
        .finalize()
        .as_bytes()
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            write!(hex, "{byte:02x}").expect("writing into a String cannot fail");
            hex
        })
}
