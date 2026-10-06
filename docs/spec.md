# Spookery referee spec

This file is the only normative spec. A rule it does not state is unspecified.

Spec text is CC BY 4.0. Runtime code under `/src` is AGPL-3.0-or-later.

This spec records one writer, the crate boundary, the threat list, loopback and one seat, and the rule that a host monitor is not shard behavior.

## Referee rules

These fourteen rules are the referee, in this order. Later sections are the detail. A sentence in another document does not add a fifteenth rule.

1. **Import boundary.** `spookery-kernel` and `spookery-tick` do not link a runtime, a database, or a model SDK.
2. **`!Send` `Txn`.** One `std::thread` owns `Now`. The tick is not spawned on a runtime.
3. **Atomic commit.** `now.yaml` is renamed last. A crash cannot apply a partial tick.
4. **Closed effect, one apply.** Ten effect classes. `apply_effect` is the only mutator of world numbers.
5. **Firewall.** `firewall::allows` is the visibility test. A hidden fact does not leak through place or cast prose.
6. **Deterministic tick.** Selector bindings freeze before the apply loop. Replay re-expands the recorded intents and calls `resolve_check` again from the recorded seed.
7. **Canon versus `Now`.** Canon is frozen. `Now` is the live snapshot.
8. **Action-row loader.** A moves file is names and rows. Selectors are a closed enum. A new verb is a new row in YAML.
9. **Seeded checks.** `die_face` is Blake2s. One copy, in `check.rs`. `difficulty` applies only when the row sets `check` and no opposing total applies.
10. **Eligibility and pause.** Both run before an agenda intent exists. An ineligible NPC produces no agenda intent.
11. **Failure tokens.** Each token has one meaning. `NOT_OWNER` and `LIVE_ONLY` are not tokens.
12. **Speech versus queue.** Speech appends to the tape. It is not a row, it does not enter the intent queue, and it does not tick.
13. **Seat law and profile split.** One player seat and one GM seat. Three static MCP profiles. `npc_act` does not call a model.
14. **Rounds.** With no GM, the runtime queues and commits one live tick. With a GM seated, the line waits for `commit_turn`.

## One writer

The shard thread is the only writer of `Now` and of `Seats`. One `std::thread` owns `Now`. `Txn` stays `!Send`, with `PhantomData<Rc<()>>` as in `src/kernel/shard.rs`. No `tokio::spawn` of a tick.

The queue is a `Vec` of intents on the shard thread. It is ephemeral. A crash drops every uncommitted line. A crash cannot apply a partial tick. The queue is not a field of `now.yaml`. It is not a side file that recovery has to remember.

`Txn::begin` clones `Now` once into `staging`. That is the only clone. `commit` moves `staging` into `Shard.now` after `write_commit` returns. It does not clone again.

`expand` takes `&Canon`, `&Now` (the staging borrow), and `&Intent`. It does not hold `Shard`. `apply_effect` takes `&mut Staging`. `Staging` borrows `Txn`'s staging, the typed key set, and the active intent, so it is `!Send`. The tick does not hold `&Shard` and `&mut Txn` in a way that aliases `Now`. Checks read `current` off the pre-loop `frozen` snapshot, the same `Now` expand uses. They do not read live staging. The item precondition reads the actor's inventory before queue. `stage_tick` receives `&mut Txn` and the queue snapshot: the `Vec` of intents is moved out for the tick and put back only if the tick is abandoned. `Shard` fields are private.

`append_session` takes the session lock and never a `Txn`. `commit_presence` takes the shard lock and never a `Txn`.

`Seats` is the set of live socket senders plus a process-local session counter. It is not a second presence document. The durable source of who is at the table is `Now.presence`, and that struct is only `humans` and `actors_present`. Session ids are not in it. The shard thread is the only writer of both `Seats` and `Now`. A player hello or drop that changes `humans` or `actors_present` calls `commit_presence` before the handler returns. A GM hello does not change those fields and does not write `now.yaml`.

Invariant, checked by a debug assert on the shard thread: `seats.player.is_some()` iff `presence.actors_present` contains the seated PC and `presence.humans` is 1. The assert is debug-only. It is not a library `unwrap`. Startup repair is `clear_crashed`. `listen` binds, then the shard thread opens the shard and, if `humans != 0` or `actors_present` is non-empty while `Seats` is empty, `commit_presence` writes zeros.

## Crate boundary

Four crates. A one-crate collapse is not the layout. The bin and `spookery-mcp` are the only crates that link a runtime.

| Crate | May link | Must not link |
|---|---|---|
| `spookery-kernel` | `serde`, `serde-saphyr` `=1.3.0`, `serde_json`, `blake2`, `thiserror`, std | `tokio`, `rmcp`, `rmcp-macros`, `sqlx`, `postgres`, `redis`, `jsonschema`, any model SDK, `spookery-tick`, `spookery-mcp` |
| `spookery-tick` | `spookery-kernel` | `tokio`, `rmcp`, `serde_json` as a direct dependency (it uses `kernel::codec`), MCP, any model SDK |
| `spookery-mcp` | `spookery-kernel`, `rmcp`, `tokio` | `spookery-tick`, `rmcp-macros`, any model SDK |
| bin `spookery` | kernel, tick, mcp, `tokio`, `rmcp`, `socket2`, `serde_json` | Business rules that belong in the kernel. A second tick implementation. |

`serde_json` in the kernel is allowed. The import scan does not forbid it. The scan stays a source scan, not a `Cargo.toml` parser. `src/kernel` and `src/tick` must not contain the byte strings `tokio`, `rmcp`, `sqlx`, `postgres`, or `redis`. `src/` must not contain `langchain`, `openai`, `anthropic`, `ollama`, or `reqwest`. A comment in kernel or tick source that names a banned crate is still a failure. This spec is where that ban is written. There is no cargo feature named `llm`.

Tokio in the bin is the accept loop, keepalive, and scheduler sleep. Tokio in `spookery-mcp` is the stdio runtime for `--mcp`, which is list-only. The tick function runs on the shard thread.

`rulesets/` is an authoring template. The loader reads `pack/rules/` only.

## Effect envelope

`Effect` is one struct. The class is the payload. Rows emit the ten classes below.

```rust
pub struct Effect {
    pub class: EffectClass,
    pub as_viewer: ViewerId,
    pub intent_id: String,
}

pub enum EffectClass {
    Clock { clock_id: String, delta: i64 },
    Meter { actor_id: String, key: String, write: MeterWrite },
    Stock { place: String, sku: String, delta: i64 },
    Location { actor_id: String, from: String, to: String },
    Relationship { a: String, b: String, delta: i64 },
    Reputation { actor_id: String, faction: String, delta: i64 },
    Memory { actor_id: String, fact_ids: Vec<String>, tags: Vec<String>, text: Option<String> },
    Rumor { rumor_id: String, fact_ids: Vec<String>, from: String, to: String, heat: i64 },
    Flag { actor_id: String, key: String, value: bool },
    Message { message_id: String, to: Vec<String>, place: Option<String>, fact_ids: Vec<String>, text: String },
}
```

`ViewerId` is `Player` or `Npc`. There is no `Gm` variant. The JSON token is `player:{id}` or `npc:{id}`.

JSONL and MCP write one flat object per effect. The object is internally tagged and flattened. `class` is a string: `clock`, `meter`, `stock`, `location`, `relationship`, `reputation`, `memory`, `rumor`, `flag`, or `message`. `MeterWrite` is `Base(i64)`, `Offset(i64)`, or `Snap`. The wire writes `write` as `base`, `offset`, or `snap`, and writes `delta` only for `base` and `offset`. Payload fields are siblings of `as` and `intent_id`. They are not nested under `class`. An externally tagged object is not the codec. Key order is fixed: `class`, then `as`, then `intent_id`, then payload fields in the order declared on that `EffectClass` variant. A custom serializer writes that order. serde's default enum encoding does not. `now.yaml` byte-stability does not depend on this object. It is the tick JSONL shape only. Tests assert this order.

## Apply

`apply_effect` is the only mutator of world numbers. Arithmetic stays `checked_add`. A meter write clamps a sum that fits in `i64`. Stock refuses a negative result. Relationship and reputation deltas outside `-1..=1` are `OUT_OF_RANGE`. Memory text longer than 240 characters is `SCHEMA`. Memory and rumor facts the viewer may not know are `ILLEGAL_KNOWLEDGE`. Relationship and reputation keep the once-per-world-day `SKIPPED` no-op. The flag key `busy` is `SCHEMA` at load and is not a template. `busy` remains the eligibility bit on the actor.

The effect's viewer must be the active intent's actor. Failure is `VIEWER_MISMATCH`.

Clocks do not clamp. An unknown clock is `UNKNOWN_KEY`. A paused clock, or a clock that requires presence while `humans == 0`, is `PAUSED_CLOCK`. `value + delta` outside `[0, max]` is `OUT_OF_RANGE`. If the intent actor is not in `clock.owners`, the result is `UNKNOWN_KEY`. That actor has no such clock key.

Location: `from` must equal the live place, and `to` must be in the profile range and on the schedule graph. Otherwise `OUT_OF_RANGE`. An `actor_id` other than the intent actor is `ACTOR_MISMATCH`. A PC location while `humans == 0` is `TABLE_EMPTY`. An NPC with no tick profile is `NO_PROFILE`.

Stock: a missing profile is `NO_PROFILE`. A place outside range or off the schedule graph, an absolute delta above `stock_rate`, or a negative result, is `OUT_OF_RANGE`.

Meter: `write` is `base`, `offset`, or `snap`. `base` adds the delta to `base` and clamps that base into `[min, base_cap]`, and keeps `offset`. `offset` adds the delta to `offset` and keeps `base`. `snap` sets the next pair to `(base, 0)`. The live pair is assigned only when `base` is inside `[min, base_cap]` and `base + offset` fits in `i64`. A sum that fits and passes `max` is clamped when `current` is read. `current` is `clamp(base + offset, min, max)`. It is not stored. An add that does not fit in `i64`, or a next pair that fails that check, is `OUT_OF_RANGE` and the record is unchanged. An unknown meter id is `UNKNOWN_KEY`. A PC and an NPC take the same write.

## Rows and selectors

A moves file is still a name or a row. The schema version string stays `moves/v0`. New fields are optional. A misspelled field is rejected because unknown fields are denied.

A row has:

- `audience`: `player`, `agenda`, or `system`. Omitted audience is `SCHEMA`. A row is not implicitly player-facing.
- one token: `none`, `actor`, or `place`
- one subject: `speaker` or `named`
- `effects`: templates that run on both outcomes
- `success` and `failure`: templates that run on that outcome
- optional `check`: an id that must exist in `checks.yaml`, or load returns `SCHEMA`
- optional `difficulty`: `i64`, default 0, used only when `check` is set and no opposing total applies
- optional `item`: an actor inventory precondition

`classify` and the GM intent tool queue a row only when `audience` is `player`. An `agenda` or `system` name, including `advance_clock`, `restock`, and `travel_schedule`, is `NOT_IN_SPEC` and is not queued. Agenda selection queues a row only when `audience` is `agenda`. A cast agenda that names a `player` or `system` row is `SCHEMA` at load. No call site resolves a `system` row. Empty-table gates still apply to anything that does get into a tick.

Load rejects a duplicate name, an empty name, an empty `item`, the flag key `busy`, a row on `say` or `talk`, an unknown class, an unknown selector, a template whose keys do not match its class, and any unknown shape. `give`, `take`, `wait`, and `attempt` are not reserved names. `attack` is not reserved. `travel` is not special-cased in Rust. A travel row is whatever templates it has, and `apply_location` enforces the graph. The docks grit check is a `grit` row. `attempt grit 3` is not a special parser. The wire object `{"kind":"check","success":true|false}` does not change.

**Yield.** The key `effects` may be present and empty only when `success` and `failure` are absent. That row is a yield. Omitted `effects` together with omitted outcome lists, on a row that set `token` or `on`, is `SCHEMA`. A yield with no `check` expands to zero effects and is still an applied intent if it was allowed. A yield with a `check` rolls, emits the check notice, and writes no world key. Nothing matches on the name `wait` or `attempt`. `wait` is a yield row (`effects` present and empty). An NPC agenda that does not list `wait` does not emit it. Absence of a ready agenda entry means no kernel intent.

**Selectors.** A template field is either a literal or one selector. Selectors do not compose. There is no expression language. Unknown selector text is `SCHEMA` at load. Adding a verb is a new row in YAML. Adding a selector is a kernel change and a conformance test. The docks fixture does not need a selector outside this list.

```rust
pub enum Selector {
    Token,
    Here,
    SchedulePlace,
    ScheduleRestDelta,
    FromWork,
    FirstRateSku,
    Rate,
    FirstDrift,
    TowardTarget,
    FirstKnownRumor,
    FirstListener,
    KeepHeat,
    FirstDueNote,
    FirstOwnedClock,
}
```

`ready` and `resolve` match on this enum. They do not match on the row's name.

Several YAML keys are one walk. Mixing a selector from a walk with a literal for another field of the same walk is `SCHEMA`. `FirstRateSku`, `Rate`, and `Here` together fill one `Stock`. `FirstDrift` and `TowardTarget` together fill one `Relationship`. `FirstKnownRumor`, `FirstListener`, and `KeepHeat` together fill one `Rumor`. `FirstDueNote` fills one `Message`. `FromWork` replaces its template with the whole work-arm list. The nickname `first_short` is not a selector. The restock walk is not a search for a sku below a cap. There is no stock maximum in the docks profile.

| Selector | Inputs | Order | Fields written | Rule |
|---|---|---|---|---|
| `Here` | Actor's live place on staging `Now` | n/a | `Stock.place` | The place is `actor.place()`, the same place argument restock uses. |
| `FirstRateSku` | `profile.stock_rate[place]`, `now.places[place].stock` | `BTreeMap` key order of that place's rate map | `Stock.sku` | For each `(sku, rate)`, stock is the live quantity or 0. Take the first sku where `stock.checked_add(rate)` is `Some` and `>= 0`. If the add overflows, skip that sku. If none qualify, the walk fails and the agenda entry is not taken. |
| `Rate` | The rate on the sku `FirstRateSku` just chose | Same walk | `Stock.delta` | That sku's `rate` value. |
| `SchedulePlace` | `profile.schedule`, `world_hour(now.world_time)` | File order of `schedule`. The winning row is the first row where `hour_in` holds | `Location.to` | If `start < end` then `start <= hour < end`. If `start > end` then `hour >= start` or `hour < end`. If `start == end`, the hour is not in the row. Travel is emitted when the travel pause lane is running, `row.place != actor.place`, and `row.place` is in range. |
| `Token` | The intent token | n/a | `Location.to` when the token is `Place`; the named actor when the token is `Actor` | For player travel, `to` is the typed place. A `none` token where a template says `token` is `SCHEMA`. |
| Location `from` | Live place of `intent.actor_id` | n/a | `Location.from`, `Location.actor_id` | `from` is the actor's live place. `actor_id` is the intent actor. `to` is the place argument. A missing actor is `UNKNOWN_KEY`. `from` is not a YAML field. |
| `ScheduleRestDelta` | The winning row's `rest_delta` | Same winning row as `SchedulePlace` | One `Meter` for the speaker. The key is `rest`. The arm is the template's `base` or `offset`. | `None` means the selector does not resolve and the agenda entry is not taken. A template key other than `rest`, or `write: snap`, is `SCHEMA` at load. The number is not clamped at load. |
| `FromWork` | `profile.work` | File order of the work list | Replaces the marker with N `Meter` classes for the speaker, one per work entry. | A missing or empty work list resolves to zero classes. The intent can still apply as a yield. A `key` or `write` on the marker is `SCHEMA`. The walk does not filter to one key. |
| `FirstDrift` | Speaker's relationships, `now.world_time`, both actors' `busy` | Pairs collected, then sorted by the other actor id. First pair wins | `Relationship.a` is the speaker. `Relationship.b` is the other actor. | Speaker busy means no intent. Keep pairs where stance is not the target, `world_time` is at least 1440 minutes after `last_interaction_world_time`, the other actor exists, and the other is not busy. |
| `TowardTarget` | That pair's stance and target | Same pair | `Relationship.delta` | `(target - stance)` clamped to `-1..=1`. Not a free delta. `apply_relationship` still returns `SKIPPED` when the once-per-world-day rule fails. |
| `FirstKnownRumor` | `now.rumors`, speaker id, `budget.max_rumor_hops` | Rumors with `heat >= 1` whose `known_by` contains the speaker, sorted by id. First rumor that has a listener | `Rumor.rumor_id`, `Rumor.fact_ids` copied from that rumor, `Rumor.from` = speaker | If `max_rumor_hops == 0`, the walk fails. `fact_ids` is a copy, not a YAML list. `from` is the speaker, not a YAML field. |
| `FirstListener` | Other actors, adjacency, `allows` | Recipient ids sorted. First id | `Rumor.to` | Adjacent means the same place, or the speaker's relationship map contains the other id. The other's viewer must `allows` every `fact_id`. No listener for this rumor means try the next rumor id. No listener at all means the agenda entry is not taken. |
| `KeepHeat` | That rumor's `heat` | Same rumor | `Rumor.heat` | Copy `rumor.heat`. `apply_rumor` returns `OUT_OF_RANGE` unless heat is in `1..=3`. The walk already required `heat >= 1`. |
| `FirstDueNote` | `profile.notes`, clocks, `now.messages` | File order of `profile.notes` | One `Message`: id, `to`, `place`, `fact_ids`, and `text` copied from that note | The first note where `clocks[due_clock].value >= due_value` and no message has that note's id. Notes are not sorted as a `BTreeMap`. |
| `FirstOwnedClock` | `now.clocks`, speaker id, `humans` | Clock ids sorted. First match | `Clock.clock_id`. `Clock.delta` is the literal `1` on the template | The owner contains the actor, `paused` is false, and `value < max`. Also skip when the clock requires presence and `humans == 0`. The skip happens at selection, so the entry is not taken. |

`Token` on a player row is the queued place or actor. It does not walk `Now`. It is stored in the binding so expand does not re-parse the line.

Bindings and the outcome list are different steps.

1. Before the apply loop, agenda selection and the player-queue snapshot resolve every selector walk against staging `Now` and store a bindings value on the candidate. That value holds the sku and rate, the schedule place, the live `from`, the work-block snapshot, the drift pair and clamped delta, the rumor id, the copied fact ids, the speaker, the listener, the heat, the note fields, and the clock id. A later intent in the same tick cannot change which sku or rumor was chosen. A walk that fails means that agenda entry is not a candidate. Player rows that use only literals and `Token` bind the token, and travel also binds the live `from`.
2. Inside the per-candidate loop, `resolve_check` runs if the row has a `check`. An unknown id is `UNKNOWN_KEY`. Templates do not run. The die cursor does not move.
3. After that roll, the selected templates are `effects`, plus `success` if the roll succeeded, plus `failure` if it failed. No roll means `effects` only. Those templates are filled from the frozen bindings, not by walking `Now` again. The result is a `Vec` of `EffectClass`.
4. Expand wraps that vec with `as_viewer` and `intent_id`. That post-roll vec is what `apply_effect` sees and what the tick JSONL stores. `parse_recorded_line` keeps the intents and does not read `effects`. `run_recorded_tick` expands those intents again, calls `resolve_check` again from the recorded seed, and then calls `apply_effect`. Replay does not install the log as `Now`. The names `need` and `resource` are not variants.

A vec built before the roll cannot be both the success list and the failure list. The pre-roll value is the bindings. The post-roll value is the effect vec. Agenda rows may have `success` and `failure`. Outcome lists are not limited to player rows.

The queued player value is not a struct per verb.

```rust
pub struct Intent {
    pub id: String,
    pub tick_index: u64,
    pub actor_id: String,
    pub actor_kind: ActorKind,
    pub source: Source,
    pub row: String,
    pub token: Token,
}

pub enum Token {
    None,
    Actor(String),
    Place(String),
}

pub enum Source {
    Kernel,
    Player,
    System,
}
```

Kernel candidates and player candidates in the tick are an `Intent` plus bindings. The player queue on `Shard` stores `Intent` only. The post-roll effect vec is tick-local until the JSONL write. Replay keeps the recorded intents and expands them again.

Priority, computed before the roll, is unchanged in order: player before NPC before system, then PC-owned clocks, claimed targets, notable tier, then `shuffle_key`. `shuffle_key` is Blake2s of shard, tick, and actor. "PC-owned clock" uses the frozen clock id in the bindings when the clock walk resolved, otherwise a literal clock id on the template. "Claimed" means the token actor is in `actors_present`, or the token place contains a present PC, or a frozen note's `to` hits `actors_present`. Notable tier is `Named`, `Major`, or `plot_relevant`.

Library paths return `Result`. The CLI prints `Reason::token()` and exits 1.

## Agenda and audience

`TickProfile` gains `agenda`, a `Vec` of entries in file order. The first entry whose `when` gates all pass and whose selectors resolve to at least one class, or an explicit yield, is the NPC's intent. Later entries are not looked at.

An entry has a row name, an optional pause lane (`named_plot_clocks`, `subsistence_clocks`, `relationship_drift`, or `travel`), an optional schedule gate (`here` or `away`), an optional activity string, and the flags `when_stock`, `when_drift`, `when_rumor`, `when_note`, and `when_clock`.

`pause` requires that lane's pause view to be running. The view is filled from the four policies in `tick.yaml` before agenda selection. `schedule: away` is the travel predicate in the selector table: the winning schedule row's place differs from the actor's place and lies in range. `schedule: here` plus `activity` is the work or rest gate: the winning row's place is the actor's place and the winning row's activity equals the string. `when_stock` is ready only when `FirstRateSku` returns a sku. `when_drift`, `when_rumor`, `when_note`, and `when_clock` are ready only when the matching walk returns a value. Notes use file order. Rumors, listeners, drift partners, and clocks use the orders in the selector table. An entry with no `when_*` flag and no schedule gate is always ready if its pause lane is running. An agenda entry names an `audience: agenda` row, or load returns `SCHEMA`.

`audience` is `player`, `agenda`, or `system`. The player parser queues only `player`. Agenda selection queues only `agenda`. No call site resolves `system`.

Eligibility runs first. The NPC must be an NPC, the profile must say eligible, the NPC must not be `busy`, no PC may be in that place, tier or schedule must qualify, the NPC must not have already acted this tick, and the idle-day or live-hour cap must be unspent. An ineligible NPC produces no agenda intent. The shop stays still while a player stands in it because that NPC is not eligible. Plot clocks with `requires_presence` are paused while `humans == 0` and the policy is `pause_when_empty`. Subsistence, drift, and travel keep their own policies. `run_if_schedule` still leaves the travel view running. The schedule gate is the real condition.

Kernel agenda does not emit `say` or `talk`. A player world line while `humans == 0` is refused at queue time with `TABLE_EMPTY`. A five-name denylist is not part of agenda selection.

## Failure tokens

`Reason` is one `thiserror` enum in the kernel. It stays fieldless and `Copy`. `Display` and `token()` are the wire words. There is no parallel code enum. JSONL lowers only `BUDGET` and `SKIPPED`, plus the log word `empty_perceive`. The CLI prints the token and exits 1. The player socket sends `{"kind":"refused","code":...}`.

`#[from]` is not used on `Reason`. I/O in the kernel collapses to `COMMIT_CONFLICT` or `SCHEMA`, so `Reason` does not wrap `std::io::Error`. The bin has its own error with `Reason` and `Io` variants. `#[from]` is used there, and only there. Tick functions return `kernel::Reason`. A tick-cause value is not an error.

Library and tick paths do not panic. Exhaustive matches replace `unreachable`. Tests may unwrap. A broken internal invariant returns `SCHEMA` rather than panicking, so a pack bug stays a token.

| Old emission | Token | One meaning |
|---|---|---|
| `NOT_OWNER` when the viewer is not the intent actor | `VIEWER_MISMATCH` | Effect viewer is not the intent actor |
| `NOT_OWNER` when a kernel source acts for a player | `KERNEL_ON_PLAYER` | Agenda tried to act for a PC |
| `NOT_OWNER` when a kernel source acts for an ineligible NPC | `INELIGIBLE` | NPC failed eligibility |
| `NOT_OWNER` when stock, location, or NPC counters need a tick profile and there is none | `NO_PROFILE` | The effect needed a tick profile and there is none |
| `NOT_OWNER` when location, memory, or a system memory names an actor other than the intent actor | `ACTOR_MISMATCH` | The effect names an actor other than the intent actor |
| `NOT_OWNER` when a stock place is outside range, off the graph, or missing a rate | `OUT_OF_RANGE` | The same bounds token location already uses |
| `NOT_OWNER` when a PC changes location while `humans == 0` | `TABLE_EMPTY` | This change requires a seated human |
| `LIVE_ONLY` on the five-name denylist | retired | Those names are not a denylist |
| `LIVE_ONLY` on `talk` while `humans == 0`, or any player world line while `humans == 0` | `TABLE_EMPTY` | The table is empty |
| The `empty_perceive` log word | `EMPTY_PERCEIVE` | JSONL spelling stays `empty_perceive`. It is not a player-socket token unless the seat surfaces it as `refused`. |

`NOT_OWNER` and `LIVE_ONLY` are not variants. Nothing emits them.

Tokens that already have one meaning and stay: `ILLEGAL_CLASS`, `ILLEGAL_KNOWLEDGE`, `OUT_OF_RANGE`, `BUDGET`, `PAUSED_CLOCK`, `UNKNOWN_KEY`, `PACK_REV_MISMATCH`, `UNKNOWN_MOVE`, `UNSCOPED_PERCEIVE`, `DENIED_TOOL`, `COMMIT_CONFLICT`, `HAND_EDIT`, `LLM_DISABLED`, `SCHEMA`, `NOT_IN_SPEC`, `SESSION_FULL`, `MISSING_ITEM`, `SKIPPED`.

`NOT_IN_SPEC` stays for a non-loopback bind, an unknown verb, `include_tick: false` with a pending queue, and an unknown tool. It is not the expand result for `give`. `MISSING_ITEM` stays the one meaning of a failed `item` precondition. `ILLEGAL_CLASS` is an unknown class at `parse_effect`, including `need` and `resource`. A field-shape error on a known class is `SCHEMA`. A moves-file unknown class is `SCHEMA`. `EffectClass` has ten variants.

## Commit

`write_commit` is the only rename protocol for world bytes:

1. Copy each live target that already exists into `state/.txn/prev`, and fsync the copies and the `prev` directory.
2. Create `state/.txn/COMPLETE`, and fsync the file and the `.txn` directory.
3. Write each new body to a temp file, fsync, and fsync the parent.
4. Rename side files first (`rumors.yaml`, `log/tick-NNNN.jsonl`, `memories/*.jsonl`).
5. Rename `now.yaml` last.
6. Fsync `state/`. Remove `.txn`. `state_rev` was incremented in memory before the bytes were emitted, and it hits disk only in that last rename.

`recover` keeps three shapes. No `COMPLETE` means delete `.txn` and leave the live tree. `COMPLETE` and `live.state_rev == prev.state_rev` means restore `prev`. `COMPLETE` and `live.state_rev == prev + 1` means delete `.txn` and keep live. Then drop tick logs whose `state_rev_after` is above live, drop memory lines whose `tick_index` is not in a live log, and rewrite `rumors.yaml` from `now.rumors` when it differs. `rumors.yaml` is not a source. The next commit rewrites it from `now.rumors`.

`commit_presence` writes `now.yaml` alone, through the same `write_commit`. Presence is `humans` and `actors_present` only. When those two fields are unchanged after canonicalize, it writes nothing and does not increment `state_rev`. When either field changes, it increments `state_rev` by 1 and then emits. It does not open a `Txn`, does not write a tick log, and does not advance clocks. A session id is not an input.

`migrate_pack` is the only writer that changes `pack_rev`. It goes through `write_commit` and appends one studio line. A hand edit of `now.yaml` that is not `spook migrate` remains `HAND_EDIT` on the next tick. Studio tools do not write `state/`.

`write_commit` has no fault parameter. The crash hook is not an environment variable and not `process::abort`. No production path reads `SPOOK_CRASH_AFTER`. Dropping the `Txn` after `apply_effect` without calling `write_commit` leaves no `state/.txn`, and leaves `state_rev` and stock at the bytes from before `begin`. Recovery on the three directory shapes above is the other half of the same proof: an incomplete `.txn` leaves the previous `now.yaml`, `COMPLETE` with an unmoved `state_rev` restores `prev`, and `COMPLETE` with the new `state_rev` keeps the new `now.yaml` and deletes a tick log and a memory line that do not belong to it.

Replay byte-compares `now.yaml` produced by this kernel. It does not pin a frozen docks hash.

## Speech

`say` and `talk` are speech. They are not rows. The tape is not an input to expand. Text is not parsed into `known_facts`. There is no whisper that grants a fact.

`talk` while `humans` is 0 returns `TABLE_EMPTY` and appends nothing. `say`, or `talk` with a human seated, takes `state/.session.lock`, appends one object, and fsyncs. That path uses no `Txn` and does not bump `state_rev`. The object keeps `seq`, `from`, `address`, `place`, `actor`, `recipients`, `text`, `tick_index`, and `world_time`. `from` is `player:{id}` or `gm`. A typed `from` on the GM tool is `SCHEMA` and appends nothing. An NPC `from` is not representable. The speech parser has no NPC speaker.

`read_session` returns the tape lines after a sequence number, up to a limit. The limit defaults to 64 and is clamped to 256. The file hard cap is 1 MiB. `append_session` reads the tail for the last `seq` and returns `OUT_OF_RANGE` when the next append would pass 1 MiB. A line that does not parse is `SCHEMA` for the whole read, not a skipped line. `session_status` with `since` uses this window.

## Rounds, seats, and profiles

With no GM, the runtime queues and commits one live tick of `live_minutes_per_tick` (default 5). With a GM seated, the line waits. `commit_turn` requires `include_tick: true`. An empty queue is a no-op and writes no tick file. `include_tick: false` with a non-empty queue is `NOT_IN_SPEC`.

One wake: the runtime sleeps `tick_every_real_seconds`, reset when a tick actually advanced, and posts a wake. The shard thread wakes the live shard. A second open of the pack on that wake is not the scheduler.

Pack profiles stay three static lists: `pack-table`, `pack-tick`, and `pack-studio`. `pack-table` does not include `say`. Order stays the order of `TABLE_TOOLS`, `TICK_TOOLS`, and `STUDIO_TOOLS` in `src/mcp/profile.rs`.

Sitting perceive allows `player:` and `npc:`. Tick-profile perceive rejects a player viewer with `UNSCOPED_PERCEIVE`. A missing `as`, or `as=gm`, is `UNSCOPED_PERCEIVE`. They are two functions. Both call the perceive budget after the policy check. `firewall::allows` is the shared visibility test.

`npc_act` on the sitting returns `LLM_DISABLED` and does not call a model. The other refuse-only GM names are not handlers. An unknown sitting tool is `NOT_IN_SPEC` and writes nothing. `apply_effect` and `patch_entity` are not sitting handlers. Effects come from row expansion during the tick.

`spook gm --stdio` remains the bridge. `spook gm` without `--stdio` is `NOT_IN_SPEC`. The stdin loop that accepts only `close` is not a ported behavior.

## Loopback and one seat

One seat. A second player or a second GM is `SESSION_FULL`.

The bind does not grow past loopback. `--bind 127.0.0.1` and `--bind 127.0.0.1:<port>` are the only legal values. The default listen address is `127.0.0.1:9737`. Any other host, including `0.0.0.0`, a LAN address, or a hostname, is `NOT_IN_SPEC` before the pack opens. `--as`, and `--mcp` combined with `--bind`, are `NOT_IN_SPEC` before the pack opens. There is no account store.

`--mcp` is list-only. It does not call `clear_crashed`, and it does not sit a GM.

## Host monitor

A host monitor is not shard behavior.

`idle_digest` takes a host viewer or a player viewer. The host sees effect rows and clocks. A player sees only their messages, allowed rumor facts, and owned clocks. `spook status` prints the host digest. Player resume is the player digest. The host digest is not a player resume, and it does not write the shard.

## Firewall, lint, and digest

`allows` means the fact is known, or it is not hidden and it is on the viewer's place, or it is not hidden and it is on a co-located owner's cast or place. Hidden facts never leak through place or cast prose.

`perceive` is budgeted. The budget is 800 tokens unless the caller passes a budget. `lint` scans Markdown bodies for hidden fact text. `strip_body` deletes the body, leaves frontmatter, and the pack still ticks. Those functions stay in firewall, perceive, digest, and lint.

Load fails with `SCHEMA` if `places.actors` is edited apart from `actors.place`. Emit rebuilds occupancy from `actors.*.place`.

Faces of a roll go to the GM socket only. The player socket gets `{"kind":"check","success":true|false}` or `{"kind":"refused","code":"..."}`. No faces. The tick JSONL lists `resolve_check` and `apply_effect` without faces. A refused tick records `rejected[].reason` as the token: `BUDGET` and `SKIPPED` lowercased, `empty_perceive` as that word, every other token uppercase.

## Non-goals

Still not built, and not implied by a missing sentence above:

- A model call inside `npc_act`, a cargo feature named `llm`, or an NPC `from` on the tape.
- Parsing speech into `known_facts`, or a whisper that grants a fact.
- A non-loopback bind, accounts, or a second GM.
- An ack cursor, or a per-client outbound queue whose purpose is to make a host event start a host turn.
- `apply_effect` or `patch_entity` on the sitting.
- Studio tools that write `state/`.
- Rewriting `.grok/skills/gm-docks`.
- Changing the player wire kinds `refused`, `check`, and `say`.
- WebSockets, HTTP, `--serve`, or a database.
- A second spec, an archive copy of an older spec, or a banner that keeps the older text in the worktree. Tags stay. The files do not.
- Publishing a crate named `spook`.

The player line grammar does change. The check wire object does not. That split is intentional.

## Threat list

| How a second writer appears | Countermeasure |
|---|---|
| Client patches YAML | The play door queues an intent or appends speech. The CLI has no path that opens `now.yaml` for a client write. |
| GM calls `apply_effect` | The tool is not a sitting handler. Effects come from expand inside a `Txn`. Only `tick_commit`, `commit_presence`, and `migrate_pack` rename files. |
| `npc_act` or a summarizer writes memories | No LLM crate. `npc_act` returns `LLM_DISABLED`. Memory lines are written inside the commit, from a `memory` effect, after `allows`. |
| Speech copied into `known_facts` or a rumor | The tape is not an input to expand. No whisper. Inbound model text is not a field on `Intent`. |
| Host digest reused as the player resume | `idle_digest` takes `DigestViewer::Host` or `DigestViewer::Player`. Different rows. |
| Hand edit of `now.yaml` | `HAND_EDIT` until `spook migrate`. |
| Replay applies JSONL effects as the new state | Replay does not install the log as `Now`. It re-expands the recorded intents, calls `resolve_check` again from the recorded seed, and then calls `apply_effect`. |
| Two ticks in flight | One lock around the tick run and `commit_turn`. The second caller gets `COMMIT_CONFLICT`. Tick code is synchronous on the shard thread. |
| `patch_entity` writes canon and the live shard | Not a sitting tool. The studio list-only server does not write `state/`. A canon byte change mismatches `pack_rev` until migrate. |
| New effect class in a handler | `ILLEGAL_CLASS` at the parse boundary. `EffectClass` has ten variants. |
| A meter whose stored pair is outside the definition | `SCHEMA` at load. Apply clamps when the sum fits in `i64`. `OUT_OF_RANGE` leaves the record unchanged when the sum does not. |
| `perceive` with no `as`, or `as=gm` | `ViewerId` has no GM variant. Tick perceive rejects players. Sitting perceive rejects anything but `player:` and `npc:`. |
| `places.actors` edited apart from `actors.place` | Load fails `SCHEMA`. Emit rebuilds occupancy from `actors.*.place`. |
| `state/rumors.yaml` treated as live | Ignored as a source. Recovery and the next commit rewrite it from `now.rumors`. |
| Feature `llm` | No such feature. |
| Tokio task per shard inside the kernel | Kernel and tick cannot name the runtime. The scan stays. |
| `rand::thread_rng` or a hand-rolled die | `die_face` is Blake2s. One copy, in `check.rs`. |
| Markdown body used as a stock count or a hidden fact | `strip_body` and `lint`. |
| A second snapshot beside `now.yaml` | Forbidden. One emit path. |
| Non-loopback bind, accounts, second GM | `NOT_IN_SPEC`, no account store, `SESSION_FULL`. |
| Hidden fact in a perceive body | `allows` plus lint. |

Privacy: the pack is local. There is no account, no connection secret, and no telemetry. The tape holds table speech on disk under the pack. That is the product log. It is not shipped anywhere.
