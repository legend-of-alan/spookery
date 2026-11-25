# Concept map

Every concept in the code has one word, and every word names one concept. This page lists those words, the synonyms that are banned for each, and the renames that bring today's code in line. [CONTRIBUTING.md](../CONTRIBUTING.md) Naming rule 19 points here, and CI checks new names against it.

The [glossary](README.md#glossary) stays the list of terms for players and pack authors. This page is for naming code. It states the canonical word for a glossary term and never gives it a second definition.

## How to use this page

1. Before you name a type, field, function, module, file or key, find its concept below and use that word.
2. A concept with no row gets a row in the same PR that names it.
3. Where today's code still says an old word, a row under [Renames still to do](#renames-still-to-do) says what replaces it. Do not copy the old word into new code. CI fails a new name that holds a banned word.

## Rules behind the words

- **Fewer words, one concept each.** Reuse a word the codebase already has before you coin one.
- **One word may cover the same action in different contexts.** Commit covers a round, a record, a save and a wire write, because it is one action. One word may not cover two different actions.
- **An ordinary word is fine when this page defines it.** The banned synonyms for that meaning are what keep it unambiguous. One thing in a list, or a value in a key-value pair, is an entry, and item means only a pack item.
- **Metaphors are banned.** Never seat and never table, in code, docs or player text. Mail words are banned too: mailbox, inbox, outbox, envelope, deliver and delivery. Recipient is a plain word and is allowed. Ledger is a metaphor that computing has absorbed, and it is allowed.
- **Names are nouns that say what the thing is.** Vague words such as helper, util, thing and stuff name nothing.
- **Question words** (why, what, when, where, who, how, which) are allowed in the pack and in the Rust that mirrors the pack at its edge. Pack keys such as `when:` keep them. The serde types that parse those keys keep them too, today the types in crates/engine/canon/rule_list/parse.rs. Past that edge, in engine types, fields, functions, locals and wire names, they are banned. Each has a plain noun: why is reason, what is the thing itself, when is time or trigger or whatever it concretely means, where is position or place, who is actor or source, how is the method's own noun. Rust `where` clauses are fine.
- **Names that belong to std, rmcp, ratatui, serde or tokio keep their spelling.**
- **Only code names are checked.** Prose, docs and doc comments are not.

## Concepts

| Concept | Word | Meaning | Never say |
| --- | --- | --- | --- |
| Game object | entity | A unique game object that has a group of components, as in Bevy. The Entity type is only its id. An actor, a place, a story and the world are entities, named by the id type entity::Id. | owner, carrier, holder, bearer, held |
| Data on an entity | component | A plain Rust type that stores data for an entity, with at most one value of each kind per entity. A meter, flag, item stack, position, wielded item and action are components. | entry (for this meaning) |
| Generic composition | parent | One thing made of or containing another in general. When the parts are components of an entity, say entity. | owner, holder, carrier, bearer |
| Where a rule lives | file | The pack file a rule lives in, the Rule.file field. | owner |
| What a rule is attached to | scope | Which pack definition a rule belongs to: Scope::Verb, Scope::Place(id), Scope::Item(id), Scope::Cast(id) or Scope::Story(id). It sets precedence, the rule id prefix and where the rule applies. | owner |
| A rule's kind | kind | Whether a rule fires on a verb, a habit or a change. The type is RuleKind, beside EffectKind and TargetKind. | when (past the pack edge), trigger |
| An actor | actor | An entity that acts. A cast file says what an actor starts as. Its control_type is pc or npc. A player is the human or program that claims a PC. | character, char, creature, person, agent |
| A place | place | A node on the map where actors stand. | room, location, area, site |
| Where an actor is | position | The place or path an actor is on. The pack key `location:` stays. | location |
| The effect that moves an actor | move | The effect that puts an actor in a place or on a path. | location (for the effect) |
| A line and column in a file | file_location | The spot in a file a diagnostic points at. The verb locate finds one. | line_column, spot, offset |
| A link between places | path, exit | The glossary defines both. A bare path in map, ecs, protocol and canon code is the map path. A file on disk ends in _path, _file or _dir. A diagnostic's key path is key_path. | edge, route, link, way, door |
| An item | item | Something an actor or place holds a count of. The definition is ItemDef. Item means only this. | thing, object, stuff, goods, gear, inventory |
| One thing in a list | entry | One thing in a list, or a value in a key-value pair. | item (for this meaning) |
| Meter, flag, modifier | meter, flag, modifier | The pack reference defines them. | stat, counter, gauge, attribute, fact, buff |
| What a player plays | verb | A word a rule names that a player can type, with a target. | move (for this meaning), act |
| One queued proposal | intent | An actor, a verb and a target, queued for the next round. | request, proposal, order |
| What a client asks the host to do | command | One wire message kind a client sends: say, queue, commit, plus the verbs. | operation, order, call |
| A verb that takes time | action | A verb with ticks, and the per-window action allowance. The pack key is `counts_as_action`, and the component id is action. | act |
| What an action costs | ActionCost | What the start or the finish of an action costs, and whether it uses the actor's action. | phase |
| Effect and change | effect, change | An effect is a checked write an intent produces. A change is the record of a write after it lands. | mutation, modification, edit, update |
| A rule's pieces | rule, if, when, gate, Cond | Cond is one condition in an if. Context is what a rule's conditions, costs and effects read. | condition, inputs, guard (for a gate, guard is only for locks) |
| A refusal | refusal | A no from the engine or host to a player or GM at run time. Its text is Refusal.message. The wire word is refused. | reject, deny, rejection, fail, detail |
| A finding about a file | diagnostic | One line of `spook check` output: file, location, message, reason. | problem, fault, diag, issue, violation |
| Why something was refused or flagged | Reason | One type for the stable reason on a refusal or a diagnostic. It may hold a string. | code (as a type name) |
| A number or an enum | code | Code means a number or an enum, never a free string. | code for a string |
| Error or warning | Severity | Whether a diagnostic is an error or a warning. It is separate from Reason. | error (as a severity) |
| A Rust error | Error | The Err value of one module or crate. A player never sees it as is. | error (as a severity) |
| A dice roll in a rule | check | The pack key `check:`, the wire check message and rule::Check. | check for lint or verify |
| A pack or save content rule | lint | What a pack or save may contain. `spook check` runs lints. | validate, ensure, is_valid |
| A journal or save integrity test | verify | Whether the journal chain, bundles and checkpoints hold. `spook verify` runs it. | validate |
| A round's parts | stage | The fixed passes of a round, in order: advance, pick, apply, finish, triggers, notify, record. Step is not used bare. | phase, step, turn, beat |
| One cycle of the model GM | turn | One model call cycle, in the agent crate only. | turn in the engine, docs or protocol |
| One message on the wire | message | One JSON text sent between a client and the host: ClientMessage and ServerMessage. | frame (except frame_queue), packet, event |
| What a message carries | payload | What a message carries after its outer fields. | body, content, data |
| The host | host | The running program that serves a pack: the game port and the MCP endpoint. | server (except the crate name spookery-server and rmcp's names) |
| An open pack | session | engine::Session, one open pack. The rmcp one is mcp_session. | RawSession |
| Who is in a session | claim, connection, role | A claim is a hold on a role, player or GM. A connection is a hold on a network resource. A Role is Role::Pc(ActorId) or Role::Gm, so claiming a PC is one step. Both players and the GM claim. | seat, table |
| The rules in force | Rules | A list of Rule. | Table |
| Telling clients what changed | notify | The stage that decides who hears a line and sees a change. A recipient is who it goes to. | deliver, delivery, mailbox, inbox, outbox, envelope |
| The save | save, checkpoint, savepoint | save is save.yaml. A checkpoint is the save published at a round. A savepoint is an ecs undo marker. | snapshot, backup |
| A stored line | record, journal, tape | A record is one line of the journal or the tape. The journal is the hash-chained rounds. The tape is speech. | row, history |
| What a round has written so far | ledger | Everything a round writes down in order, before commit records it. | RoundDraft, RoundWrites |
| Speech the player sees | tape, transcript | The tape stores speech. The transcript is the client's display of it. | scrollback, history |
| Game time | tick | The unit. world_time is the save's tick count, and ticks is a length of time. | now, clock, minutes |
| Where an intent came from | source | Engine, Player or Gm. A file's text is FileText. | origin |
| A shape in canon | Doc, Yaml, Def | Doc is a whole file, Yaml is one row of it, and Def is the model. Issue #521 settled this. | Descriptor, desc, Spec |
| Receiving a message | handle, handler | The verb is handle and the noun is handler. A resource handle, such as JoinHandle, is a different meaning. | process, dispatch, on_*, receive_* (for this meaning) |

## Verbs

| Verb | Means | Never |
| --- | --- | --- |
| read | Bytes or text from disk or a socket. | load, collect, fetch |
| parse | Text into typed values, with serde. | load, decode |
| compile | Pack source in any format, YAML or JSON, into the engine's loaded form. It is not specific to YAML. | load, lower, convert |
| load | The whole route from disk to a runtime value for one thing, never one step of it. | load for a single step |
| emit | A typed value into stored text or bytes. | serialize (outside serde), dump |
| write | Bytes onto disk. | save, persist, flush |
| render | A value into text a client shows. | format (outside std), draw |
| build | Assemble an outgoing view or message from parts. | make, create, construct |
| collect | Only Iterator::collect. Walking a folder is list or walk. | collect_files |
| get | One value by key from a map or store, returning an Option. | fetch, lookup |
| find | A search across many by a predicate. | search, lookup |
| resolve | A word or name a player or pack wrote into an id or entity. | lookup, map |
| pick | Choose one among candidates. | select, choose |
| take | Remove a value and return it. Locking is lock_read and lock_write. | take for locking |
| locate | Find a file_location in text. | |
| start and finish | Anything that runs, ends on its own and has a cost or a time: an action, a round, a roll, a turn, a run. | begin, init, initialize, end, done, stop (for a normal end) |
| open and close | A resource with a handle: a file, a connection, a session, a pack. | start, init |
| claim and release | A hold on a role. | take, free |
| commit | Publish a round, a record, a save or a wire write. It is one action in several contexts. Checkpoint publishes the save at a round. | seal, settle |

## Same word, different concepts

These stay separate, with a sharp line between them.

| Word | The meanings | The line |
| --- | --- | --- |
| path | a map link, a file on disk, a key inside a file | A bare path is the map link. |
| check | a dice roll, a pack lint, a journal integrity test | check, lint and verify. |
| session | engine::Session, an MCP client's link | The MCP one is mcp_session. |
| token | an intent's target token, a claim token, a rate limit token | target_token, claim_token. TokenBucket stays. |
| key | a serialized key, a change key, KeyCode in the client | In engine and protocol code a key is a serialized or change key. |
| code | a refusal code, a key code, a diagnostic code | Code means a number or an enum only. |
| source | where an intent came from, a file's text | The intent one is source. The file one is FileText. |
| report | four report types | Each name says what it reports: WriteReport, VerifyReport, ReplayReport, MoveReport. |
| outcome | the check band, the commit result | The band is Band. The commit one is CommitResult. |
| record | a journal or tape line | Not a general word. |
| message | a wire message, a refusal's text, a line of a model turn | Each is text sent to someone, so one word covers them. |

## Entity and component

These are Bevy's definitions, and Spookery uses them as they are.

- An entity is a unique game object that is given a group of components. The Entity type is only its id. See the [Bevy ECS introduction](https://bevyengine.org/learn/quick-start/getting-started/ecs/) and the [Entity docs](https://docs.rs/bevy_ecs/0.20.1/bevy_ecs/entity/struct.Entity.html).
- A component is a plain Rust type that stores data for an entity, with at most one value of each kind per entity. See the [Component docs](https://docs.rs/bevy_ecs/0.20.1/bevy_ecs/component/trait.Component.html).
- Stores hold entities, which have components. There is no entry under an entity.

## Renames still to do

Each row is one later PR, one concept per PR. Until a rename lands, the old name stays in the code and the check lets it stand. A rename that changes a pack key or the save format bumps RULES_REV. A rename that changes a wire key or type bumps the protocol version.

| Today | Becomes | Bump |
| --- | --- | --- |
| rule::Owner | Scope | |
| rule::When and Rule.when, and when_of | RuleKind, kind and kind_of. The pack key `when:` and its parse types stay. | |
| rules::Table | Rules | |
| protocol::Seat and Seats.gm | Role, with the GM claiming Role::Gm too | protocol |
| the file crates/server/seat.rs | claim.rs, with the connection code moved to its own module | |
| the file docs/seat-protocol.md | docs/client-protocol.md | |
| TableFull, TableEmpty, SeatTaken | SessionFull, SessionEmpty, ActorTaken. The player lines become Session full, Session empty, The host refused the write, The host did not understand that. | protocol |
| Act, `counts_as_act`, the component id act | Action, `counts_as_action`, action | RULES_REV |
| Phase | ActionCost | |
| EffectKind::Location | EffectKind::Move | |
| LineColumn | FileLocation | |
| the stage deliver | notify | |
| wire key body, and ServerBody and the other Server types | payload, and the Host types. Issue #785 rides in the same bump. | protocol |
| move_id, QueuedMove, MoveReport, UNKNOWN_MOVE | verb names | protocol |
| Refusal.detail | Refusal.message | protocol |
| the protocol Code, Reason and the engine Code | one Reason that may hold a string, and Severity for error or warning. Code::Other(String) maps onto Reason. | protocol |
| the journal Step type and extra_steps | dropped. The round line already holds effects, the rule ids that fired, and one trigger-cap flag. | RULES_REV |
| seal_round, seal_migrate, seal_checkpoint, journal::seal, sealed_chain | commit names, such as commit_round | |
| begin_round, Dice::begin and other begin names | start names | |
| why_exit_lacks_flag, why_meter_not_carried, a local why | reason names | |
| the locals who and whose | actor | |
| the name in a CantAfford line, a test's wait label | name and event | |
| the file-text source in canon | FileText | |
| Descriptor, desc and RuleSpec | Def, in the canon plan | |
| take_read and take_write | lock_read and lock_write | |
| a rule's owner and the owner of a meter in the docs | file and entity | |
| condition in the docs | Cond | |

## The check

CI reads the word lists below and checks the identifiers on the lines a PR adds to Rust files. It skips comments, string literals and the pack-edge parse file named above. It fails on a new identifier that holds a banned word, and prints the file, the line, the word and what to use. An identifier already in the code before the PR stands, so the check never blocks on a name that has not been renamed yet, and a rename PR clears it. A name in the keep line stands too.

The words CI checks are lowercase and match the parts of a name, split on snake_case and CamelCase. A word that depends on context is not in the list, such as item, move, process, dispatch and the on_ and receive_ prefixes.

```naming-words
seat: never, a metaphor
table: never, a metaphor
mailbox: never, a metaphor
inbox: never, a metaphor
outbox: never, a metaphor
envelope: never, a metaphor
deliver: notify
delivery: notify
body: payload
server: host
act: action
begin: start
seal: commit
owner: entity, parent or scope
holder: entity or parent
carrier: entity or parent
bearer: entity or parent
why: reason
what: the thing itself
when: time, trigger or kind
where: position or place
who: actor or source
how: the method's own noun
which: the thing itself
keep: ServerHandler ServerCapabilities ServerInfo bearer_auth
```
