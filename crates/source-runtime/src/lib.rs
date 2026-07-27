#![doc = "Restart-safe raw source polling and WAL-first publication runtime."]

use std::{future::Future, sync::Arc, time::Duration};

use observation_envelope::Clock;
use reqwest::{Client, StatusCode, Url};
use serde_json::Value;
use source_capture::{CaptureError, DurableSourceCapture, RawSourceMessage};
use storage_ports::{BrokerError, BrokerPublisher};
use thiserror::Error;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use wal::ObservationWal;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransportFailureKind {
    Network,
    Timeout,
    HttpRetryable,
    HttpPermanent,
    ResponseTooLarge,
}

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("transient source transport failure: {kind:?}")]
    Transient { kind: TransportFailureKind },
    #[error("permanent source transport failure: {kind:?}")]
    Permanent { kind: TransportFailureKind },
    #[error("source response exceeds the configured {max}-byte limit")]
    ResponseTooLarge { max: usize },
}

impl TransportError {
    #[must_use]
    pub const fn transient(kind: TransportFailureKind) -> Self {
        Self::Transient { kind }
    }

    #[must_use]
    pub const fn permanent(kind: TransportFailureKind) -> Self {
        Self::Permanent { kind }
    }

    #[must_use]
    pub const fn kind(&self) -> TransportFailureKind {
        match self {
            Self::Transient { kind } | Self::Permanent { kind } => *kind,
            Self::ResponseTooLarge { .. } => TransportFailureKind::ResponseTooLarge,
        }
    }

    #[must_use]
    pub const fn retryable(&self) -> bool {
        matches!(self, Self::Transient { .. })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PollDisposition {
    Success,
    RetryableFailure { kind: TransportFailureKind },
    PermanentFailure { kind: TransportFailureKind },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PollEvent {
    pub message: RawSourceMessage,
    pub disposition: PollDisposition,
    pub cycle_complete: bool,
}

impl PollEvent {
    /// Builds a successful exact source response.
    ///
    /// # Errors
    ///
    /// Rejects invalid source channel or message-type fields.
    pub fn success(
        source_channel: impl Into<String>,
        source_message_type: impl Into<String>,
        payload: impl AsRef<[u8]>,
        source_sequence: u64,
        cycle_complete: bool,
    ) -> Result<Self, CaptureError> {
        Ok(Self {
            message: RawSourceMessage::new(source_channel, source_message_type, payload)?
                .with_source_sequence(source_sequence),
            disposition: PollDisposition::Success,
            cycle_complete,
        })
    }

    /// Builds a retryable HTTP response while retaining its exact body.
    ///
    /// # Errors
    ///
    /// Rejects invalid source channel or message-type fields.
    pub fn retryable_failure(
        source_channel: impl Into<String>,
        source_message_type: impl Into<String>,
        payload: impl AsRef<[u8]>,
        source_sequence: u64,
        kind: TransportFailureKind,
    ) -> Result<Self, CaptureError> {
        Ok(Self {
            message: RawSourceMessage::new(source_channel, source_message_type, payload)?
                .with_source_sequence(source_sequence),
            disposition: PollDisposition::RetryableFailure { kind },
            cycle_complete: false,
        })
    }

    /// Builds a permanent HTTP response while retaining its exact body.
    ///
    /// # Errors
    ///
    /// Rejects invalid source channel or message-type fields.
    pub fn permanent_failure(
        source_channel: impl Into<String>,
        source_message_type: impl Into<String>,
        payload: impl AsRef<[u8]>,
        source_sequence: u64,
        kind: TransportFailureKind,
    ) -> Result<Self, CaptureError> {
        Ok(Self {
            message: RawSourceMessage::new(source_channel, source_message_type, payload)?
                .with_source_sequence(source_sequence),
            disposition: PollDisposition::PermanentFailure { kind },
            cycle_complete: false,
        })
    }
}

pub trait SourceTransport: Send {
    fn next_event(&mut self) -> impl Future<Output = Result<PollEvent, TransportError>> + Send;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BackoffPolicy {
    initial: Duration,
    maximum: Duration,
}

impl BackoffPolicy {
    /// Creates a bounded exponential backoff policy.
    ///
    /// # Errors
    ///
    /// Rejects zero durations or an initial delay above the maximum.
    pub fn new(initial: Duration, maximum: Duration) -> Result<Self, SourceLoopError> {
        if initial.is_zero() || maximum.is_zero() || initial > maximum {
            return Err(SourceLoopError::InvalidConfig(
                "backoff durations must be non-zero and initial must not exceed maximum",
            ));
        }
        Ok(Self { initial, maximum })
    }

    fn delay(self, consecutive_failures: u64) -> Duration {
        let shift = consecutive_failures.saturating_sub(1).min(31) as u32;
        self.initial
            .checked_mul(1_u32 << shift)
            .unwrap_or(self.maximum)
            .min(self.maximum)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceLoopConfig {
    poll_interval: Duration,
    backoff: BackoffPolicy,
}

impl SourceLoopConfig {
    /// Creates polling and retry timing configuration.
    ///
    /// # Errors
    ///
    /// Rejects a zero polling interval.
    pub fn new(poll_interval: Duration, backoff: BackoffPolicy) -> Result<Self, SourceLoopError> {
        if poll_interval.is_zero() {
            return Err(SourceLoopError::InvalidConfig(
                "poll interval must be non-zero",
            ));
        }
        Ok(Self {
            poll_interval,
            backoff,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceState {
    Starting,
    Healthy,
    Degraded,
    Failed,
    Stopped,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IncompleteInterval {
    pub opened_at_unix_ns: i64,
    pub closed_at_unix_ns: Option<i64>,
    pub failure_count: u64,
    pub reason: TransportFailureKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceHealth {
    pub state: SourceState,
    pub successful_observations: u64,
    pub next_collector_sequence: u64,
    pub consecutive_failures: u64,
    pub active_interval: Option<IncompleteInterval>,
    pub last_closed_interval: Option<IncompleteInterval>,
}

impl SourceHealth {
    fn new(next_collector_sequence: u64) -> Self {
        Self {
            state: SourceState::Starting,
            successful_observations: 0,
            next_collector_sequence,
            consecutive_failures: 0,
            active_interval: None,
            last_closed_interval: None,
        }
    }

    fn record_failure(&mut self, now_unix_ns: i64, kind: TransportFailureKind) {
        self.state = SourceState::Degraded;
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        match &mut self.active_interval {
            Some(interval) => interval.failure_count = interval.failure_count.saturating_add(1),
            None => {
                self.active_interval = Some(IncompleteInterval {
                    opened_at_unix_ns: now_unix_ns,
                    closed_at_unix_ns: None,
                    failure_count: 1,
                    reason: kind,
                });
            }
        }
    }

    fn record_success(&mut self, now_unix_ns: i64, next_collector_sequence: u64) {
        self.state = SourceState::Healthy;
        self.successful_observations = self.successful_observations.saturating_add(1);
        self.next_collector_sequence = next_collector_sequence;
        self.consecutive_failures = 0;
        if let Some(mut interval) = self.active_interval.take() {
            interval.closed_at_unix_ns = Some(now_unix_ns);
            self.last_closed_interval = Some(interval);
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunExit {
    CycleComplete,
    Cancelled,
}

#[derive(Debug, Error)]
pub enum SourceLoopError {
    #[error("invalid source loop configuration: {0}")]
    InvalidConfig(&'static str),
    #[error("raw source capture failed: {0}")]
    Capture(#[from] CaptureError),
    #[error("broker publication failed after WAL commit: {0}")]
    Broker(#[from] BrokerError),
    #[error("permanent source transport failure: {0}")]
    Transport(#[from] TransportError),
}

pub struct SourceLoop<T, W, B>
where
    T: SourceTransport,
    W: ObservationWal,
    B: BrokerPublisher,
{
    transport: T,
    capture: DurableSourceCapture<W>,
    broker: B,
    topic: String,
    clock: Arc<dyn Clock>,
    cancellation: CancellationToken,
    config: SourceLoopConfig,
    health: SourceHealth,
}

impl<T, W, B> SourceLoop<T, W, B>
where
    T: SourceTransport,
    W: ObservationWal,
    B: BrokerPublisher,
{
    /// Binds one transport, capture session, broker route, and shutdown token.
    ///
    /// # Errors
    ///
    /// Rejects an empty broker topic.
    pub fn new(
        transport: T,
        capture: DurableSourceCapture<W>,
        broker: B,
        topic: impl Into<String>,
        clock: Arc<dyn Clock>,
        cancellation: CancellationToken,
        config: SourceLoopConfig,
    ) -> Result<Self, SourceLoopError> {
        let topic = topic.into();
        if topic.trim().is_empty() {
            return Err(SourceLoopError::InvalidConfig("topic must not be empty"));
        }
        let next_collector_sequence = capture.session().next_sequence();
        Ok(Self {
            transport,
            capture,
            broker,
            topic,
            clock,
            cancellation,
            config,
            health: SourceHealth::new(next_collector_sequence),
        })
    }

    #[must_use]
    pub const fn health(&self) -> &SourceHealth {
        &self.health
    }

    #[must_use]
    pub fn into_parts(self) -> (T, DurableSourceCapture<W>, B) {
        (self.transport, self.capture, self.broker)
    }

    /// Runs until every request in one polling cycle succeeds.
    ///
    /// # Errors
    ///
    /// Stops on WAL, broker, permanent transport, or permanent HTTP failures.
    pub async fn run_until_cycle_complete(&mut self) -> Result<RunExit, SourceLoopError> {
        loop {
            let event = tokio::select! {
                () = self.cancellation.cancelled() => {
                    self.health.state = SourceState::Stopped;
                    return Ok(RunExit::Cancelled);
                }
                result = self.transport.next_event() => {
                    match result {
                        Ok(event) => event,
                        Err(error) if error.retryable() => {
                            self.record_failure(error.kind()).await?;
                            continue;
                        }
                        Err(error) => {
                            self.health.state = SourceState::Failed;
                            return Err(SourceLoopError::Transport(error));
                        }
                    }
                }
            };

            let committed = match self.capture.capture(event.message) {
                Ok(committed) => committed,
                Err(error) => {
                    self.health.state = SourceState::Failed;
                    return Err(SourceLoopError::Capture(error));
                }
            };
            if let Err(error) = self.broker.publish(&self.topic, &[committed]).await {
                self.health.state = SourceState::Failed;
                return Err(SourceLoopError::Broker(error));
            }
            self.health.next_collector_sequence = self.capture.session().next_sequence();

            match event.disposition {
                PollDisposition::Success => {
                    self.health.record_success(
                        self.clock.wall_time_unix_ns(),
                        self.capture.session().next_sequence(),
                    );
                    if event.cycle_complete {
                        return Ok(RunExit::CycleComplete);
                    }
                }
                PollDisposition::RetryableFailure { kind } => {
                    self.record_failure(kind).await?;
                }
                PollDisposition::PermanentFailure { kind } => {
                    self.health.state = SourceState::Failed;
                    return Err(SourceLoopError::Transport(TransportError::permanent(kind)));
                }
            }
        }
    }

    /// Runs polling cycles until cancellation.
    ///
    /// # Errors
    ///
    /// Stops on WAL, broker, permanent transport, or permanent HTTP failures.
    pub async fn run_until_cancelled(&mut self) -> Result<RunExit, SourceLoopError> {
        loop {
            if self.cancellation.is_cancelled() {
                self.health.state = SourceState::Stopped;
                return Ok(RunExit::Cancelled);
            }
            if self.run_until_cycle_complete().await? == RunExit::Cancelled {
                return Ok(RunExit::Cancelled);
            }
            if self.cancellation.is_cancelled() {
                self.health.state = SourceState::Stopped;
                return Ok(RunExit::Cancelled);
            }
            tokio::select! {
                () = self.cancellation.cancelled() => {
                    self.health.state = SourceState::Stopped;
                    return Ok(RunExit::Cancelled);
                }
                () = sleep(self.config.poll_interval) => {}
            }
        }
    }

    async fn record_failure(&mut self, kind: TransportFailureKind) -> Result<(), SourceLoopError> {
        self.health
            .record_failure(self.clock.wall_time_unix_ns(), kind);
        let delay = self.config.backoff.delay(self.health.consecutive_failures);
        tokio::select! {
            () = self.cancellation.cancelled() => {
                self.health.state = SourceState::Stopped;
            }
            () = sleep(delay) => {}
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HttpMethod {
    Get,
    Post,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HttpRequestSpec {
    method: HttpMethod,
    url: Url,
    source_channel: String,
    source_message_type: String,
    body: Option<Vec<u8>>,
}

impl HttpRequestSpec {
    /// Builds a validated HTTP GET request.
    ///
    /// # Errors
    ///
    /// Rejects unsafe URLs and invalid source metadata.
    pub fn get(
        url: impl AsRef<str>,
        source_channel: impl Into<String>,
        source_message_type: impl Into<String>,
    ) -> Result<Self, SourceLoopError> {
        Self::new(
            HttpMethod::Get,
            url.as_ref(),
            source_channel.into(),
            source_message_type.into(),
            None,
        )
    }

    /// Builds a validated JSON-RPC POST request.
    ///
    /// # Errors
    ///
    /// Rejects unsafe URLs, invalid source metadata, or unserializable
    /// parameters.
    pub fn json_rpc(
        url: impl AsRef<str>,
        source_channel: impl Into<String>,
        source_message_type: impl Into<String>,
        method: impl Into<String>,
        params: &Value,
        id: u64,
    ) -> Result<Self, SourceLoopError> {
        let body = serde_json::to_vec(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": method.into(),
            "params": params,
            "id": id,
        }))
        .map_err(|_| SourceLoopError::InvalidConfig("JSON-RPC request is not serializable"))?;
        Self::new(
            HttpMethod::Post,
            url.as_ref(),
            source_channel.into(),
            source_message_type.into(),
            Some(body),
        )
    }

    fn new(
        method: HttpMethod,
        url: &str,
        source_channel: String,
        source_message_type: String,
        body: Option<Vec<u8>>,
    ) -> Result<Self, SourceLoopError> {
        let url =
            Url::parse(url).map_err(|_| SourceLoopError::InvalidConfig("source URL is invalid"))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(SourceLoopError::InvalidConfig(
                "HTTP polling requires an HTTP or HTTPS URL",
            ));
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(SourceLoopError::InvalidConfig(
                "credentials must not be embedded in source URLs",
            ));
        }
        if source_channel.trim().is_empty() || source_message_type.trim().is_empty() {
            return Err(SourceLoopError::InvalidConfig(
                "source channel and message type must not be empty",
            ));
        }
        Ok(Self {
            method,
            url,
            source_channel,
            source_message_type,
            body,
        })
    }

    #[must_use]
    pub const fn method(&self) -> HttpMethod {
        self.method
    }

    #[must_use]
    pub fn url(&self) -> &str {
        self.url.as_str()
    }

    #[must_use]
    pub fn source_channel(&self) -> &str {
        &self.source_channel
    }

    #[must_use]
    pub fn source_message_type(&self) -> &str {
        &self.source_message_type
    }

    #[must_use]
    pub fn body(&self) -> Option<&[u8]> {
        self.body.as_deref()
    }
}

pub struct HttpPollingTransport {
    client: Client,
    requests: Vec<HttpRequestSpec>,
    request_index: usize,
    next_source_sequence: u64,
    max_response_bytes: usize,
}

impl HttpPollingTransport {
    /// Creates a bounded sequential HTTP polling transport.
    ///
    /// # Errors
    ///
    /// Rejects empty plans, zero timeouts, zero response limits, or invalid
    /// client configuration.
    pub fn new(
        requests: impl IntoIterator<Item = HttpRequestSpec>,
        timeout: Duration,
        max_response_bytes: usize,
    ) -> Result<Self, SourceLoopError> {
        let requests = requests.into_iter().collect::<Vec<_>>();
        if requests.is_empty() {
            return Err(SourceLoopError::InvalidConfig(
                "HTTP polling requires at least one request",
            ));
        }
        if timeout.is_zero() || max_response_bytes == 0 {
            return Err(SourceLoopError::InvalidConfig(
                "HTTP timeout and response limit must be non-zero",
            ));
        }
        let client = Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| SourceLoopError::InvalidConfig("HTTP client configuration is invalid"))?;
        Ok(Self {
            client,
            requests,
            request_index: 0,
            next_source_sequence: 0,
            max_response_bytes,
        })
    }

    async fn fetch(
        &self,
        request: &HttpRequestSpec,
    ) -> Result<(StatusCode, Vec<u8>), TransportError> {
        let builder = match request.method {
            HttpMethod::Get => self.client.get(request.url.clone()),
            HttpMethod::Post => self
                .client
                .post(request.url.clone())
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(request.body.clone().unwrap_or_default()),
        };
        let mut response = builder
            .send()
            .await
            .map_err(|error| map_reqwest_error(&error))?;
        if response
            .content_length()
            .is_some_and(|length| length > self.max_response_bytes as u64)
        {
            return Err(TransportError::ResponseTooLarge {
                max: self.max_response_bytes,
            });
        }
        let status = response.status();
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| map_reqwest_error(&error))?
        {
            if body.len().saturating_add(chunk.len()) > self.max_response_bytes {
                return Err(TransportError::ResponseTooLarge {
                    max: self.max_response_bytes,
                });
            }
            body.extend_from_slice(&chunk);
        }
        Ok((status, body))
    }
}

impl SourceTransport for HttpPollingTransport {
    async fn next_event(&mut self) -> Result<PollEvent, TransportError> {
        let request = &self.requests[self.request_index];
        let (status, body) = self.fetch(request).await?;
        let source_sequence = self.next_source_sequence;
        self.next_source_sequence = self.next_source_sequence.saturating_add(1);

        if status.is_success() {
            self.request_index += 1;
            let cycle_complete = self.request_index == self.requests.len();
            if cycle_complete {
                self.request_index = 0;
            }
            return PollEvent::success(
                request.source_channel.clone(),
                request.source_message_type.clone(),
                body,
                source_sequence,
                cycle_complete,
            )
            .map_err(|_| TransportError::permanent(TransportFailureKind::HttpPermanent));
        }

        let retryable = matches!(
            status,
            StatusCode::REQUEST_TIMEOUT | StatusCode::TOO_EARLY | StatusCode::TOO_MANY_REQUESTS
        ) || status.is_server_error();
        if retryable {
            PollEvent::retryable_failure(
                request.source_channel.clone(),
                request.source_message_type.clone(),
                body,
                source_sequence,
                TransportFailureKind::HttpRetryable,
            )
            .map_err(|_| TransportError::permanent(TransportFailureKind::HttpPermanent))
        } else {
            PollEvent::permanent_failure(
                request.source_channel.clone(),
                request.source_message_type.clone(),
                body,
                source_sequence,
                TransportFailureKind::HttpPermanent,
            )
            .map_err(|_| TransportError::permanent(TransportFailureKind::HttpPermanent))
        }
    }
}

fn map_reqwest_error(error: &reqwest::Error) -> TransportError {
    if error.is_timeout() {
        TransportError::transient(TransportFailureKind::Timeout)
    } else {
        TransportError::transient(TransportFailureKind::Network)
    }
}
