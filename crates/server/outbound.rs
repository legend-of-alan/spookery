//! One socket's outbound queue. A frame is counted against the frame and byte caps
//! from the moment it is queued until the writer has sent it.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use super::limits::{OUTBOUND_BYTE_CAP, OUTBOUND_FRAME_CAP};

pub(super) enum Message {
    Frame(Vec<u8>),
    /// Flushed, then acknowledged, then the writer shuts the socket down.
    Last(Vec<u8>, tokio::sync::oneshot::Sender<()>),
}

pub(super) type CancelTx = tokio::sync::watch::Sender<bool>;

/// Sender half of one socket's outbound queue.
#[derive(Clone)]
pub(super) struct OutTx {
    tx: tokio::sync::mpsc::Sender<Message>,
    queued_bytes: Arc<AtomicUsize>,
}

/// Receiver half. A frame's bytes stay counted until it is written.
pub(super) struct OutRx {
    rx: tokio::sync::mpsc::Receiver<Message>,
    queued: Arc<AtomicUsize>,
}

impl Message {
    pub(super) fn len(&self) -> usize {
        match self {
            Message::Frame(bytes) | Message::Last(bytes, _) => bytes.len(),
        }
    }
}

impl OutTx {
    fn try_send(&self, item: Message) -> bool {
        let len = item.len();
        if !self.reserve(len) {
            return false;
        }
        if self.tx.try_send(item).is_err() {
            release(&self.queued_bytes, len);
            return false;
        }
        true
    }

    /// An empty queue takes one frame of any size, so a welcome still fits.
    fn reserve(&self, len: usize) -> bool {
        self.queued_bytes
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |cur| {
                if cur == 0 {
                    return Some(len);
                }
                let next = cur.saturating_add(len);
                if next > OUTBOUND_BYTE_CAP {
                    None
                } else {
                    Some(next)
                }
            })
            .is_ok()
    }
}

fn release(queued: &AtomicUsize, len: usize) {
    let _ = queued.fetch_update(Ordering::AcqRel, Ordering::Acquire, |cur| {
        Some(cur.saturating_sub(len))
    });
}

pub(super) fn channel() -> (OutTx, OutRx, CancelTx, tokio::sync::watch::Receiver<bool>) {
    let (tx, rx) = tokio::sync::mpsc::channel(OUTBOUND_FRAME_CAP);
    let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
    let queued = Arc::new(AtomicUsize::new(0));
    (
        OutTx {
            tx,
            queued_bytes: Arc::clone(&queued),
        },
        OutRx { rx, queued },
        cancel_tx,
        cancel_rx,
    )
}

pub(super) fn try_send_frame(tx: &OutTx, bytes: Vec<u8>) -> bool {
    tx.try_send(Message::Frame(bytes))
}

/// Queues the closing frame and waits until the writer has flushed it.
pub(super) async fn send_last(tx: &OutTx, frame: Vec<u8>) {
    let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
    if tx.try_send(Message::Last(frame, ack_tx)) {
        let _ = ack_rx.await;
    }
}

/// Sends each queued frame to `sink` until the queue closes, the seat is
/// cancelled, or `sink` fails. A frame's bytes are released once it is sent.
pub(super) async fn write_frames(
    mut sink: impl AsyncFnMut(&[u8]) -> bool,
    mut frames: OutRx,
    mut cancel: tokio::sync::watch::Receiver<bool>,
) {
    loop {
        let frame = tokio::select! {
            biased;
            frame = frames.rx.recv() => frame,
            _ = cancel.changed() => break,
        };
        let Some(frame) = frame else {
            break;
        };
        match frame {
            Message::Frame(bytes) => {
                tokio::select! {
                    biased;
                    sent = sink(&bytes) => {
                        if !sent {
                            break;
                        }
                        release(&frames.queued, bytes.len());
                    }
                    _ = cancel.changed() => break,
                }
            }
            Message::Last(bytes, ack) => {
                let _ = sink(&bytes).await;
                let _ = ack.send(());
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::write_all_flush;

    #[tokio::test]
    async fn the_last_frame_is_flushed_before_its_ack() {
        use tokio::io::AsyncReadExt;
        let (mut client, mut server) = tokio::io::duplex(256);
        let (tx, rx, _cancel_tx, cancel_rx) = channel();
        let writer = tokio::spawn(write_frames(
            async move |frame: &[u8]| write_all_flush(&mut server, frame).await,
            rx,
            cancel_rx,
        ));
        assert!(try_send_frame(&tx, b"one\n".to_vec()));
        let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
        assert!(tx.try_send(Message::Last(b"bye\n".to_vec(), ack_tx)));
        ack_rx.await.expect("ack");
        drop(tx);
        writer.await.expect("writer");
        let mut buf = Vec::new();
        client.read_to_end(&mut buf).await.expect("read");
        assert_eq!(buf, b"one\nbye\n");
    }

    #[test]
    fn the_frame_cap_rejects_before_the_byte_cap() {
        let (tx, _rx, _cancel_tx, _cancel_rx) = channel();
        for _ in 0..OUTBOUND_FRAME_CAP {
            assert!(try_send_frame(&tx, b"n".to_vec()));
        }
        assert!(!try_send_frame(&tx, b"n".to_vec()));
    }

    #[tokio::test]
    async fn the_byte_cap_rejects_until_a_frame_is_written() {
        let (tx, mut rx, _cancel_tx, _cancel_rx) = channel();
        assert!(try_send_frame(&tx, vec![b'a'; OUTBOUND_BYTE_CAP + 8]));
        assert!(
            !try_send_frame(&tx, b"n".to_vec()),
            "a second frame was queued past the byte cap"
        );
        let taken = rx.rx.recv().await.expect("frame");
        let Message::Frame(bytes) = taken else {
            panic!("expected a frame");
        };
        assert_eq!(bytes.len(), OUTBOUND_BYTE_CAP + 8);
        assert!(
            !try_send_frame(&tx, b"n".to_vec()),
            "a taken frame was released before it was written"
        );
        release(&rx.queued, bytes.len());
        assert!(try_send_frame(&tx, b"n".to_vec()));
    }
}
