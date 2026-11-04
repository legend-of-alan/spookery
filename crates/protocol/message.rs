//! The one envelope every seat sends and receives, its bodies, and the word table.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{ClockView, CommitBody, NoteView, Outcome, Refusal, Speaker, TableStatus, View};

/// The wire version the hello announces.
pub const VERSION: u32 = 1;

/// One message on a seat's stream. `id` is chosen by the client and comes back as
/// `reply_to` on the reply. `seq` is set by the server, from one counter per table, on
/// everything it sends but the hello. The body is nested, so its fields never clash with
/// these.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message<Body> {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
    pub body: Body,
}

/// Sent from a client to the table.
pub type ClientMessage = Message<ClientBody>;

/// Sent from the table to a client.
pub type ServerMessage = Message<ServerBody>;

/// One word and its arguments. Each word reads `args` as its own argument type.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientBody {
    pub word: String,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub args: Map<String, Value>,
}

impl ClientMessage {
    /// A client message with `id` and `word`. Panics when `args` is not a JSON object.
    pub fn new(id: &str, word: &str, args: Value) -> Self {
        let Value::Object(args) = args else {
            panic!("{word} arguments are not an object");
        };
        Self {
            id: Some(id.to_string()),
            seq: None,
            reply_to: None,
            body: ClientBody {
                word: word.to_string(),
                args,
            },
        }
    }
}

/// What the table sends. A client ignores a kind or a field it does not know.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ServerBody {
    /// The first message on a connection, and the only one with no `seq`. `open` lists the
    /// PCs a player seat may claim.
    Hello {
        version: u32,
        words: Vec<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        open: Vec<String>,
    },
    /// The reply to `claim`. `token` takes this PC back after a dropped connection. A new
    /// claim carries `scrollback`, the says this PC heard before, oldest first. A claim by
    /// token carries `missed`, the messages sent to this PC after the `last_seq` it named, and
    /// `gap` when more than `MISSED_MESSAGE_CAP` messages were missed.
    Claimed {
        actor: String,
        token: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        scrollback: Vec<Say>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        missed: Vec<ServerMessage>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        gap: bool,
    },
    Released,
    /// Sent to the GM whenever a claim starts or ends.
    Presence {
        actors_present: Vec<String>,
    },
    Refused(Refusal),
    Queued {
        intent: String,
        actor: String,
        #[serde(rename = "move")]
        move_name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<String>,
    },
    Say(Say),
    /// One committed round. Every seat gets it.
    Round {
        round_index: u64,
        world_time: u64,
    },
    /// A player's own check. No faces: the GM reads those in `committed`.
    Check {
        outcome: Outcome,
    },
    /// The GM's reply to `commit`, after its rounds.
    Committed(CommitBody),
    /// The GM's reply to `status`.
    Status(Box<TableStatus>),
    View(Box<View>),
    Resume {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        notes: Vec<NoteView>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        clocks: Vec<ClockView>,
    },
    /// Help, look, and map. The text is already the lines to print.
    Lines {
        text: String,
    },
}

/// One say. The GM's copy lists its `recipients`; a player's copy does not.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Say {
    pub from: Speaker,
    pub address: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recipients: Vec<String>,
    pub text: String,
}

/// Which seat may send a word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seat {
    Player,
    Gm,
}

/// One built-in word. A player seat also has the pack's verbs as words.
pub struct Word {
    pub name: &'static str,
    pub seat: Seat,
}

impl Word {
    const fn new(name: &'static str, seat: Seat) -> Self {
        Self { name, seat }
    }
}

/// Every built-in word, in hello order.
pub const WORDS: &[Word] = &[
    Word::new("claim", Seat::Player),
    Word::new("release", Seat::Player),
    Word::new("say", Seat::Player),
    Word::new("look", Seat::Player),
    Word::new("help", Seat::Player),
    Word::new("map", Seat::Player),
    Word::new("status", Seat::Gm),
    Word::new("perceive", Seat::Gm),
    Word::new("queue", Seat::Gm),
    Word::new("say", Seat::Gm),
    Word::new("commit", Seat::Gm),
    Word::new("release", Seat::Gm),
];

/// `claim` arguments: a free PC, or the token of a claim this player held before and the
/// last `seq` it read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum ClaimArgs {
    Actor {
        actor: String,
    },
    Token {
        token: String,
        /// The last `seq` the player read. A missing `last_seq` means 0, so the reply
        /// catches up on everything the claim still keeps.
        #[serde(default)]
        last_seq: u64,
    },
}

/// A pack verb's arguments, sent by a player seat.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MoveArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

/// The arguments of a word that takes none.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct NoArgs {}

impl ServerMessage {
    /// One JSON text, the body nested.
    pub fn to_text(&self) -> String {
        serde_json::to_string(self).expect("server message")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Code, Reason};

    #[test]
    fn a_refused_reply_nests_the_code_in_its_body() {
        let reply = ServerMessage {
            id: None,
            seq: Some(1),
            reply_to: Some("c1".into()),
            body: ServerBody::Refused(Reason::Schema.into()),
        };
        let text = reply.to_text();
        assert_eq!(
            text,
            r#"{"seq":1,"reply_to":"c1","body":{"kind":"refused","code":"SCHEMA"}}"#
        );
        let back: ServerMessage = serde_json::from_str(&text).unwrap();
        assert_eq!(back, reply);
        let unknown: ServerMessage =
            serde_json::from_str(r#"{"seq":2,"body":{"kind":"refused","code":"NEW"},"x":1}"#)
                .unwrap();
        assert!(matches!(
            unknown.body,
            ServerBody::Refused(Refusal {
                code: Code::Other(_),
                ..
            })
        ));
    }

    #[test]
    fn a_client_body_is_a_word_and_its_args() {
        let message: ClientMessage =
            serde_json::from_str(r#"{"id":"c8","body":{"word":"travel","args":{"target":"x"}}}"#)
                .unwrap();
        assert_eq!(message.body.word, "travel");
        let args: MoveArgs = serde_json::from_value(Value::Object(message.body.args)).unwrap();
        assert_eq!(args.target.as_deref(), Some("x"));
        let bare: ClientMessage = serde_json::from_str(r#"{"body":{"word":"wait"}}"#).unwrap();
        assert!(bare.body.args.is_empty());
        assert!(serde_json::from_str::<ClientMessage>(r#"{"body":{"word":"w","x":1}}"#).is_err());
    }

    #[test]
    fn a_claim_names_an_actor_or_a_token() {
        let read = |text: &str| serde_json::from_str::<ClaimArgs>(text);
        let actor = ClaimArgs::Actor {
            actor: "pc_rowan".into(),
        };
        assert_eq!(read(r#"{"actor":"pc_rowan"}"#).unwrap(), actor);
        let token = ClaimArgs::Token {
            token: "t1".into(),
            last_seq: 4,
        };
        assert_eq!(read(r#"{"token":"t1","last_seq":4}"#).unwrap(), token);
        assert!(read(r#"{"actor":"pc_rowan","token":"t1"}"#).is_err());
        assert!(read(r#"{"actor":"pc_rowan","last_seq":4}"#).is_err());
        assert!(read("{}").is_err());
    }

    #[test]
    fn a_hello_has_no_seq() {
        let hello = ServerMessage {
            id: None,
            seq: None,
            reply_to: None,
            body: ServerBody::Hello {
                version: VERSION,
                words: vec!["claim".into()],
                open: vec!["pc_rowan".into()],
            },
        };
        assert_eq!(
            hello.to_text(),
            r#"{"body":{"kind":"hello","version":1,"words":["claim"],"open":["pc_rowan"]}}"#
        );
        let released = ServerMessage {
            body: ServerBody::Released,
            ..hello
        };
        assert_eq!(released.to_text(), r#"{"body":{"kind":"released"}}"#);
    }
}
