//! Each connection's capped outbound queue and its writer.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{ready, Context, Poll};

use tokio::sync::{mpsc, watch};

use super::limits::{OUTBOUND_BYTE_CAP, OUTBOUND_FRAME_CAP};

/// The host's end of one connection's frame queue. The host sends that connection frames with
/// it. Also holds the connection's cancel: dropping it cancels the connection.
pub(super) struct FrameSender {
    tx: mpsc::Sender<Vec<u8>>,
    queued_bytes: Arc<AtomicUsize>,
    _cancel: watch::Sender<()>,
}

/// The writer's end of one connection's frame queue. That connection's socket or event writer
/// reads from it.
/// A frame's bytes stay counted until it is written.
pub(super) struct FrameReceiver {
    rx: mpsc::Receiver<Vec<u8>>,
    queued_bytes: Arc<AtomicUsize>,
    pub(super) cancel: watch::Receiver<()>,
}

impl FrameSender {
    /// Queues one frame. False when the queue is full or closed.
    pub(super) fn send(&self, frame: Vec<u8>) -> bool {
        let len = frame.len();
        if !self.reserve(len) {
            return false;
        }
        if self.tx.try_send(frame).is_err() {
            release(&self.queued_bytes, len);
            return false;
        }
        true
    }

    /// An empty queue takes one frame of any size, so a welcome still fits.
    fn reserve(&self, len: usize) -> bool {
        self.queued_bytes
            .try_update(Ordering::AcqRel, Ordering::Acquire, |cur| {
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

impl FrameReceiver {
    /// The next frame, or `None` once the queue is closed and empty. Its bytes are released
    /// as it is handed on, for a writer that cannot see the send.
    pub(super) fn poll_frame(&mut self, cx: &mut Context<'_>) -> Poll<Option<Vec<u8>>> {
        let frame = ready!(self.rx.poll_recv(cx));
        if let Some(bytes) = &frame {
            release(&self.queued_bytes, bytes.len());
        }
        Poll::Ready(frame)
    }
}

fn release(queued: &AtomicUsize, len: usize) {
    queued.update(Ordering::AcqRel, Ordering::Acquire, |cur| {
        cur.saturating_sub(len)
    });
}

/// One connection's frame queue. Like `mpsc::channel()`, it returns the sender and
/// receiver halves. A frame is counted against `OUTBOUND_FRAME_CAP` and
/// `OUTBOUND_BYTE_CAP` from the moment it is queued until the writer has sent
/// it.
pub(super) fn frame_queue() -> (FrameSender, FrameReceiver) {
    let (tx, rx) = mpsc::channel(OUTBOUND_FRAME_CAP);
    let (cancel_tx, cancel_rx) = watch::channel(());
    let queued_bytes = Arc::new(AtomicUsize::new(0));
    (
        FrameSender {
            tx,
            queued_bytes: Arc::clone(&queued_bytes),
            _cancel: cancel_tx,
        },
        FrameReceiver {
            rx,
            queued_bytes,
            cancel: cancel_rx,
        },
    )
}

/// Sends each queued frame to `sink` until the queue closes, the connection is
/// cancelled, or `sink` fails. A frame's bytes are released once it is sent.
pub(super) async fn write_frames(
    mut sink: impl AsyncFnMut(&[u8]) -> bool,
    mut frames_rx: FrameReceiver,
) {
    loop {
        let frame = tokio::select! {
            biased;
            frame = frames_rx.rx.recv() => frame,
            _ = frames_rx.cancel.changed() => break,
        };
        let Some(bytes) = frame else {
            break;
        };
        tokio::select! {
            biased;
            sent = sink(&bytes) => {
                if !sent {
                    break;
                }
                release(&frames_rx.queued_bytes, bytes.len());
            }
            _ = frames_rx.cancel.changed() => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_frame_cap_rejects_before_the_byte_cap() {
        let (frames_tx, _frames_rx) = frame_queue();
        for _ in 0..OUTBOUND_FRAME_CAP {
            assert!(frames_tx.send(b"n".to_vec()));
        }
        assert!(!frames_tx.send(b"n".to_vec()));
    }

    #[tokio::test]
    async fn the_byte_cap_rejects_until_a_frame_is_written() {
        let (frames_tx, mut frames_rx) = frame_queue();
        assert!(frames_tx.send(vec![b'a'; OUTBOUND_BYTE_CAP + 8]));
        assert!(
            !frames_tx.send(b"n".to_vec()),
            "a second frame was queued past the byte cap"
        );
        let bytes = frames_rx.rx.recv().await.expect("frame");
        assert_eq!(bytes.len(), OUTBOUND_BYTE_CAP + 8);
        assert!(
            !frames_tx.send(b"n".to_vec()),
            "a taken frame was released before it was written"
        );
        release(&frames_rx.queued_bytes, bytes.len());
        assert!(frames_tx.send(b"n".to_vec()));
    }
}
