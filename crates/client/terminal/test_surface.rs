//! The surface play's tests drive in place of the game window.

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use super::{Surface, Terminal};
use crate::protocol::ServerMessage;

/// The surface play's tests drive: it has a place panel, takes each line from a channel and
/// sends each call play makes on it, in order.
pub(super) struct TestSurface {
    lines: UnboundedReceiver<String>,
    calls: UnboundedSender<SurfaceCall>,
}

/// A call play made on the test surface, with the message it passed.
#[derive(Debug)]
pub(crate) enum SurfaceCall {
    Show(ServerMessage),
    FillPlacePanel(ServerMessage),
}

impl Terminal {
    /// The test surface: sends each line `lines` gives, and each call play makes to `calls`.
    pub(crate) fn test(
        lines: UnboundedReceiver<String>,
        calls: UnboundedSender<SurfaceCall>,
    ) -> Self {
        Self {
            surface: Surface::Test(TestSurface { lines, calls }),
            last_seq: 0,
        }
    }
}

impl TestSurface {
    pub(super) fn send(&self, call: SurfaceCall) {
        (self.calls.send(call)).expect("the test stopped reading the surface");
    }

    pub(super) async fn next_line(&mut self) -> Option<String> {
        self.lines.recv().await
    }
}
