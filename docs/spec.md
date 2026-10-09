# Spookery referee spec

This file is the only normative spec. A rule it does not state is unspecified.

The engine is open source (AGPL-3.0). Runtime code, the tests of that code, and repository tooling and scripts are AGPL-3.0-or-later. Documentation, schemas, and packs are CC BY 4.0.

This spec records one writer, the crate boundary, the threat list, loopback and one seat, and the rule that a host monitor is not shard behavior.

## Referee rules

These fourteen rules are the referee, in this order. Later sections are the detail. A sentence in another document does not add a fifteenth rule.

1. **Import boundary.** `spookery-kernel` and `spookery-tick` do not link a runtime, a database, or a model SDK.
2. **`!Send` `Txn`.** One `std::thread` owns `Now`. The tick is not spawned on a runtime.
3. **Atomic commit.** `now.yaml` is renamed last. A crash cannot apply a partial tick.
4. **Closed effect, one apply.** Seven effect classes. `apply_effect` is the only mutator of world numbers.
5. **Firewall.** `firewall::allows` is the visibility test. A hidden fact does not leak through place or cast prose.
6. **Deterministic tick.** A row's literals are filled before the apply loop. Replay re-expands the recorded intents and calls `resolve_check` again from the recorded seed.
7. **Canon versus `Now`.** Canon is frozen. `Now` is the live snapshot.
8. **Action-row loader.** A moves file is names and rows. A field is a literal, or the player token on a player row. A new verb is a new row in YAML.
9. **Seeded checks.** `die_face` is Blake2s. One copy, in `check.rs`. `difficulty` applies only when the row sets `check` and no opposing total applies.
10. **Eligibility and habits.** Both run before a kernel habit intent exists. An ineligible NPC produces no habit intent. A player-sourced intent is not a habit intent.
11. **Failure tokens.** Each token has one meaning. `NOT_OWNER` and `LIVE_ONLY` are not tokens.
12. **Speech versus queue.** Speech appends to the tape. It is not a row, it does not enter the intent queue, and it does not tick.
13. **Seat law.** One player seat and one GM seat. The seat's `tools/list` is the menu.
14. **Rounds.** With no GM, the runtime queues and commits one live tick. With a GM seated, the line waits for `commit_turn`.

## One writer

The shard thread is the only writer of `Now` and of `Seats`. One `std::thread` owns `Now`. `Txn` stays `!Send`, with `PhantomData<Rc<()>>` as in `crates/kernel/shard.rs`. No `tokio::spawn` of a tick.

The queue is a `Vec` of intents on the shard thread. It is ephemeral. A crash drops every uncommitted line. A crash cannot apply a partial tick. The queue is not a field of `now.yaml`. It is not a side file that recovery has to remember.

`Txn::begin` clones `Now` once into `staging`. That is the only clone. `commit` moves `staging` into `Shard.now` after `write_commit` returns. It does not clone again.

`expand` takes `&Canon`, `&Now` (the staging borrow), and `&Intent`. It does not hold `Shard`. `apply_effect` takes `&mut Staging`. `Staging` borrows `Txn`'s staging, the typed key set, and the active intent, so it is `!Send`. The tick does not hold `&Shard` and `&mut Txn` in a way that aliases `Now`. Checks read `current` off the pre-loop `frozen` snapshot, the same `Now` expand uses. They do not read live staging. The item precondition reads the actor's inventory before queue. `stage_tick` receives `&mut Txn` and the queue snapshot: the `Vec` of intents is moved out for the tick and put back only if the tick is abandoned. `Shard` fields are private.

`append_session` takes the session lock and never a `Txn`. `commit_presence` takes the shard lock and never a `Txn`.

`Seats` is the set of live socket senders plus a process-local session counter. It is not a second presence document. The durable source of who is at the table is `Now.presence`, and that struct is only `humans` and `actors_present`. Session ids are not in it. The shard thread is the only writer of both `Seats` and `Now`. A player hello or drop that changes `humans` or `actors_present` calls `commit_presence` before the handler returns. A GM hello does not change those fields and does not write `now.yaml`.

Invariant, checked by a debug assert on the shard thread: `seats.player.is_some()` iff `presence.actors_present` contains the seated PC and `presence.humans` is 1. The assert is debug-only. It is not a library `unwrap`. Startup repair is `clear_crashed`. `listen` binds, then the shard thread opens the shard and, if `humans != 0` or `actors_present` is non-empty while `Seats` is empty, `commit_presence` writes zeros.

## Crate boundary

Three crates, all under `crates/`. `crates/kernel` is `spookery-kernel`. `crates/tick` is `spookery-tick`. `crates/spook` is package `spook`, library `spookery`, binary `spook`. Protocol, the player client, and the model client are modules of that library: `protocol`, `cli`, and `ai`. A one-crate collapse is not the layout. The `spook` package may link a runtime. The `ai` module may link an HTTP client.

| Crate or module | May link | Must not link |
|---|---|---|
| `protocol` | `serde`, `serde_json` | `tokio`, `rmcp`, `reqwest`, `spookery-kernel`, `cli`, `ai` |
| `spookery-kernel` | `serde`, `serde-saphyr` `=1.3.0`, `serde_json`, `blake2`, `thiserror`, std | `tokio`, `rmcp`, `rmcp-macros`, `sqlx`, `postgres`, `redis`, `jsonschema`, `reqwest`, any model SDK, `spookery-tick`, the `spook` crate |
| `spookery-tick` | `spookery-kernel` | `tokio`, `rmcp`, `reqwest`, `serde_json` as a direct dependency (it uses `kernel::codec`), MCP, any model SDK, the `spook` crate |
| `cli` | `protocol`, `spookery-kernel` | `ai`, `reqwest`, `rmcp`, `tokio` |
| `ai` | `protocol`, `spookery-kernel`, an HTTP client, a runtime | `spookery-tick`, `cli`, `rmcp`, any model SDK |
| bin `spook` | `spookery-kernel`, `spookery-tick`, `tokio`, `rmcp`, `socket2`, `serde_json`, and its `protocol`, `cli`, and `ai` modules | `reqwest` outside `crates/spook/src/ai/`. Seat code that calls `cli` or `ai`. Business rules that belong in the kernel. A second tick implementation. |

`serde_json` in the kernel is allowed. The import scan does not forbid it. The scan stays a source scan, not a `Cargo.toml` parser. It reads every runtime file under `crates/kernel`, `crates/tick`, and `crates/spook/src`. `crates/kernel` and `crates/tick` must not contain the byte strings `tokio`, `rmcp`, `sqlx`, `postgres`, `redis`, `reqwest`, `langchain`, `openai`, `anthropic`, or `ollama`. `reqwest` may appear only under `crates/spook/src/ai/`. `protocol` must not contain `tokio`, `rmcp`, `reqwest`, `spookery_kernel`, `crate::cli`, or `crate::ai`. `cli` must not contain `tokio`, `rmcp`, `reqwest`, or `crate::ai`. `ai` must not contain `rmcp`, `crate::cli`, or `spookery_tick`. `seat` must not contain `crate::cli` or `crate::ai`. A comment in a scanned file that names a banned crate is still a failure. This spec is where that ban is written. There is no cargo feature named `llm`. An optional cargo feature may be named `ai`.

Tokio in `spook` is the accept loop, keepalive, and scheduler sleep. The tick function runs on the shard thread.

## Effect envelope

`Effect` is one struct. The class is the payload. Rows emit the seven classes below.

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
    Memory { actor_id: String, fact_ids: Vec<String>, tags: Vec<String>, text: Option<String> },
    Flag { actor_id: String, key: String, value: bool },
    Message { message_id: String, to: Vec<String>, place: Option<String>, fact_ids: Vec<String>, text: String },
}
```

`ViewerId` is `Player` or `Npc`. There is no `Gm` variant. The JSON token is `player:{id}` or `npc:{id}`.

JSONL and MCP write one flat object per effect. The object is internally tagged and flattened. `class` is a string: `clock`, `meter`, `stock`, `location`, `memory`, `flag`, or `message`. `MeterWrite` is `Base(i64)`, `Offset(i64)`, or `Snap`. The wire writes `write` as `base`, `offset`, or `snap`, and writes `delta` only for `base` and `offset`. Payload fields are siblings of `as` and `intent_id`. They are not nested under `class`. An externally tagged object is not the codec. Key order is fixed: `class`, then `as`, then `intent_id`, then payload fields in the order declared on that `EffectClass` variant. A custom serializer writes that order. serde's default enum encoding does not. `now.yaml` byte-stability does not depend on this object. It is the tick JSONL shape only. Tests assert this order.

## Apply

`apply_effect` is the only mutator of world numbers. Arithmetic stays `checked_add`. A meter write clamps a sum that fits in `i64`. Stock refuses a negative result. Memory text longer than 240 characters is `SCHEMA`. Memory facts the viewer may not know are `ILLEGAL_KNOWLEDGE`. The flag key `busy` is `SCHEMA` at load and is not a template. `busy` remains the eligibility bit on the actor.

The effect's viewer must be the active intent's actor. Failure is `VIEWER_MISMATCH`.

Clocks do not clamp. An unknown clock is `UNKNOWN_KEY`. A paused clock, or a clock that requires presence while `humans == 0`, is `PAUSED_CLOCK`. `value + delta` outside `[0, max]` is `OUT_OF_RANGE`. If the intent actor is not in `clock.owners`, the result is `UNKNOWN_KEY`. That actor has no such clock key.

Location: `from` must equal the live place, and `to` must be in the profile range. Otherwise `OUT_OF_RANGE`. An `actor_id` other than the intent actor is `ACTOR_MISMATCH`. A PC location while `humans == 0` is `TABLE_EMPTY`. An NPC with no tick profile is `NO_PROFILE`.

Stock: a missing profile is `NO_PROFILE`. A place outside range, an absolute delta above `stock_rate`, or a negative result, is `OUT_OF_RANGE`.

Meter: `write` is `base`, `offset`, or `snap`. `base` adds the delta to `base` and clamps that base into `[min, base_cap]`, and keeps `offset`. `offset` adds the delta to `offset` and keeps `base`. `snap` sets the next pair to `(base, 0)`. The live pair is assigned only when `base` is inside `[min, base_cap]` and `base + offset` fits in `i64`. A sum that fits and passes `max` is clamped when `current` is read. `current` is `clamp(base + offset, min, max)`. It is not stored. An add that does not fit in `i64`, or a next pair that fails that check, is `OUT_OF_RANGE` and the record is unchanged. An unknown meter id is `UNKNOWN_KEY`. A PC and an NPC take the same write.

## Rows and habits

A moves file is still a name or a row. The schema version string stays `moves/v0`. New fields are optional. A misspelled field is rejected because unknown fields are denied.

A row has:

- `audience`: `player` or `habit`. Omitted audience, or `system`, is `SCHEMA`. A row is not implicitly player-facing.
- one token: `none`, `actor`, or `place`
- one subject: `speaker` or `named`
- `effects`: templates that run on both outcomes
- `success` and `failure`: templates that run on that outcome
- optional `check`: an id that must exist in `checks.yaml`, or load returns `SCHEMA`
- optional `difficulty`: `i64`, default 0, used only when `check` is set and no opposing total applies
- optional `item`: an actor inventory precondition

`classify` queues a row only when `audience` is `player`. A `habit` name is `NOT_IN_SPEC` on a player line and is not queued. The GM intent tool queues a `player` row for a player or an NPC, and a `habit` row only when `actor_kind` is `npc`. A player actor queuing a habit row is `NOT_IN_SPEC`. `source` on that tool stays `player`. A cast habit that names a `player` row is `SCHEMA` at load. Empty-table gates still apply to a kernel habit intent. They do not apply to a player-sourced intent.

Load rejects a duplicate name, an empty name, an empty `item`, the flag key `busy`, a row named `say`, an unknown class, a search word, a template whose keys do not match its class, and any unknown shape. `say` is the speech line, so that name is not a row. `travel` is not special-cased in Rust. A travel row is whatever templates it has, and `apply_location` enforces the range. The docks grit check is a `grit` row. `attempt grit 3` is not a special parser. The wire object `{"kind":"check","success":true|false}` does not change.

**Yield.** The key `effects` may be present and empty only when `success` and `failure` are absent. That row is a yield. Omitted `effects` together with omitted outcome lists, on a row that set `token` or `on`, is `SCHEMA`. A yield with no `check` expands to zero effects and is still an applied intent if it was allowed. A yield with a `check` rolls, emits the check notice, and writes no world key. Nothing matches on the name `wait` or `attempt`. `wait` is a yield row (`effects` present and empty). An NPC habit that does not list `wait` does not emit it. Absence of an on habit whose row expands means no kernel intent.

**Fields.** A template field is a literal, except `to: token` on a player travel row. That token is the queued place. Search words (`here`, `first_rate`, `rate`, `first_drift`, `toward_target`, `first_known`, `first_listener`, `keep`, `first_due_note`, `first_owned`) are `SCHEMA` at load. There is no expression language. Adding a verb is a new row in YAML.

Stock names `place`, `sku`, and `delta`. `apply_stock` still refuses a place outside range, an absolute delta above `stock_rate`, or a negative result. Message names `id`, `to`, `place`, `fact_ids`, and `text`. Clock names `clock` and `delta`. Location `from` is the actor's live place and is not a YAML field. A meter template is a literal key and write. There is no stock maximum in the docks profile.

Bindings and the outcome list are different steps.

1. Habit selection picks a row before the apply loop. Expand fills that row's literals against the staging `Now` it sees, including `from` taken from the actor's live place. A row that does not expand is not the kernel intent. Player travel binds the typed place.
2. Inside the per-candidate loop, `resolve_check` runs if the row has a `check`. An unknown id is `UNKNOWN_KEY`. Templates do not run. The die cursor does not move.
3. After that roll, the selected templates are `effects`, plus `success` if the roll succeeded, plus `failure` if it failed. No roll means `effects` only. Those templates are filled from the row, not by choosing a new target. The result is a `Vec` of `EffectClass`.
4. Expand wraps that vec with `as_viewer` and `intent_id`. That post-roll vec is what `apply_effect` sees and what the tick JSONL stores. `parse_recorded_line` keeps the intents and does not read `effects`. `run_recorded_tick` expands those intents again, calls `resolve_check` again from the recorded seed, and then calls `apply_effect`. Replay does not install the log as `Now`. The names `need` and `resource` are not variants.

A vec built before the roll cannot be both the success list and the failure list. The post-roll value is the effect vec. Habit rows may have `success` and `failure`. Outcome lists are not limited to player rows.

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

Priority, computed before the roll, is player before NPC before system, then PC-owned clocks, claimed targets, then `shuffle_key`. `shuffle_key` is Blake2s of shard, tick, and actor. "PC-owned clock" uses the clock id on the expanded effect. "Claimed" means the token actor is in `actors_present`, or the token place contains a present PC, or a message's `to` hits `actors_present`.

Library paths return `Result`. The CLI prints `Reason::token()` and exits 1.

## Habits

A tick profile has `habits`, a list in file order. The idle runner produces at most one kernel intent per eligible NPC. It takes the first habit that is on and whose first expanding row is an `audience: habit` row. Later habits wait. A habit that is always ready starves the ones under it. No such row means no kernel intent.

A habit is on when every gate it sets is true. `runs_while_empty` is required. When `humans == 0`, only a habit with `runs_while_empty: true` can be chosen. When someone is seated, that flag does not block the runner. `from_hour` and `to_hour` are optional and come together. Each is an integer in `0..24`. The hour is `(world_time / 60) % 24`, read after the tick has advanced world time. `from < to` means `from <= hour < to`. `from > to` wraps midnight. `from == to` is never on. The check reads the hour. It does not write at the window's edge. `clock` and `at_least` are optional and come together. The habit is on when that clock exists, is not paused, and `value >= at_least`. A missing clock means the habit is off. `unless_message` is optional. The habit is off when a message with that id is already in `now.messages`.

An empty id, a duplicate id, an empty row list, a half-set hour pair, a half-set clock pair, or an hour outside `0..24` is `SCHEMA`. A habit row name that is not an `audience: habit` row is `SCHEMA`.

`audience` is `player` or `habit`. The player parser queues only `player`. Habit selection queues only `habit`.

Eligibility runs first. The NPC must be an NPC, the profile must say eligible, the NPC must not be `busy`, and no PC may be in that place. The NPC must not have already acted this tick, and the idle-day or live-hour cap must be unspent. An ineligible NPC produces no habit intent. The shop stays still while a player stands in it because that NPC is not eligible. `allowed` checks those gates only for `source: kernel`. A player-sourced intent is not checked against the hour, the clock, `unless_message`, `runs_while_empty`, or eligibility. Apply still checks range, stock rate, the firewall, paused clocks, and the other effect rules.

NPC perceive includes `habits`, the ids that are on for that NPC, in file order. Player perceive sends an empty list. Queueing an intent does not consult that list.

`tick.yaml` pause is `named_plot_clocks`. `now.pause` stores that one view. Plot clocks with `requires_presence` are paused while `humans == 0` and the policy is `pause_when_empty`.

Kernel habits do not emit speech. A player world line while `humans == 0` is refused at queue time with `TABLE_EMPTY`. A five-name denylist is not part of habit selection.

## Failure tokens

`Reason` is one `thiserror` enum in the kernel. It stays fieldless and `Copy`. `Display` and `token()` are the wire words. There is no parallel code enum. JSONL lowers only `BUDGET`, plus the log word `empty_perceive`. The CLI prints the token and exits 1 for an engine refusal. A usage error prints clap's message and exits 2. The player socket sends `{"kind":"refused","code":...}`.

`#[from]` is not used on `Reason`. I/O in the kernel collapses to `COMMIT_CONFLICT` or `SCHEMA`, so `Reason` does not wrap `std::io::Error`. The bin has its own error with `Reason` and `Io` variants. `#[from]` is used there, and only there. Tick functions return `kernel::Reason`. A tick-cause value is not an error.

Library and tick paths do not panic. Exhaustive matches replace `unreachable`. Tests may unwrap. A broken internal invariant returns `SCHEMA` rather than panicking, so a pack bug stays a token.

| Old emission | Token | One meaning |
|---|---|---|
| `NOT_OWNER` when the viewer is not the intent actor | `VIEWER_MISMATCH` | Effect viewer is not the intent actor |
| `NOT_OWNER` when a kernel source acts for a player | `KERNEL_ON_PLAYER` | A kernel habit tried to act for a PC |
| `NOT_OWNER` when a kernel source acts for an ineligible NPC | `INELIGIBLE` | NPC failed eligibility |
| `NOT_OWNER` when stock, location, or NPC counters need a tick profile and there is none | `NO_PROFILE` | The effect needed a tick profile and there is none |
| `NOT_OWNER` when location, memory, or a system memory names an actor other than the intent actor | `ACTOR_MISMATCH` | The effect names an actor other than the intent actor |
| `NOT_OWNER` when a stock place is outside range, or missing a rate | `OUT_OF_RANGE` | The same bounds token location already uses |
| `NOT_OWNER` when a PC changes location while `humans == 0` | `TABLE_EMPTY` | This change requires a seated human |
| `LIVE_ONLY` on the five-name denylist | retired | Those names are not a denylist |
| `LIVE_ONLY` on a player world line while `humans == 0` | `TABLE_EMPTY` | A world line needs a seated human |
| The `empty_perceive` log word | `EMPTY_PERCEIVE` | JSONL spelling stays `empty_perceive`. It is not a player-socket token unless the seat surfaces it as `refused`. |

`NOT_OWNER` and `LIVE_ONLY` are not variants. Nothing emits them.

Tokens that already have one meaning and stay: `ILLEGAL_CLASS`, `ILLEGAL_KNOWLEDGE`, `OUT_OF_RANGE`, `BUDGET`, `PAUSED_CLOCK`, `UNKNOWN_KEY`, `PACK_REV_MISMATCH`, `UNKNOWN_MOVE`, `UNSCOPED_PERCEIVE`, `COMMIT_CONFLICT`, `HAND_EDIT`, `SCHEMA`, `NOT_IN_SPEC`, `SESSION_FULL`, `MISSING_ITEM`.

`NOT_IN_SPEC` stays for a non-loopback bind, an unknown verb, `include_tick: false` with a pending queue, and an unknown tool. It is not the expand result for `give`. `MISSING_ITEM` stays the one meaning of a failed `item` precondition. `ILLEGAL_CLASS` is an unknown class at `parse_effect`, including `need` and `resource`. A field-shape error on a known class is `SCHEMA`. A moves-file unknown class is `SCHEMA`. `EffectClass` has seven variants.

## Commit

`write_commit` is the only rename protocol for world bytes:

1. Copy each live target that already exists into `state/.txn/prev`, and fsync the copies and the `prev` directory.
2. Create `state/.txn/COMPLETE`, and fsync the file and the `.txn` directory.
3. Write each new body to a temp file, fsync, and fsync the parent.
4. Rename side files first (`log/tick-NNNN.jsonl`, `memories/*.jsonl`).
5. Rename `now.yaml` last.
6. Fsync `state/`. Remove `.txn`. `state_rev` was incremented in memory before the bytes were emitted, and it hits disk only in that last rename.

`recover` keeps three shapes. No `COMPLETE` means delete `.txn` and leave the live tree. `COMPLETE` and `live.state_rev == prev.state_rev` means restore `prev`. `COMPLETE` and `live.state_rev == prev + 1` means delete `.txn` and keep live. Then drop tick logs whose `state_rev_after` is above live, and drop memory lines whose `tick_index` is not in a live log.

`commit_presence` writes `now.yaml` alone, through the same `write_commit`. Presence is `humans` and `actors_present` only. When those two fields are unchanged after canonicalize, it writes nothing and does not increment `state_rev`. When either field changes, it increments `state_rev` by 1 and then emits. It does not open a `Txn`, does not write a tick log, and does not advance clocks. A session id is not an input.

`migrate_pack` is the only writer that changes `pack_rev`. It goes through `write_commit` and appends one studio line. A hand edit of `now.yaml` that is not `spook migrate` remains `HAND_EDIT` on the next tick. Studio tools do not write `state/`.

`write_commit` has no fault parameter. The crash hook is not an environment variable and not `process::abort`. No production path reads `SPOOK_CRASH_AFTER`. Dropping the `Txn` after `apply_effect` without calling `write_commit` leaves no `state/.txn`, and leaves `state_rev` and stock at the bytes from before `begin`. Recovery on the three directory shapes above is the other half of the same proof: an incomplete `.txn` leaves the previous `now.yaml`, `COMPLETE` with an unmoved `state_rev` restores `prev`, and `COMPLETE` with the new `state_rev` keeps the new `now.yaml` and deletes a tick log and a memory line that do not belong to it.

Replay byte-compares `now.yaml` produced by this kernel. It does not pin a frozen docks hash.

## Speech

`say` is speech. It is not a row. The tape is not an input to expand. Text is not parsed into `known_facts`. There is no whisper that grants a fact.

A player line is `say` plus the words. The first word is the address when it is `place`, or an actor id, and more words follow. `place` is the speaker's live place. An actor id is that actor. Any other line is address `table`, and every word is the text. A line of one word is table text, including when that word is `place` or an actor id. An empty `say` is `SCHEMA`.

Speech appends when `humans` is 0. The append takes `state/.session.lock`, writes one object, and fsyncs. That path uses no `Txn` and does not bump `state_rev`. The object keeps `seq`, `from`, `address`, `place`, `actor`, `recipients`, `text`, `tick_index`, and `world_time`. `from` is `player:{id}` or `gm`. A typed `from` on the GM tool is `SCHEMA` and appends nothing. An NPC `from` is not representable. The speech parser has no NPC speaker.

Recipients are seated player actors. `table` includes every seated player. `place` includes the seated players in that place. `actor` includes that actor when they are a seated player. The speaking player is included. An NPC stored in `actor` is on the tape and is not a recipient.

The GM tool is the same act. `address` is `table`, `place`, or `actor`. `place` is required when the address is `place`, and `actor` is required when the address is `actor`. `actor` must name an actor. The other field is absent.

`read_session` returns the tape lines after a sequence number, up to a limit. The limit defaults to 64 and is clamped to 256. The file hard cap is 1 MiB. `append_session` reads the tail for the last `seq` and returns `OUT_OF_RANGE` when the next append would pass 1 MiB. A line that does not parse is `SCHEMA` for the whole read, not a skipped line. `session_status` with `since` uses this window.

## Rounds, seats, and profiles

With no GM, the runtime queues and commits one live tick of `live_minutes_per_tick` (default 5). With a GM seated, the line waits. `commit_turn` requires `include_tick: true`. An empty queue is a no-op and writes no tick file. `include_tick: false` with a non-empty queue is `NOT_IN_SPEC`.

One wake: the runtime sleeps `tick_every_real_seconds`, reset when a tick actually advanced, and posts a wake. The shard thread wakes the live shard. A second open of the pack on that wake is not the scheduler.

The seat's `tools/list` is the menu.

Sitting perceive allows `player:` and `npc:`. A missing `as`, or `as=gm`, is `UNSCOPED_PERCEIVE`. Perceive calls the perceive budget after that check. `firewall::allows` is the shared visibility test.

An unknown sitting tool is `NOT_IN_SPEC` and writes nothing. `apply_effect` and `patch_entity` are not sitting handlers. Effects come from row expansion during the tick.

`spook gm` is an unknown subcommand.

## Loopback and one seat

One seat. A second player or a second GM is `SESSION_FULL`.

The bind does not grow past loopback. `--addr 127.0.0.1` and `--addr 127.0.0.1:<port>` are the only legal values. `--addr` is one global flag, and every command checks it. The default address is `127.0.0.1:9737`. Any other host, including `0.0.0.0`, a LAN address, or a hostname, is `NOT_IN_SPEC` before the pack opens, for every command. There is no account store.

## Host monitor

A host monitor is not shard behavior.

`idle_digest` takes a host viewer or a player viewer. The host sees effect rows and clocks. A player sees only their messages and owned clocks. `spook status` prints the host digest. Player resume is the player digest. The host digest is not a player resume, and it does not write the shard.

## Host

`spook ai-gm <pack>` sits the one GM seat and dials `--addr`, which defaults to `127.0.0.1:9737`. It starts neither `spook run` nor `spook play`. One seat remains one seat. A second sitter gets the refusal the hello already returns, and the host exits.

`spook ai-gm` with no pack, or with any extra argument, is a usage error before a dial. A refused hello exits 1. A shard that is not listening exits 1 and leaves the pack bytes as they were.

On start the host reads `XAI_API_KEY` and `SPOOK_GM_MODEL`. Either one missing: it writes that fact to stderr and exits 1 before it dials. `SPOOK_GM_URL` defaults to `https://api.x.ai/v1`. The key is not an argument and not a pack file.

The host calls `pack_rev` and `load_canon` on the pack path. It does not call `open_shard`, `apply_effect`, or `write_commit`. After initialize it calls `session_status` with no arguments and compares `pack_rev`. A mismatch disconnects and exits 1. A match writes `listening` to stderr. stdout stays empty.

A non-empty queue is committed once after the model's turn. Talk alone does not tick. If the model call fails, the host skips the model's says and intents and still commits a queue the player already filled. The seat stays up. A commit that rolled dice gets one say-only follow-up. That follow-up does not queue and does not commit. `commit_turn`'s result includes `checks` (`faces`, `stat`, `total`, `target`, `success`). The same roll notices are still sent as `check` events on the GM socket. The player wire stays `{"kind":"check","success":true|false}`. Faces stay off the player socket.

An addressed actor who is not in the room gets one table say, `They are not here.`, and no model call. On an actor-addressed wake, a say is kept only when its address is `actor` and its actor id is the one who was addressed. On a table or place wake, a say is kept only when it is `table`, or `place` naming the speaker's place. A queued wake, or a sit wake for a non-empty queue, has no addressed actor. The host censuses the PC in `presence.actors_present` the way a table wake censuses the speaker, and keeps a say only when it is `table`, or `place` naming that PC's place. If that list has no PC, the host skips the model, submits no say, and still runs the commit step. One model reply may submit at most two says and one intent, after that rule. A say the rule rejects does not consume the cap. An intent whose move is not in the menu is dropped before the cap.

Speech already on the tape at sit is not answered. High water starts at the current last seq. A queue that is already waiting still commits.

The POST body is the bounded prompt: the last 16 says this process has handled, the new player lines, the move menu, the stripped views, and roll faces on a follow-up. A restart remembers nothing. A player view that is only a room census drops `known_facts`, `messages`, and `unknown_mentioned`. An NPC view keeps each fact's `id` and `text`. Every fact `body` is dropped. The prompt does not receive a canon or a cast body. The tape file stays on disk under the pack. The POST does not carry the tape file, `now.yaml`, or a cast file.

## Firewall, lint, and digest

`allows` means the fact is known, or it is not hidden and it is on the viewer's place, or it is not hidden and it is on a co-located owner's cast or place. Hidden facts never leak through place or cast prose.

`perceive` is budgeted. The budget is 800 tokens unless the caller passes a budget. `lint` scans Markdown bodies for hidden fact text. `strip_body` deletes the body, leaves frontmatter, and the pack still ticks. Those functions stay in firewall, perceive, digest, and lint.

Load fails with `SCHEMA` if `places.actors` is edited apart from `actors.place`. Emit rebuilds occupancy from `actors.*.place`.

Faces of a roll go to the GM socket only. The player socket gets `{"kind":"check","success":true|false}` or `{"kind":"refused","code":"..."}`. No faces. The tick JSONL lists `resolve_check` and `apply_effect` without faces. A refused tick records `rejected[].reason` as the token: `BUDGET` lowercased, `empty_perceive` as that word, every other token uppercase.

## Non-goals

Still not built, and not implied by a missing sentence above:

- A cargo feature named `llm`, or an NPC `from` on the tape.
- Parsing speech into `known_facts`, or a whisper that grants a fact.
- A non-loopback bind, accounts, or a second GM.
- An ack cursor, or a per-client outbound queue whose purpose is to make a host event start a host turn.
- `apply_effect` or `patch_entity` on the sitting.
- Studio tools that write `state/`.
- Changing the player wire kinds `refused`, `check`, and `say`.
- WebSockets, `--serve`, or a database.
- A second spec, an archive copy of an older spec, or a banner that keeps the older text in the worktree. Tags stay. The files do not.
- Publishing a crate named `spook`.

The player line grammar does change. The check wire object does not. That split is intentional.

## Threat list

| How a second writer appears | Countermeasure |
|---|---|
| Client patches YAML | The play door queues an intent or appends speech. The CLI has no path that opens `now.yaml` for a client write. |
| GM calls `apply_effect` | The tool is not a sitting handler. Effects come from expand inside a `Txn`. Only `tick_commit`, `commit_presence`, and `migrate_pack` rename files. |
| A summarizer writes memories | No LLM crate. Memory lines are written inside the commit, from a `memory` effect, after `allows`. |
| Speech copied into `known_facts` | The tape is not an input to expand. No whisper. Inbound model text is not a field on `Intent`. |
| Host digest reused as the player resume | `idle_digest` takes `DigestViewer::Host` or `DigestViewer::Player`. Different rows. |
| Hand edit of `now.yaml` | `HAND_EDIT` until `spook migrate`. |
| Replay applies JSONL effects as the new state | Replay does not install the log as `Now`. It re-expands the recorded intents, calls `resolve_check` again from the recorded seed, and then calls `apply_effect`. |
| Two ticks in flight | One lock around the tick run and `commit_turn`. The second caller gets `COMMIT_CONFLICT`. Tick code is synchronous on the shard thread. |
| `patch_entity` writes canon and the live shard | Not a sitting tool. A canon byte change mismatches `pack_rev` until migrate. |
| New effect class in a handler | `ILLEGAL_CLASS` at the parse boundary. `EffectClass` has seven variants. |
| A meter whose stored pair is outside the definition | `SCHEMA` at load. Apply clamps when the sum fits in `i64`. `OUT_OF_RANGE` leaves the record unchanged when the sum does not. |
| `perceive` with no `as`, or `as=gm` | `ViewerId` has no GM variant. Sitting perceive rejects anything but `player:` and `npc:`. |
| `places.actors` edited apart from `actors.place` | Load fails `SCHEMA`. Emit rebuilds occupancy from `actors.*.place`. |
| Feature `llm` | No such feature. |
| Tokio task per shard inside the kernel | Kernel and tick cannot name the runtime. The scan stays. |
| `rand::thread_rng` or a hand-rolled die | `die_face` is Blake2s. One copy, in `check.rs`. |
| Markdown body used as a stock count or a hidden fact | `strip_body` and `lint`. |
| A second snapshot beside `now.yaml` | Forbidden. One emit path. |
| Non-loopback bind, accounts, second GM | `NOT_IN_SPEC`, no account store, `SESSION_FULL`. |
| Hidden fact in a perceive body | `allows` plus lint. |

Privacy: the pack is local. There is no account, no connection secret, and no telemetry. The tape holds table speech on disk under the pack. That is the product log. The tape file stays on disk. `spook ai-gm` sends table text off the machine only as the body of its chat-completions POST: the last 16 says this process has handled, the new player lines, the move menu, the stripped views, and roll faces on a follow-up.
