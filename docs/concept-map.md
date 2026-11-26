# Concept map

A short guide to the word each concept gets, and the synonyms to avoid for it. Names should be obvious from the code, so this page lists only the words that have a competing synonym. A different word is fine when it names a genuinely different concept. [CONTRIBUTING.md](../CONTRIBUTING.md) Naming rule 19 points here. The [glossary](README.md#glossary) still defines game terms for players and pack authors.

## Rules

- One word per action, wherever it happens. Commit covers a round, a record, a save and a wire write. One word never covers two different actions.
- Reuse a word the code has before coining one. Add a row here only when a concept has two names in use.
- Avoid metaphors.
- Question words (why, what, when, where, who, how, which) are fine in the pack and in the serde types that parse it at the edge, which is only crates/engine/canon/rule_list/parse.rs. Past the edge, prefer a plain noun. Rust `where` clauses are fine.
- Names that belong to std, rmcp, ratatui, serde or tokio keep their spelling.

## Words

| Use | Never |
| --- | --- |
| entity: a game object with components, as in Bevy. component: plain data on an entity. | owner, carrier, holder, bearer, held |
| parent: any other composition | owner, holder, carrier, bearer |
| file: the pack file a rule lives in | owner, rel |
| Scope: what a rule is attached to (verb, place, item, cast, story) | owner |
| RuleKind and kind: whether a rule fires on a verb, a habit or a change | when (past the pack edge) |
| actor | character, creature, person |
| place | room, location, site |
| position: where an actor is | location |
| move: the effect that puts an actor in a place | location (for the effect) |
| file_location: a line and column in a file | line_column, spot |
| item: a pack item only | thing, stuff, gear, inventory |
| entry: one thing in a list or map | item |
| meter, flag, modifier | stat, counter, gauge, attribute, buff |
| verb: what a player plays | move, act |
| intent: a queued verb | proposal, order |
| command: what a client asks the host to do | operation, order |
| action, ActionCost | act, phase |
| effect, change | mutation, modification, update |
| Cond | condition |
| refusal, with Refusal.message | reject, deny, fail, detail |
| diagnostic | problem, fault, diag, diags |
| Reason: why something was refused or flagged. code: a number or an enum, never a string. | code as a string |
| Severity: error or warning | error (as a severity) |
| check: a dice roll. lint: a pack rule. verify: a journal test. | validate |
| stage | phase, beat |
| message: one wire text. payload: what it carries. | packet, body |
| host | server |
| claim: a hold on a role. connection: a hold on a network resource. Role: Pc(ActorId) or Gm. | seat, table |
| Rules: the list of rules | Table |
| notify | deliver, delivery |
| save, checkpoint | snapshot |
| tick | clock |
| source: where an intent came from | origin |
| handle, handler: receiving a message | on_, receive_ |
| start, finish | begin, init |
| commit | seal, settle |
| read, load, emit, write, render | convert, dump |
| parse: pack text (YAML or JSON) into its typed source form | convert |
| compile: the typed source form into the engine's loaded form | convert, build |
| build: assemble any other value, as a builder does | convert, compile |
| Yaml: the typed YAML of one pack file. A part of it ends in Yaml, as in CondYaml. | Doc |
| Def: what the pack says about one thing | Descriptor |
| get: one value by key. find: a search by predicate. resolve: a name into an id. | lookup, search |

A bare path is a map link, and a file on disk ends in _path, _file or _dir. A key path inside a pack file, such as rules[0].when, is key_path. The MCP session is mcp_session. A token says which, such as claim_token.

Entity and component keep Bevy's meaning: [Entity](https://docs.rs/bevy_ecs/0.20.1/bevy_ecs/entity/struct.Entity.html) is only an id, and [Component](https://docs.rs/bevy_ecs/0.20.1/bevy_ecs/component/trait.Component.html) is plain data, one value of each kind per entity.
