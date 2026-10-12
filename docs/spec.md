# Spookery referee spec

This file is the only normative spec. A rule it does not state is unspecified.

The engine is open source (AGPL-3.0). Runtime code, the tests of that code, and repository tooling and scripts are AGPL-3.0-or-later. Documentation, schemas, and packs are CC BY 4.0.

## Status and scope

**Non-normative.** Referee law stays in the fourteen rules below. Rule 13 remains the seat law.

No releases yet. No compatibility guarantees.

- A generic RPG referee engine. Authors write a pack and host it like a private server.
- One GM seat (AI or human) and 1 to 3 player seats. Each player sits a distinct pack PC. A 4th player gets `SESSION_FULL`. A hello for a PC another seat holds gets `SEAT_TAKEN`. That seat model is built.
- Players walk around and take basic actions.
- Packs are written and updated by people and by coding agents (Grok, Claude Code, Cursor), so the format must be machine-checkable: every refusal has a file, a location, a stable code and a fix hint. `spook check <pack>` prints those in plain text. `spook check --json <pack>` prints the same diagnostics as one JSON array. Generated schemas are not built yet (#29).
- The rules primitives must be able to express a 5e-style game with no 5e-specific code. Generic engine, not a 5e replacement, and no SRD text. `packs/sample-d20` is that sample. It plays a fixed-damage fight against a literal difficulty. It does not yet express a difficulty taken from the target's defense meter, rolled effect amounts, advantage-style rolls, a modifier from equipment, a condition when hit points reach zero, or consume an item (drink checks for the tonic but never uses it up, so it heals forever).

This spec records one writer, the crate boundary, the threat list, loopback and the seat cap, and the rule that a host monitor is not shard behavior.

## Referee rules

These fourteen rules are the referee, in this order. Later sections are the detail. A sentence in another document does not add a fifteenth rule.

1. **Import boundary.** `spookery-kernel` and `spookery-tick` do not link a runtime, a database, or a model SDK.
2. **`!Send` `Txn`.** One `std::thread` owns `Now`. The tick is not spawned on a runtime.
3. **Atomic commit.** `now.yaml` is renamed last. A crash cannot apply a partial tick.
4. **Closed effect, one apply.** Seven effect classes. `apply_effect` is the only mutator of world numbers.
5. **Firewall.** `firewall::allows` is the visibility test. A hidden fact does not leak through place or cast prose.
6. **Deterministic tick.** A row's literals are filled before the apply loop. Replay re-expands the recorded intents and calls `resolve_check` again from the recorded seed. It also replays every journaled commit since the base: a presence commit restores that presence, and a migrate commit restores `pack_rev`.
7. **Canon versus `Now`.** Canon is frozen. `Now` is the live snapshot.
8. **Action-row loader.** A moves file is names and rows. A field is a literal, or the player token on a player row. A new verb is a new row in YAML.
9. **Seeded checks.** `die_face` is Blake2s. One copy, in `check.rs`. The intent's own actor resolves the check, PC or NPC. Another actor does not. `difficulty` applies only when the row sets `check` and no opposing total applies. The total is `success` when it reaches the target, `partial` when the row sets `partial_at` and the total reaches that threshold but not the target, and `miss` otherwise. A row without `partial_at` is `success` or `miss`. An opposing total replaces `difficulty`, and that roll stays `success` or `miss` even when the row sets `partial_at`, because `partial_at` is compared with the row's own `difficulty`.
10. **Eligibility and habits.** Both run before a kernel habit intent exists. An ineligible NPC produces no habit intent. A player-sourced intent is not a habit intent.
11. **Failure tokens.** Each token has one meaning. `NOT_OWNER` and `LIVE_ONLY` are not tokens.
12. **Speech versus queue.** Speech appends to the tape. It is not a row, it does not enter the intent queue, and it does not tick.
13. **Seat law.** One GM seat and up to three player seats. Each player seat holds a distinct pack PC. The cap of three is the engine's, not a pack setting. A fourth player hello is `SESSION_FULL`, checked before any other player-hello refusal. A hello for a PC another seat holds is `SEAT_TAKEN`. A second GM is `SESSION_FULL`. The seat's `tools/list` is the menu.
14. **Rounds.** With no GM, each player line queues and commits its own tick. With a GM seated, every player's lines queue, and one `commit_turn` commits them in queue order.

## One writer

The shard thread is the only writer of `Now` and of `Seats`. One `std::thread` owns `Now`. `Txn` stays `!Send`, with `PhantomData<Rc<()>>` as in `crates/kernel/shard.rs`. No `tokio::spawn` of a tick.

The queue is a `Vec` of intents on the shard thread. It is ephemeral. A crash drops every uncommitted line. A crash cannot apply a partial tick. The queue is not a field of `now.yaml`. It is not a side file that recovery has to remember.

`Txn::begin` clones `Now` once into `staging`. That is the only clone. `commit` moves `staging` into `Shard.now` after `write_commit` returns. It does not clone again.

`expand` takes `&Canon`, `&Now` (the staging borrow), and `&Intent`. It does not hold `Shard`. `apply_effect` takes `&mut Staging`. `Staging` borrows `Txn`'s staging, the typed key set, and the active intent, so it is `!Send`. The tick does not hold `&Shard` and `&mut Txn` in a way that aliases `Now`. Checks read `current` off the pre-loop `frozen` snapshot, the same `Now` expand uses. They do not read live staging. The item precondition reads the actor's inventory before queue. `stage_tick` receives `&mut Txn` and the queue snapshot: the `Vec` of intents is moved out for the tick and put back only if the tick is abandoned. `Shard` fields are private.

`append_session` takes the session lock and never a `Txn`. `commit_presence` takes the shard lock and never a `Txn`.

`Seats` is one GM seat and up to three player seats, plus a process-local session counter. Each player seat is one live socket bound to one pack PC. The cap of three is a constant in the seat code. It is not a pack setting. `Seats` is not a second presence document. The durable source of who is at the table is `Now.presence`, and that struct is only `humans` and `actors_present`. Session ids are not in it. The shard thread is the only writer of both `Seats` and `Now`. A player hello or drop that changes `humans` or `actors_present` calls `commit_presence` once before the handler returns. A drop releases only that seat. A GM hello does not change those fields and does not write `now.yaml`.

Invariant, checked by a debug assert on the shard thread: `presence.humans` equals the number of held player seats, and `presence.actors_present` is exactly the set of those PCs. The assert is debug-only. It is not a library `unwrap`. Startup repair is `clear_crashed`. `listen` binds, then the shard thread opens the shard and, if `humans != 0` or `actors_present` is non-empty while `Seats` is empty, `commit_presence` writes zeros.

## Crate boundary

Three crates, all under `crates/`. `crates/kernel` is `spookery-kernel`. `crates/tick` is `spookery-tick`. `crates/spook` is package `spook`, library `spookery`, binary `spook`. Protocol, the player client, and the model client are modules of that library: `protocol`, `cli`, and `ai`. A one-crate collapse is not the layout. The `spook` package may link a runtime. The `ai` module may link an HTTP client.

| Crate or module | May link | Must not link |
|---|---|---|
| `protocol` | `serde`, `serde_json` | `tokio`, `rmcp`, `reqwest`, `spookery-kernel`, `cli`, `ai` |
| `spookery-kernel` | `serde`, `serde-saphyr` `=1.3.0`, `serde_json`, `blake2`, `thiserror`, std | `tokio`, `rmcp`, `rmcp-macros`, `sqlx`, `postgres`, `redis`, `jsonschema`, `reqwest`, any model SDK, `spookery-tick`, the `spook` crate |
| `spookery-tick` | `spookery-kernel` | `tokio`, `rmcp`, `reqwest`, `serde_json` as a direct dependency (it uses `kernel::codec`), MCP, any model SDK, the `spook` crate |
| `cli` | `protocol`, `spookery-kernel` | `ai`, `reqwest`, `rmcp`, `tokio` |
| `ai` | `protocol`, `spookery-kernel`, an HTTP client, a runtime | `spookery-tick`, `cli`, `rmcp`, any model SDK |
| `mcp` | `tokio`, `serde_json` | `reqwest`, `cli`, `ai` |
| `audit` | `spookery-kernel`, `spookery-tick`, the player perceive renderer | `tokio`, `rmcp`, `reqwest` |
| bin `spook` | `spookery-kernel`, `spookery-tick`, `tokio`, `rmcp`, `socket2`, `serde_json`, and its `protocol`, `cli`, `ai`, `mcp`, and `audit` modules | `reqwest` outside `crates/spook/src/ai/`. Seat code that calls `cli` or `ai`. Business rules that belong in the kernel. A second tick implementation. |

`serde_json` in the kernel is allowed. The import scan does not forbid it. The scan stays a source scan, not a `Cargo.toml` parser. It reads every runtime file under `crates/kernel`, `crates/tick`, and `crates/spook/src`. `crates/kernel` and `crates/tick` must not contain the byte strings `tokio`, `rmcp`, `sqlx`, `postgres`, `redis`, `reqwest`, `langchain`, `openai`, `anthropic`, or `ollama`. `reqwest` may appear only under `crates/spook/src/ai/`. `protocol` must not contain `tokio`, `rmcp`, `reqwest`, `spookery_kernel`, `crate::cli`, or `crate::ai`. `cli` must not contain `tokio`, `rmcp`, `reqwest`, or `crate::ai`. `ai` must not contain `rmcp`, `crate::cli`, or `spookery_tick`. `seat` must not contain `crate::cli` or `crate::ai`. `mcp` must not contain `reqwest`, `crate::cli`, or `crate::ai`. `audit` must not contain `tokio`, `rmcp`, or `reqwest`. A comment in a scanned file that names a banned crate is still a failure. This spec is where that ban is written. There is no cargo feature named `llm`. An optional cargo feature may be named `ai`.

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

A moves file is still a name or a row. The tag is `moves`. New fields are optional. A misspelled field is rejected because unknown fields are denied.

A row has:

- `audience`: `player` or `habit`. Omitted audience, or `system`, is `SCHEMA`. A row is not implicitly player-facing.
- one token: `none`, `actor`, or `place`
- one subject: `speaker` or `named`
- `effects`: templates that run on every outcome
- `success`, `partial`, and `failure`: templates that run on that outcome. `failure` is the miss list
- optional `check`: an id that must exist in `checks.yaml`, or load returns `SCHEMA`
- optional `difficulty`: `i64`, default 0, used only when `check` is set and no opposing total applies
- optional `partial_at`: `i64`, strictly below `difficulty`. It requires `check` and a `partial` list. A `partial` list without `partial_at`, a `partial_at` without that list, a `partial_at` at or above `difficulty`, or a `partial_at` without `check`, is `SCHEMA`
- optional `item`: an actor inventory precondition

`classify` queues a row only when `audience` is `player`. A `habit` name is `NOT_IN_SPEC` on a player line and is not queued. The GM intent tool queues a `player` row for a player or an NPC, and a `habit` row only when `actor_kind` is `npc`. A player actor queuing a habit row is `NOT_IN_SPEC`. `source` on that tool stays `player`. A cast habit that names a `player` row is `SCHEMA` at load. Empty-table gates still apply to a kernel habit intent. They do not apply to a player-sourced intent.

Load rejects a duplicate name, an empty name, an empty `item`, the flag key `busy`, a row named `say`, an unknown class, a search word, a template whose keys do not match its class, and any unknown shape. `say` is the speech line, so that name is not a row. `travel` is not special-cased in Rust. A travel row is whatever templates it has, and `apply_location` enforces the range. The docks grit check is a `grit` row. `attempt grit 3` is not a special parser. The player wire object is `{"kind":"check","outcome":"success"|"partial"|"miss"}`.

**Yield.** The key `effects` may be present and empty only when `success`, `partial`, and `failure` are absent. That row is a yield. Omitted `effects` together with omitted outcome lists, on a row that set `token` or `on`, is `SCHEMA`. A yield with no `check` expands to zero effects and is still an applied intent if it was allowed. A yield with a `check` rolls, emits the check notice, and writes no world key. Nothing matches on the name `wait` or `attempt`. `wait` is a yield row (`effects` present and empty). An NPC habit that does not list `wait` does not emit it. Absence of an on habit whose row expands means no kernel intent.

**Fields.** A template field is a literal, except `to: token` on a player travel row. That token is the queued place. Search words (`here`, `first_rate`, `rate`, `first_drift`, `toward_target`, `first_known`, `first_listener`, `keep`, `first_due_note`, `first_owned`) are `SCHEMA` at load. There is no expression language. Adding a verb is a new row in YAML.

Stock names `place`, `sku`, and `delta`. `apply_stock` still refuses a place outside range, an absolute delta above `stock_rate`, or a negative result. Message names `id`, `to`, `place`, `fact_ids`, and `text`. Clock names `clock` and `delta`. Location `from` is the actor's live place and is not a YAML field. A meter template is a literal key and write. There is no stock maximum in the docks profile.

Bindings and the outcome list are different steps.

1. Habit selection picks a row before the apply loop. Expand fills that row's literals against the staging `Now` it sees, including `from` taken from the actor's live place. A row that does not expand is not the kernel intent. Player travel binds the typed place.
2. Inside the per-candidate loop, `resolve_check` runs if the row has a `check`. The viewer is that intent's actor. A PC or an NPC may roll for themselves. A viewer who is not the actor is `UNKNOWN_KEY`. An unknown id is `UNKNOWN_KEY`. Templates do not run. The die cursor does not move.
3. After that roll, the selected templates are `effects`, plus `success` on success, `partial` on partial, and `failure` on miss. No roll means `effects` only. Those templates are filled from the row, not by choosing a new target. The result is a `Vec` of `EffectClass`.
4. Expand wraps that vec with `as_viewer` and `intent_id`. That post-roll vec is what `apply_effect` sees and what the tick JSONL stores. `parse_recorded_line` keeps the intents and does not read `effects`. `run_recorded_tick` expands those intents again, calls `resolve_check` again from the recorded seed, and then calls `apply_effect`. Replay does not install the log as `Now`. A presence record restores `presence` and `state_rev`. A migrate record restores `pack_rev` and `state_rev`. Replay applies those commits. It does not install them as effects. The names `need` and `resource` are not variants.

A vec built before the roll cannot be the success list, the partial list, and the failure list at once. The post-roll value is the effect vec. Habit rows may have `success`, `partial`, and `failure`. Outcome lists are not limited to player rows.

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

Kernel candidates and player candidates in the tick are an `Intent` plus bindings. The player queue on `Shard` stores `Intent` only. The post-roll effect vec is tick-local until the JSONL write. Replay keeps the recorded intents and expands them again. It also applies presence and migrate records as their commits.

Priority, computed before the roll, is player before NPC before system, then PC-owned clocks, claimed targets, then `shuffle_key`. `shuffle_key` is Blake2s of shard, tick, and actor. "PC-owned clock" uses the clock id on the expanded effect. "Claimed" means the token actor is in `actors_present`, or the token place contains a present PC, or a message's `to` hits `actors_present`.

Library paths return `Result`. `run`, `tick`, `status`, and `migrate` print `Reason::token()` and exit 1. `spook check` prints one diagnostic line per problem, or one JSON array when `--json` is set. `spook replay`, `spook verify`, and `spook audit` print their own lines.

## Habits

A tick profile has `habits`, a list in file order. The idle runner produces at most one kernel intent per eligible NPC. It takes the first habit that is on and whose first expanding row is an `audience: habit` row. Later habits wait. A habit that is always ready starves the ones under it. No such row means no kernel intent.

A habit is on when every gate it sets is true. `runs_while_empty` is required. When `humans == 0`, only a habit with `runs_while_empty: true` can be chosen. When someone is seated, that flag does not block the runner. `from_hour` and `to_hour` are optional and come together. Each is an integer in `0..24`. The hour is `(world_time / 60) % 24`, read after the tick has advanced world time. `from < to` means `from <= hour < to`. `from > to` wraps midnight. `from == to` is never on. The check reads the hour. It does not write at the window's edge. `clock` and `at_least` are optional and come together. The habit is on when that clock exists, is not paused, and `value >= at_least`. A missing clock means the habit is off. `unless_message` is optional. The habit is off when a message with that id is already in `now.messages`.

An empty id, a duplicate id, an empty row list, a half-set hour pair, a half-set clock pair, or an hour outside `0..24` is `SCHEMA`. A habit row name that is not an `audience: habit` row is `SCHEMA`.

`audience` is `player` or `habit`. The player parser queues only `player`. Habit selection queues only `habit`.

Eligibility runs first. The NPC must be an NPC, the profile must say eligible, the NPC must not be `busy`, and no PC may be in that place. The NPC must not have already acted this tick, and the idle-day or live-hour cap must be unspent. An ineligible NPC produces no habit intent. The shop stays still while a player stands in it because that NPC is not eligible. `allowed` checks those gates only for `source: kernel`. A player-sourced intent is not checked against the hour, the clock, `unless_message`, `runs_while_empty`, or eligibility. Apply still checks range, stock rate, the firewall, paused clocks, and the other effect rules.

NPC perceive includes `habits`, the ids that are on for that NPC, in file order. Player perceive sends an empty list. Queueing an intent does not consult that list.

`tick.yaml` pause is `named_plot_clocks`. `now.pause` stores that one view. Plot clocks with `requires_presence` are paused while `humans == 0` and the policy is `pause_when_empty`.

`tick.yaml` may set `idle` to `on` or `off`. Omitted means `on`. Any other value is `SCHEMA`. `on` wakes an empty table. `off` leaves an empty table still: the runtime does not arm the wake timer, a wake does not tick, and a later wake or a reopen does not catch up. `last_tick_at` and `world_time` stay unchanged while the table is empty. Seated play does not read `idle`. With no GM, a player line commits its own tick. With a GM, `commit_turn` commits the queue. `spook tick` is a manual tick and does not read `idle`. Turning `idle` back to `on` after a long `off` period applies the same catch-up cap as any other long absence.

The empty-table knobs sit beside `idle`. `tick_every_real_seconds` is the real seconds between wakes. `idle_minutes_per_tick` is the world minutes one empty tick advances. `idle_catchup_max_ticks` caps how many ticks a catch-up writes. `idle_max_world_hours_per_real_hour` caps world hours gained per real hour of absence. `idle_acts_per_world_day` caps habit acts on an empty day. `tick_every_real_seconds: 0` and `idle_minutes_per_tick: 0` are `SCHEMA`. A seated catch-up is one tick.

Kernel habits do not emit speech. A player world line while `humans == 0` is refused at queue time with `TABLE_EMPTY`. A five-name denylist is not part of habit selection.

## Failure tokens

`Reason` is one `thiserror` enum in the kernel. It stays fieldless and `Copy`. `Display` and `token()` are the wire words. There is no parallel code enum. JSONL lowers only `BUDGET`, plus the log word `empty_perceive`. `run`, `tick`, `status`, and `migrate` print the token and exit 1 for an engine refusal. `spook check` prints the diagnostic line instead of the bare token. A usage error prints clap's message and exits 2. The player socket sends `{"kind":"refused","code":...}`.

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

Tokens that already have one meaning and stay: `ILLEGAL_CLASS`, `ILLEGAL_KNOWLEDGE`, `OUT_OF_RANGE`, `BUDGET`, `PAUSED_CLOCK`, `UNKNOWN_KEY`, `PACK_REV_MISMATCH`, `UNKNOWN_MOVE`, `UNSCOPED_PERCEIVE`, `COMMIT_CONFLICT`, `HAND_EDIT`, `SCHEMA`, `NOT_IN_SPEC`, `SESSION_FULL`, `SEAT_TAKEN`, `MISSING_ITEM`, `LINE_TOO_LONG`.

`LINE_TOO_LONG` is a hello, a player line, or a GM TCP line over its cap.

`SEAT_TAKEN` is a hello for a PC another player seat already holds. `SESSION_FULL` is a fourth player hello, or a second GM. The fourth-player check runs before the held-PC check.

`NOT_IN_SPEC` stays for a non-loopback bind, an unknown verb, `include_tick: false` with a pending queue, and an unknown tool. It is not the expand result for `give`. `MISSING_ITEM` stays the one meaning of a failed `item` precondition. `ILLEGAL_CLASS` is an unknown class at `parse_effect`, including `need` and `resource`. A field-shape error on a known class is `SCHEMA`. A moves-file unknown class is `SCHEMA`. `EffectClass` has seven variants.

## Commit

`write_commit` is the only rename protocol for world bytes:

1. Copy each live target that already exists into `state/.txn/prev`, and fsync the copies and the `prev` directory.
2. Create `state/.txn/COMPLETE`, and fsync the file and the `.txn` directory.
3. Write each new body to a temp file, fsync, and fsync the parent.
4. Rename side files first (`log/tick-NNNN.jsonl`, `log/presence-NNNN.json`, `log/migrate-NNNN.json`, `log/base.yaml`, `memories/*.jsonl`).
5. Rename `now.yaml` last.
6. Fsync `state/`. Remove `.txn`. `state_rev` was incremented in memory before the bytes were emitted, and it hits disk only in that last rename.

Every commit that writes `now.yaml` also writes one journal record in that same plan. A tick writes `log/tick-NNNN.jsonl`. A presence change writes `log/presence-NNNN.json` with the new presence and the `state_rev` after the commit. A migrate writes `log/migrate-NNNN.json` with `pack_rev` before and after and the `state_rev` after the commit. `NNNN` on a tick file is the tick index. `NNNN` on a presence or migrate file is the `state_rev` after the commit. Each record carries `now_hash`, the Blake2s of the `now.yaml` bytes that commit wrote. `now.yaml` is still renamed last, so a crash cannot leave it ahead of its record.

The first commit on a pack with no `log/base.yaml` writes that file in the same plan. Its bytes are the pre-commit `now.yaml`. `migrate` writes `log/base.yaml` again, in the same plan, as the post-migrate `now.yaml`, and the migrate record marks that point. Older records stay on disk. Replay from the base applies only the records after it, because those older ticks do not replay against the new canon. `state/log/base.yaml` is replay input. Nothing reads it as the live save. `emit_now` is still the only path that produces `now.yaml`.

`recover` keeps three shapes. No `COMPLETE` means delete `.txn` and leave the live tree. `COMPLETE` and `live.state_rev == prev.state_rev` means restore `prev`. `COMPLETE` and `live.state_rev == prev + 1` means delete `.txn` and keep live. Then drop journal records whose `state_rev_after` is above live, drop `base.yaml` when its `state_rev` is above the live save, and drop memory lines whose `tick_index` is not in a live log.

`commit_presence` writes `now.yaml` through the same `write_commit`, with the presence journal record in that plan. Presence is `humans` and `actors_present` only. When those two fields are unchanged after canonicalize, it writes nothing and does not increment `state_rev`. When either field changes, it increments `state_rev` by 1 and then emits. It does not open a `Txn`, does not write a tick log, and does not advance clocks. A session id is not an input.

`migrate_pack` is the only writer that changes `pack_rev`. It goes through `write_commit`, writes the migrate journal record and the new base in that plan, and then appends one studio line. Replay does not read the studio line. A hand edit of `now.yaml` that is not `spook migrate` remains `HAND_EDIT` on the next tick. Studio tools do not write `state/`.

`write_commit` has no fault parameter. The crash hook is not an environment variable and not `process::abort`. No production path reads `SPOOK_CRASH_AFTER`. Dropping the `Txn` after `apply_effect` without calling `write_commit` leaves no `state/.txn`, and leaves `state_rev` and stock at the bytes from before `begin`. Recovery on the three directory shapes above is the other half of the same proof: an incomplete `.txn` leaves the previous `now.yaml`, `COMPLETE` with an unmoved `state_rev` restores `prev`, and `COMPLETE` with the new `state_rev` keeps the new `now.yaml` and deletes a journal record and a memory line that do not belong to it.

Replay byte-compares `now.yaml` produced by this kernel, for every journaled commit since the base. It does not pin a frozen docks hash. A pack that has never committed has no base and no journal. `spook verify` then compares `now.yaml` to itself and reports zero records.

## Speech

`say` is speech. It is not a row. The tape is not an input to expand. Text is not parsed into `known_facts`. There is no whisper that grants a fact.

A player line is `say` plus the words. The first word is the address when it is `place`, or an actor id, and more words follow. `place` is the speaker's live place. An actor id is that actor. Any other line is address `table`, and every word is the text. A line of one word is table text, including when that word is `place` or an actor id. An empty `say` is `SCHEMA`.

Speech appends when `humans` is 0. The append takes `state/.session.lock`, writes one object, and fsyncs. That path uses no `Txn` and does not bump `state_rev`. The object keeps `seq`, `from`, `address`, `place`, `actor`, `recipients`, `text`, `tick_index`, and `world_time`. `from` is `player:{id}` or `gm`. A typed `from` on the GM tool is `SCHEMA` and appends nothing. An NPC `from` is not representable. The speech parser has no NPC speaker.

Recipients are seated player actors. `table` includes every seated player. `place` includes the seated players in that place. `actor` includes that actor when they are a seated player. The speaking player is included. An NPC stored in `actor` is on the tape and is not a recipient.

The GM tool is the same act. `address` is `table`, `place`, or `actor`. `place` is required when the address is `place`, and `actor` is required when the address is `actor`. `actor` must name an actor. The other field is absent.

`read_session` returns the tape lines after a sequence number, up to a limit. The limit defaults to 64 and is clamped to 256. The file hard cap is 1 MiB. `append_session` reads the tail for the last `seq` and returns `OUT_OF_RANGE` when the next append would pass 1 MiB. A line that does not parse is `SCHEMA` for the whole read, not a skipped line. `session_status` with `since` uses this window.

## Rounds, seats, and profiles

With no GM, the runtime queues and commits one live tick of `live_minutes_per_tick` (default 5). With a GM seated, the line waits. `commit_turn` requires `include_tick: true`. An empty queue is a no-op and writes no tick file. `include_tick: false` with a non-empty queue is `NOT_IN_SPEC`.

One wake: when `idle` is `on` (the default), the runtime sleeps `tick_every_real_seconds`, reset when a tick actually advanced, and posts a wake. The shard thread wakes the live shard. A second open of the pack on that wake is not the scheduler. When `idle` is `off`, that timer is not armed, an empty table does not tick, and it does not catch up on a later wake or a reopen. `last_tick_at` and `world_time` stay unchanged while the table is empty. Turning `idle` back to `on` after a long `off` period applies the existing catch-up cap. A seated table still commits its own ticks, and `spook tick` still ticks.

The seat's `tools/list` is the menu. Player input is the move menu plus `say`, with no AI parsing in v1 (see #54).

Sitting perceive allows `player:` and `npc:`. A missing `as`, or `as=gm`, is `UNSCOPED_PERCEIVE`. Perceive calls the perceive budget after that check. `firewall::allows` is the shared visibility test.

An unknown sitting tool is `NOT_IN_SPEC` and writes nothing. `apply_effect` and `patch_entity` are not sitting handlers. Effects come from row expansion during the tick.

## Human GM

`spook gm <pack>` sits the GM seat and dials `--addr`, which defaults to `127.0.0.1:9737`. It starts neither `spook run` nor `spook play`. `spook gm` and `spook ai-gm` are two clients of the same single GM seat. A second GM is `SESSION_FULL`.

`spook gm` with no pack, or with any extra argument, is a usage error before a dial. A refused hello prints a sentence and the token on stderr and exits 1. `SESSION_FULL` prints `The table is full. [SESSION_FULL]`. A shard that is not listening exits 1 and leaves the pack bytes as they were.

stdin is one command per line, and each command calls one seat tool. An empty line sends nothing. An unknown command prints the command list and sends nothing.

`status` calls `session_status`. It prints `Seated: <names>` or `Seated: nobody`, then `Queue:` and one `<name>: <move>` line per queued row, or `Queue: empty`. `look <actor>` calls `perceive` and prints that view the way `spook play` prints a perceive. The actor is `player:<id>`, `npc:<id>`, or a pack id. A pack PC is `player`. A pack NPC is `npc`. An id listed in `actors_present` is `player`. An actor the client cannot place prints `That is not in the world. [UNKNOWN_KEY]` and sends nothing. `say <text>` speaks to the table. `say @<place> <text>` speaks to that place. `say #<actor> <text>` speaks to that actor. The say tool sends no `from`, so the GM speaks as the GM. `queue <actor> <move> [target]` calls `intent` with `source` `player` and that actor's kind. When the move's token is `actor`, the target object is an actor. Any other typed target is a place. `commit` calls `commit_turn` with `include_tick` true. It prints `Changed:` and one line per diff summary, or `Nothing changed.`, then a roll line for each check on the result. `help` prints the commands. `quit` leaves the seat.

Events print as they arrive, as text, with no protocol JSON. A say is `<name>: <text>`. A queued line is `<name> queued: <move>`. A check is `Roll: <faces>, +<stat> = <total> vs <target>, <outcome>`, where `<outcome>` is `success`, `partial`, or `miss`. Faces are joined with ` + `. A negative stat keeps its minus sign. The line uses the faces, stat, total, target, and outcome carried on the check event. A check already printed from `commit` is not printed again. Names drop a leading `player:` or `npc:` prefix, turn `_` into a space, and capitalize the first letter, so `player:pc_rowan` prints `Pc rowan`. The speaker `gm` prints `GM`.

A tool refusal prints a sentence and the token on stderr, using the same sentences as `spook play`, and the client stays up.

`spook gm --raw` prints each socket frame as one JSON line on stdout. A refusal prints that line and its token on stderr.

`spook new <dir>` writes a minimal pack into that directory and prints the next commands on stdout: `spook check`, `spook run`, and `spook play`. It creates `<dir>` when that path is missing and the parent directory exists. A missing parent exits 1, prints `<parent> does not exist` on stderr, and creates nothing. An existing non-empty directory, or a path that is not a directory, exits 1, prints a sentence on stderr, and writes nothing. A write that fails removes what it created: a directory that was already empty stays empty, and a directory this command created is removed. The written save has `state_rev` 0 and a `pack_rev` computed from `world/`, `cast/`, and `rules/`. The command writes only under `<dir>`. It does not take the lock or bind a port.

`spook check <pack>` reads the pack and prints one line per problem. It loads `pack.yaml`, `world/`, `cast/`, `rules/*`, and `state/now.yaml` when that file is present, compares `pack_rev` with the canon hash, and scans Markdown bodies and message template text for hidden-fact text. It does not take the lock, bind a port, recover a transaction, or write under the pack. A clean pack prints `no problems` on stdout and exits 0. Any problem exits 1. Problems go to stderr as `file:line: message (CODE)` when a line is known, or `file: path: message (CODE)` when the location is a key path such as `moves[2].audience`. A missing file uses the key path `schema`. The message is plain English and says how to fix the problem. The code in parentheses is one of the codes in the table below. `run`, `tick`, `status`, and `migrate` still print only the token.

`spook check --json <pack>` runs the same checks. It prints one JSON array on stdout and writes nothing else there. Each element is a diagnostic with `file`, `line`, `column`, `path`, `code`, `message`, and `hint`. `line` and `column` are 1-based when present. An unknown `line`, `column`, `path`, or `hint` is null. The array is ordered by `file`, then `line`, then `path`. A null sorts before a value. A clean pack prints `[]` and exits 0. Any problem exits 1.

The codes `spook check` can emit are stable. The message and the hint may change.

| Code | Meaning |
|---|---|
| `SCHEMA` | The loader rejected the file's shape. |
| `NOT_IN_SPEC` | A file the loader requires is missing. |
| `PACK_REV_MISMATCH` | `pack_rev` is not the hash of `world/`, `cast/`, and `rules/`. |
| `HAND_EDIT` | `state/now.yaml` is not the canonical save text. |
| `HIDDEN_FACT_IN_BODY` | A hidden fact's text appears in a Markdown body. |
| `HIDDEN_FACT_IN_TEXT` | A hidden fact's text appears in a message template's `text`. A move row is not a body, so this is not `HIDDEN_FACT_IN_BODY`. |

`spook replay <pack>` replays the journal from `state/log/base.yaml` into a temp directory, deletes that directory, and prints `state_rev <r>, <n> records`. `spook verify <pack>` reads `now.yaml` once, replays the journal up to that `state_rev`, and byte-compares. Later records are ignored. A match prints `verified <n> records, state_rev <r>` and exits 0. A mismatch exits 1 and prints the first divergent record and the first differing `now.yaml` line. `--from <dir>` uses that directory's `state/now.yaml` as the base instead. Both commands write nothing under the pack, take no lock, and bind no port. A pack that has never committed has no journal and no base. Verify treats `now.yaml` as its own base, so the check passes with zero records. A path with no `pack.yaml` or no `state/now.yaml` is not a pack, and both commands print `NOT_IN_SPEC` and exit 1.

## Loopback

A fourth player hello is `SESSION_FULL`. A hello for a PC another seat holds is `SEAT_TAKEN`. A second GM is `SESSION_FULL`. A refused hello gets one refusal line. The seat then closes its write side and reads and discards whatever the client already sent, until the client hangs up or 2 seconds pass, so a client that sent more than its hello still gets the refusal.

A hello line and a player line are at most 8 KiB, not counting the newline. A longer line is `LINE_TOO_LONG` and the socket closes. The server does not keep more than that cap. Invalid UTF-8 on a hello or a player line is `SCHEMA`, not a replacement character. A player connection stays up after that player-line refusal. A hello that is `SCHEMA` or `LINE_TOO_LONG` is refused and closed. A socket that has not sent a complete hello within 10 seconds is closed, and nothing is committed.

A player connection may send 5 lines per second, and it may burst to 10. When that allowance is used, the connection waits until the next line is allowed before it reads again. The line is kept. The wait is on that connection alone, so a flood does not delay another seat. The GM seat is not rate-limited. One GM is the trusted seat, and its commits already run one at a time on the shard thread.

A GM TCP line, after the hello, is at most 1 MiB, the same cap as an HTTP POST body. A longer line ends that GM session, releases the seat, and the refusal line is `LINE_TOO_LONG`.

Each player socket, the GM TCP socket, and the HTTP GM session has an outbound queue of 256 frames. The shard thread does not wait on a send. A full queue means that client is not reading. That connection is dropped and its seat is released. A dropped GM stops reading, so a later tool call does not run. Other seats keep their frames. This queue does not start a turn.

The bind does not grow past loopback. `--addr 127.0.0.1` and `--addr 127.0.0.1:<port>` are the only legal values. `--addr` is one global flag, and every command checks it. The default address is `127.0.0.1:9737`. Any other host, including `0.0.0.0`, a LAN address, or a hostname, is `NOT_IN_SPEC` before the pack opens, for every command. There is no account store.

One GM seat has three doors. `spook gm` and `spook ai-gm` dial the TCP listener and send `gm`. `spook mcp` is the stdio door for a coding agent: it takes no pack, dials `--addr`, sends `gm`, and copies bytes between its stdio and that socket. After the hello the socket is newline-delimited JSON-RPC, the same framing as MCP stdio. A refused hello prints a sentence and the token on stderr, writes nothing to stdout, and exits 1. `SESSION_FULL` prints `The table is full. [SESSION_FULL]`. A refusal later on that socket, including a GM line over the cap, is the same: the sentence and the token go to stderr, that line is not written to stdout, and the process exits 1. A shard that is not listening exits 1. Stdout carries protocol bytes only.

`spook run --mcp-http 127.0.0.1:<port>` opens the third door, a streamable HTTP endpoint at `/mcp` on its own port. The flag is off unless it is set. The address uses the same loopback parser as `--addr`, so any other host is `NOT_IN_SPEC` before the pack opens. Initialize on that endpoint claims the same GM seat through the same hello path. A second GM, whether it arrives on HTTP, on the TCP hello, or through `spook mcp`, is `SESSION_FULL`, and on HTTP that refusal is a JSON-RPC error whose message contains the token. HTTP DELETE, or the session ending, releases the seat. A request whose `Host` is not `127.0.0.1` is refused with 403. That includes `localhost`, so a client must address the endpoint as `http://127.0.0.1:<port>/mcp`. A request whose `Origin` header is present and is not `http://127.0.0.1` is refused with 403. A missing `Origin` is accepted. There is still no auth. A POST body larger than 1 MiB is refused. `spook/event` notifications may be ignored by a generic client. `session_status` with `since` is the pull path.

## Host monitor

A host monitor is not shard behavior.

`idle_digest` takes a host viewer or a player viewer. The host sees effect rows and clocks. A player sees only their messages and owned clocks. `spook status` prints the host digest. Player resume is the player digest. The host digest is not a player resume, and it does not write the shard.

## Player client

`spook play <pack>` sits a player seat and dials `--addr`. `--as player:<id>` picks the actor. Without it, the client sits the pack's start PC. Each client holds one distinct PC. The player socket wire is unchanged: the client only changes how it prints frames.

By default each frame is decoded and printed as readable text, one rendering per `PlayerMessage`, with no protocol JSON. A say is the speaker and the text: `GM: evening`, `You: hello` for the seated player, `Mira: the lamp is lit` for `npc:mira`. `queued` is `Your action is queued.` and does not print the queue id. `check` is `Your roll succeeded.`, `Your roll partly succeeded.`, or `Your roll missed.`. `perceive` prints `You are in <place>.`, `Here: <actors>`, and the `You know:`, `Notes:`, `Clocks:` (`Assassination debt: 2/6, paused`), and `Stock:` sections, skipping empty ones. It does not print fact bodies, unknown mentions, habits, or ids. `resume` is `While you were away:` followed by one line per player digest row. A message row is its text, without the message id, and an owned clock row reads like a perceive clock (`Assassination debt: 2/6, paused`). A row the client cannot read is shown as it came. A refusal is a sentence and the token, such as `The table is full. [SESSION_FULL]`, `That character is already taken. [SEAT_TAKEN]`, `That line is too long. [LINE_TOO_LONG]`, on stderr. A token without a sentence is `The table refused that. [<TOKEN>]`. Ids are shown as names: a leading `npc:` or `player:` is dropped, `_` becomes a space, and the first letter is capitalized. A line the client cannot decode is written to stderr unchanged.

`spook play --raw` prints each protocol frame as one JSON line on stdout, as the socket sent it. A `refused` frame prints only its token on stderr.

A refusal before the first `perceive` exits 1. After the first `perceive`, a refusal is printed and play continues. stdin lines go to the socket unchanged.

## Host

`spook ai-gm <pack>` sits the single GM seat, the same seat as `spook gm`, and dials `--addr`, which defaults to `127.0.0.1:9737`. It starts neither `spook run` nor `spook play`. It stays a TCP client. It does not speak MCP stdio. A second GM, on that TCP seat, on `spook mcp`, or on `--mcp-http`, gets the refusal the hello already returns, and the host exits. `spook run` is the only listener. `--mcp-http` is an extra loopback port on that same process, not a second server.

`spook ai-gm` with no pack, or with any extra argument, is a usage error before a dial. A refused hello exits 1. A shard that is not listening exits 1 and leaves the pack bytes as they were.

On start the host reads `SPOOK_GM_KEY`, `SPOOK_GM_URL`, and `SPOOK_GM_MODEL`, in that order. The first one missing: it writes `<NAME> is unset` to stderr and exits 1 before it dials. There is no default URL. The client posts to `{SPOOK_GM_URL}/chat/completions` with the key as a bearer token. The key is not an argument and not a pack file.

The host calls `pack_rev` and `load_canon` on the pack path. It does not call `open_shard`, `apply_effect`, or `write_commit`. After initialize it calls `session_status` with no arguments and compares `pack_rev`. A mismatch disconnects and exits 1. A match writes `listening` to stderr. stdout stays empty.

A non-empty queue is committed once after the model's turn. Talk alone does not tick. If the model call fails, the host skips the model's says and intents and still commits a queue the player already filled. The seat stays up. Each model call waits at most 60 seconds and at most 10 seconds to connect, and a timeout, a connect error, HTTP 429, or a 5xx is tried once more after 2 seconds. A commit that rolled dice gets one say-only follow-up. That follow-up does not queue and does not commit. `commit_turn`'s result includes `checks` (`faces`, `stat`, `total`, `target`, `outcome`). The same roll notices are still sent as `check` events on the GM socket. The player wire is `{"kind":"check","outcome":"success"|"partial"|"miss"}`. Faces stay off the player socket.

An addressed actor who is not in the room gets one table say, `They are not here.`, and no model call. On an actor-addressed wake, a say is kept only when its address is `actor` and its actor id is the one who was addressed. On a table or place wake, a say is kept only when it is `table`, or `place` naming the speaker's place. A queued wake, or a sit wake for a non-empty queue, has no addressed actor. The host censuses the PC in `presence.actors_present` the way a table wake censuses the speaker, and keeps a say only when it is `table`, or `place` naming that PC's place. If that list has no PC, the host skips the model, submits no say, and still runs the commit step. One model reply may submit at most two says and one intent, after that rule. A say the rule rejects does not consume the cap. An intent whose move is not in the menu is dropped before the cap.

Speech already on the tape at sit is not answered. High water starts at the current last seq. A queue that is already waiting still commits.

The POST body is the bounded prompt: the last 16 says this process has handled, the new player lines, the move menu, the stripped views, and roll faces on a follow-up. A restart remembers nothing. A player view that is only a room census drops `known_facts`, `messages`, and `unknown_mentioned`. An NPC view keeps each fact's `id` and `text`. Every fact `body` is dropped. The prompt does not receive a canon or a cast body. The tape file stays on disk under the pack. The POST does not carry the tape file, `now.yaml`, or a cast file.

## Firewall, lint, and digest

`allows` means the fact is known, or it is not hidden and it is on the viewer's place, or it is not hidden and it is on a co-located owner's cast or place. Hidden facts never leak through place or cast prose.

`perceive` drops a message whose `text` contains the text of a fact that viewer is not allowed to know, the same way it drops a message with an unallowed `fact_ids` entry. `apply` does not refuse that message. Who can see a place note changes as actors move, so the check belongs at perceive time. `spook check` reports that template with `HIDDEN_FACT_IN_TEXT`, the file, and the line.

`spook audit <pack>` replays the journal once from its base into one temp directory and deletes that directory. At every committed state, including the base, it renders the player perceive line for every PC. It fails when that line contains the text of a hidden fact that PC does not list in `known_facts`, or that fact's id as a JSON string, so a shorter id does not match a longer one. That set is read from the canon and the save. Audit does not call `allows`. A player check notice is an outcome or a refusal token and carries no fact id or text. The journal does not store the notice, so the engine line audit scans is the perceive line. One `pass <pc>` or `FAIL <pc>` line is printed per PC, then each offending perceive line as `state_rev <r> <pc> <line>`.

Speech is the table's choice. For each tape line and each recipient, a hidden fact's text the recipient did not know when the line was written is a note: `note seq <n> speaker <from> recipient <id>`. Notes do not change the exit code. `--strict` makes them fail. Tape lines are matched to the earliest journal state with the same tick index, and the output says `notes follow the journal state at the tape line's tick`. When a line cannot be matched, notes use the final state, and the output says `notes use the final state`. Exit 0 when no perceive line leaks and either there are no notes or `--strict` is off. Exit 1 on a perceive leak, or on a note when `--strict` is set. A path with no `pack.yaml` or no `state/now.yaml` prints `NOT_IN_SPEC` and exits 1. The command writes nothing under the pack, takes no lock, and binds no port.

`perceive` is budgeted. The budget is 800 tokens unless the caller passes a budget. `lint` scans Markdown bodies for hidden fact text, and `spook check` also scans message template text. `strip_body` deletes the body, leaves frontmatter, and the pack still ticks. Those functions stay in firewall, perceive, digest, and lint.

Load fails with `SCHEMA` if `places.actors` is edited apart from `actors.place`. Emit rebuilds occupancy from `actors.*.place`.

Faces of a roll go to the GM socket only. The player socket gets `{"kind":"check","outcome":"success"|"partial"|"miss"}` or `{"kind":"refused","code":"..."}` for that player's own roll. An NPC's roll sends no player frame. No faces. The tick JSONL lists `resolve_check` and `apply_effect` without faces. A refused tick records `rejected[].reason` as the token: `BUDGET` lowercased, `empty_perceive` as that word, every other token uppercase.

## Non-goals

Still not built, and not implied by a missing sentence above:

- A cargo feature named `llm`, or an NPC `from` on the tape.
- Parsing speech into `known_facts`, or a whisper that grants a fact.
- A non-loopback bind, accounts, or a second GM.
- An ack cursor, or a per-client outbound queue whose purpose is to make a host event start a host turn. The seat's 256-frame send queue only drops a slow reader.
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
| Replay applies JSONL effects as the new state | Replay does not install the log as `Now`. It re-expands the recorded intents, calls `resolve_check` again from the recorded seed, and then calls `apply_effect`. A presence record restores presence. A migrate record restores `pack_rev`. Neither is an effect list. |
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
| A second snapshot beside `now.yaml` | `state/log/base.yaml` is replay input written in the commit. It is never read as the live save. One emit path still writes `now.yaml`. |
| Non-loopback bind, accounts, second GM, fourth player, held PC | `NOT_IN_SPEC` for a non-loopback bind. No account store. `SESSION_FULL` for a second GM or a fourth player. `SEAT_TAKEN` for a held PC. |
| Hidden fact in a perceive body | `allows` plus lint. |
| A client sends an endless line, floods lines, or stops reading | Hello and player lines cap at 8 KiB (`LINE_TOO_LONG`). A GM TCP line caps at 1 MiB. Invalid UTF-8 on a hello or player line is `SCHEMA`. A hello that does not finish in 10 seconds is closed and commits nothing. A player connection allows 5 lines per second with a burst of 10, and it waits for the next allowance instead of dropping the line. The GM seat is not rate-limited. An outbound queue of 256 frames drops a client that stops reading, and the shard thread does not block on that send. |

Privacy: the pack is local. There is no account, no connection secret, and no telemetry. The tape holds table speech on disk under the pack. That is the product log. The tape file stays on disk. `spook ai-gm` sends table text off the machine only as the body of its chat-completions POST: the last 16 says this process has handled, the new player lines, the move menu, the stripped views, and roll faces on a follow-up.
