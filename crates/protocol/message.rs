//! The one envelope every seat sends and receives, its bodies, and the command table.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{
    Audience, ChangeView, CommandView, CommitBody, HostStatus, NoteView, Outcome, PlaceView,
    Refusal, Speaker, View,
};

/// The wire version the hello announces.
pub const VERSION: u32 = 3;

/// One message on a seat's stream. `id` is chosen by the client and comes back as
/// `reply_to` on the reply. `seq` is set by the server, from one counter per host, on
/// everything it sends but the hello. The body is nested, so its fields never clash with
/// these.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Message<Body> {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
    pub body: Body,
}

/// Sent from a client to the host.
pub type ClientMessage = Message<ClientBody>; // repeats module: re-exported at the crate root

/// Sent from the host to a client.
pub type ServerMessage = Message<ServerBody>; // repeats module: re-exported at the crate root

/// One command, named by `kind`, and its arguments. Each command reads `args` as its own
/// argument type.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ClientBody {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub args: Map<String, Value>,
}

impl ClientMessage {
    /// A client message with `id` and `command`. Panics when `args` is not a JSON object.
    pub fn new(id: &str, command: &str, args: Value) -> Self {
        let Value::Object(args) = args else {
            panic!("{command} arguments are not an object");
        };
        Self {
            id: Some(id.to_string()),
            seq: None,
            reply_to: None,
            body: ClientBody {
                kind: command.to_string(),
                args,
            },
        }
    }
}

/// What the host sends. A client ignores a kind or a field it does not know.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ServerBody {
    /// The first message on a connection, and the only one with no `seq`. On a player seat,
    /// `open` lists the PCs nobody holds, `taken` the PCs held right now, and `pc_names`
    /// each one's name by id.
    Hello {
        version: u32,
        commands: Vec<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        open: Vec<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        taken: Vec<String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        pc_names: BTreeMap<String, String>,
    },
    /// The reply to `claim`. `token` takes this PC back after a dropped connection.
    /// `round_index` and `world_time` are the world's round and tick. A new claim carries
    /// `scrollback`, the speech this PC heard before, oldest first. A claim by token carries
    /// `missed_messages`, the messages sent to this PC after the `last_seq` it named, and
    /// `is_missed_truncated` when more than `MISSED_MESSAGE_CAP` messages were missed.
    Claimed {
        #[serde(rename = "actor")]
        actor_id: String,
        token: String,
        #[serde(default)]
        round_index: u64,
        #[serde(default)]
        world_time: u64,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        scrollback: Vec<Speech>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        missed_messages: Vec<ServerMessage>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        is_missed_truncated: bool,
    },
    Released,
    /// Sent to the GM whenever a claim starts or ends.
    Presence {
        actors_present: Vec<String>,
    },
    Refused(Refusal),
    Queued {
        intent: String,
        #[serde(rename = "actor")]
        actor_id: String,
        #[serde(rename = "move")]
        move_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<String>,
    },
    Say(Speech),
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
    Status(Box<HostStatus>),
    View(Box<View>),
    Resume {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        notes: Vec<NoteView>,
    },
    /// One narrate line, already the text to print.
    Lines {
        text: String,
    },
    /// The changes one round showed `viewer`, in the order they happened.
    Changes {
        viewer: String,
        changes: Vec<ChangeView>,
    },
    /// The reply to `help`: the commands this player can use now.
    Help {
        commands: Vec<CommandView>,
    },
    /// The reply to `map`: each place this player visited.
    Map {
        places: Vec<PlaceView>,
    },
}

/// One line of speech. The GM's copy lists its `recipient_ids`; a player's copy does not.
/// `from_name` is the speaking actor's name, left out when the GM speaks as itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Speech {
    pub from: Speaker,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub from_name: String,
    pub to: Audience,
    /// Who actually heard it: the ids of the seated players' actors that `to` reached when it
    /// was said, plus a seated speaker. `to` is who the speaker addressed; this is who was
    /// there, kept because seats and places change later. Empty on a player's copy.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recipient_ids: Vec<String>,
    pub text: String,
}

/// Which seat may send a command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seat {
    Player,
    Gm,
}

/// One built-in command, by the `kind` a client message sends. A player seat also has the
/// pack's verbs as commands.
pub struct Command {
    pub id: &'static str,
    pub seat: Seat,
}

impl Command {
    const fn new(id: &'static str, seat: Seat) -> Self {
        Self { id, seat }
    }
}

/// Every built-in command, in hello order.
pub const COMMANDS: &[Command] = &[
    Command::new("claim", Seat::Player),
    Command::new("release", Seat::Player),
    Command::new("say", Seat::Player),
    Command::new("look", Seat::Player),
    Command::new("help", Seat::Player),
    Command::new("map", Seat::Player),
    Command::new("cancel", Seat::Player),
    Command::new("status", Seat::Gm),
    Command::new("perceive", Seat::Gm),
    Command::new("queue", Seat::Gm),
    Command::new("say", Seat::Gm),
    Command::new("commit", Seat::Gm),
    Command::new("release", Seat::Gm),
];

/// Whether `kind` is a built-in command for `seat`.
pub fn is_built_in(seat: Seat, kind: &str) -> bool {
    COMMANDS
        .iter()
        .any(|command| command.seat == seat && command.id == kind)
}

/// `claim` arguments: a free PC, or the token of a claim this player held before and the
/// last `seq` it read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(untagged, deny_unknown_fields)]
pub enum ClaimArgs {
    Actor {
        #[serde(rename = "actor")]
        actor_id: String,
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
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct MoveArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

/// The arguments of a command that takes none.
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

/// The JSON Schema of a client message, published as `schema/client.json`. Its `$defs`
/// also hold each command's `args`, named after the type that reads them.
#[cfg(feature = "schema")]
pub fn generate_client_schema() -> schemars::Schema {
    let mut generator = schemars::SchemaGenerator::default();
    generator.subschema_for::<ClaimArgs>();
    generator.subschema_for::<MoveArgs>();
    generator.subschema_for::<NoArgs>();
    generator.subschema_for::<crate::SayArgs>();
    generator.subschema_for::<crate::PerceiveArgs>();
    generator.subschema_for::<crate::QueueArgs>();
    generator.subschema_for::<crate::CommitArgs>();
    generator.into_root_schema_for::<ClientMessage>()
}

/// The JSON Schema of a server message, published as `schema/server.json`.
#[cfg(feature = "schema")]
pub fn generate_server_schema() -> schemars::Schema {
    schemars::SchemaGenerator::default().into_root_schema_for::<ServerMessage>()
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
    fn a_client_body_is_a_kind_and_its_args() {
        let message: ClientMessage =
            serde_json::from_str(r#"{"id":"c8","body":{"kind":"travel","args":{"target":"x"}}}"#)
                .unwrap();
        assert_eq!(message.body.kind, "travel");
        let args: MoveArgs = serde_json::from_value(Value::Object(message.body.args)).unwrap();
        assert_eq!(args.target.as_deref(), Some("x"));
        let bare: ClientMessage = serde_json::from_str(r#"{"body":{"kind":"wait"}}"#).unwrap();
        assert!(bare.body.args.is_empty());
        assert!(serde_json::from_str::<ClientMessage>(r#"{"body":{"kind":"w","x":1}}"#).is_err());
    }

    #[test]
    fn a_claim_names_an_actor_or_a_token() {
        let read = |text: &str| serde_json::from_str::<ClaimArgs>(text);
        let actor = ClaimArgs::Actor {
            actor_id: "rowan".into(),
        };
        assert_eq!(read(r#"{"actor":"rowan"}"#).unwrap(), actor);
        let token = ClaimArgs::Token {
            token: "t1".into(),
            last_seq: 4,
        };
        assert_eq!(read(r#"{"token":"t1","last_seq":4}"#).unwrap(), token);
        assert!(read(r#"{"actor":"rowan","token":"t1"}"#).is_err());
        assert!(read(r#"{"actor":"rowan","last_seq":4}"#).is_err());
        assert!(read("{}").is_err());
    }

    /// Writes this build's schema to a temp dir when the published one differs.
    #[cfg(feature = "schema")]
    #[test]
    fn the_published_schema_matches_this_build() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("schema");
        for (file, schema) in [
            ("client.json", generate_client_schema()),
            ("server.json", generate_server_schema()),
        ] {
            let published = std::fs::read_to_string(dir.join(file)).unwrap_or_default();
            let published: Option<Value> = serde_json::from_str(&published).ok();
            if published.as_ref() == Some(schema.as_value()) {
                continue;
            }
            let schema_dir =
                std::env::temp_dir().join(format!("spookery-schema-{}", std::process::id()));
            std::fs::create_dir_all(&schema_dir).unwrap();
            let schema_path = schema_dir.join(file);
            let text = serde_json::to_string_pretty(&schema).unwrap() + "\n";
            std::fs::write(&schema_path, text).unwrap();
            panic!(
                "schema/{file} is not this build's; copy {} over it",
                schema_path.display()
            );
        }
    }

    #[test]
    fn a_hello_has_no_seq() {
        let hello = ServerMessage {
            id: None,
            seq: None,
            reply_to: None,
            body: ServerBody::Hello {
                version: VERSION,
                commands: vec!["claim".into()],
                open: vec!["rowan".into()],
                taken: vec!["wren".into()],
                pc_names: BTreeMap::new(),
            },
        };
        assert_eq!(
            hello.to_text(),
            r#"{"body":{"kind":"hello","version":3,"commands":["claim"],"open":["rowan"],"taken":["wren"]}}"#
        );
        let released = ServerMessage {
            body: ServerBody::Released,
            ..hello
        };
        assert_eq!(released.to_text(), r#"{"body":{"kind":"released"}}"#);
    }
}
