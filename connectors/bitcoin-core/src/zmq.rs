use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use zeromq::{Socket, SocketRecv};

use crate::error::ZmqFrameError;

/// Validated source notification without Bitcoin consensus decoding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ZmqNotification {
    pub topic: String,
    pub body: Vec<u8>,
    pub transport_sequence: u32,
}

/// Parses Bitcoin Core's three-part ZMQ notification format.
///
/// # Errors
///
/// Returns framing, topic, sequence, or size-bound violations.
pub fn parse_multipart(
    parts: &[impl AsRef<[u8]>],
    maximum_body_bytes: usize,
) -> Result<ZmqNotification, ZmqFrameError> {
    if parts.len() != 3 {
        return Err(ZmqFrameError::InvalidPartCount);
    }
    let topic = std::str::from_utf8(parts[0].as_ref())
        .map_err(|_| ZmqFrameError::InvalidTopic)?
        .to_owned();
    if !matches!(topic.as_str(), "rawtx" | "rawblock" | "sequence") {
        return Err(ZmqFrameError::UnsupportedTopic);
    }
    let body = parts[1].as_ref();
    if body.len() > maximum_body_bytes {
        return Err(ZmqFrameError::BodyTooLarge {
            topic,
            actual: body.len(),
            maximum: maximum_body_bytes,
        });
    }
    let sequence: [u8; 4] = parts[2]
        .as_ref()
        .try_into()
        .map_err(|_| ZmqFrameError::InvalidSequence)?;
    Ok(ZmqNotification {
        topic,
        body: body.to_vec(),
        transport_sequence: u32::from_le_bytes(sequence),
    })
}

/// Connects one bounded subscriber and forwards validated notifications.
///
/// # Errors
///
/// Returns socket, framing, or channel-closure errors.
pub async fn receive_topic(
    endpoint: &str,
    topic: &str,
    maximum_body_bytes: usize,
    sender: mpsc::Sender<ZmqNotification>,
    cancellation: CancellationToken,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut socket = zeromq::SubSocket::new();
    socket.connect(endpoint).await?;
    socket.subscribe(topic).await?;
    loop {
        let message = tokio::select! {
            () = cancellation.cancelled() => return Ok(()),
            result = socket.recv() => result?,
        };
        let parts = message.into_vec();
        let notification = parse_multipart(&parts, maximum_body_bytes)?;
        sender.send(notification).await?;
    }
}
