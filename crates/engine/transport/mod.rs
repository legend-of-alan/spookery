//! The protocol router: client messages in, addressed server messages out, with no sockets. A
//! network adapter or an in-process host drives a `Router` one request at a time.

mod gm;
mod message_body;
mod participants;
mod router;

pub use participants::ConnId;
pub use router::{Outbox, Rejection, Router};
