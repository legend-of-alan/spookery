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
- The rules primitives must be able to express a 5e-style game with no 5e-specific code. Generic engine, not a 5e replacement, and no SRD text. `packs/sample-d20` is that sample. It plays a fixed-damage fight against a literal difficulty, and drinking the tonic uses that tonic up. It does not yet express a difficulty taken from the target's defense meter, rolled effect amounts, advantage-style rolls, a modifier from equipment, or a condition when hit points reach zero.

This spec records one writer, the crate boundary, the threat list, loopback and the seat cap, and the rule that a host monitor is not shard behavior.

## Referee rules

These fourteen rules are the referee, in this order. Later sections are the detail. A sentence in another document does not add a fifteenth rule.

1. **Import boundary.** `spookery-kernel` and `spookery-round` do not link a runtime, a database, or a model SDK.
2. **`!Send` `Draft`.** One `std::thread` owns `Now`. The tick is not spawned on a runtime.
3. **Atomic commit.** `save.yaml` is renamed last. A crash cannot apply a partial tick.
4. **Closed effect, one apply.** The effect classes are `clock`, `meter`, `item`, `wield`, `unwield`, `location`, `memory`, `flag`, `message`, and `narrate`. `apply_effect` is the only mutator of world numbers. A rule's effects apply all together. If one effect is refused, or the list is longer than `max_effects_per_intent`, that intent is rolled back and none of its effects are saved. The tick names the refusal. That intent does not report its roll. The effect cap's reason is `budget`.
5. **Firewall.** `firewall::allows` is the visibility test. A hidden fact does not leak through place or cast prose.
6. **Deterministic tick.** A row's literals are filled before the apply loop. `round::play` runs `advance`, `complete`, `gather`, `resolve`, `apply`, `run_triggers`, `observe`, and `record` in that order. Replay runs those same stages. Each roll's seed is Blake2s of `{world}|{tick}|{rule}|{subject}|{roll_index}`. The tick line keeps `seed` as `{world}|{tick}`. Replay re-expands the recorded intents and rolls again from those keys. It also replays every journaled commit since the base: a presence commit restores that presence, and a migrate commit restores `pack_rev`.
7. **Canon versus `Now`.** Canon is frozen. `Now` is the live snapshot.
8. **Action-row loader.** A moves file is names and rows. A field is a literal, or the player token on a player row. A new verb is a new row in YAML.
9. **Seeded checks.** `rules::roll` is the only roll. It is Blake2s, one copy, in `rules/roll.rs`, and `crates/round` calls it from one site. There is no shared die cursor and no dice cap. The intent's own actor resolves the check, PC or NPC. Another actor does not. `difficulty` applies only when the row sets `check` and no opposing total applies. The total is `success` when it reaches the target, `partial` when the row sets `partial_at` and the total reaches that threshold but not the target, and `miss` otherwise. A row without `partial_at` is `success` or `miss`. An opposing total replaces `difficulty`, and that roll stays `success` or `miss` even when the row sets `partial_at`, because `partial_at` is compared with the row's own `difficulty`.
10. **Eligibility and habits.** Both run before a kernel habit intent exists. An ineligible NPC produces no habit intent. A player-sourced intent is not a habit intent.
11. **Failure tokens.** Each token has one meaning. `NOT_OWNER` and `LIVE_ONLY` are not tokens.
12. **Speech versus queue.** Speech appends to the tape. It is not a row, it does not enter the intent queue, and it does not tick.
13. **Seat law.** One GM seat and up to three player seats. Each player seat holds a distinct pack PC. The cap of three is the engine's, not a pack setting. A fourth player hello is `SESSION_FULL`, checked before any other player-hello refusal. A hello for a PC another seat holds is `SEAT_TAKEN`. A second GM is `SESSION_FULL`. The seat's `tools/list` is the menu.
14. **Rounds.** With no GM, each player line queues and commits its own tick. With a GM seated, every player's lines queue, and one `commit_round` commits them in queue order.

## One writer

The shard thread is the only writer of `Now` and of `Seats`. One `std::thread` owns `Now`. `Draft` stays `!Send`, with `PhantomData<Rc<()>>` as in `crates/kernel/world.rs`. No `tokio::spawn` of a tick.

The queue is a `Vec` of intents on the shard thread. It is ephemeral. A crash drops every uncommitted line. A crash cannot apply a partial tick. The queue is not a field of `save.yaml`. It is not a side file that recovery has to remember.

`Draft::begin` clones `Now` once into `state`. That is the only clone. `commit` moves `state` into `World.now` after `write_commit` returns. It does not clone again.

`expand` takes `&Canon`, `&Now` (the draft borrow), and `&Intent`. It does not hold `World`. `apply_effect` takes `&mut Draft`. The tick does not hold `&World` and `&mut Draft` in a way that aliases `Now`. Checks read `current` off the pre-loop `frozen` snapshot, the same `Now` expand uses. They do not read the live draft. Perceive and `allowed` run again in apply on the live draft, so a later intent sees what earlier ones in the tick already changed. The item precondition reads the actor's inventory before queue. `round::play` receives `&mut World` and runs the eight stages over one `Draft`. `World` does not publish `set_pending`, `set_rolls`, `queue_intent`, or `clear_queue`.

`append_session` takes the session lock and never a `Draft`. `commit_presence` takes the shard lock and never a `Draft`.

`Seats` is one GM seat and up to three player seats, plus a process-local session counter. Each player seat is one live socket bound to one pack PC. The cap of three is a constant in the seat code. It is not a pack setting. `Seats` is not a second presence document. The durable source of who is at the table is `Now.presence`, and that struct is only `humans` and `actors_present`. Session ids are not in it. The shard thread is the only writer of both `Seats` and `Now`. A player hello or drop that changes `humans` or `actors_present` calls `commit_presence` once before the handler returns. A drop releases only that seat. Sitting the GM does not change those fields and does not write `save.yaml`.

Invariant, checked by a debug assert on the shard thread: `presence.humans` equals the number of held player seats, and `presence.actors_present` is exactly the set of those PCs. The assert is debug-only. It is not a library `unwrap`. Startup repair is `clear_crashed`. `listen` binds, then the shard thread opens the shard and, if `humans != 0` or `actors_present` is non-empty while `Seats` is empty, `commit_presence` writes zeros.

## Crate boundary

Three crates, all under `crates/`. `crates/kernel` is `spookery-kernel`. `crates/round` is `spookery-round`. `crates/spook` is package `spook`, library `spookery`, binary `spook`. Protocol, the player client, and the model client are modules of that library: `protocol`, `terminal`, and `ai`. A one-crate collapse is not the layout. The `spook` package may link a runtime. The `ai` module may link an HTTP client.

| Crate or module | May link | Must not link |
|---|---|---|
| `protocol` | `serde`, `serde_json` | `tokio`, `rmcp`, `reqwest`, `spookery-kernel`, `terminal`, `ai` |
| `spookery-kernel` | `serde`, `serde-saphyr` `=1.3.0`, `serde_json`, `blake2`, `thiserror`, std | `tokio`, `rmcp`, `rmcp-macros`, `sqlx`, `postgres`, `redis`, `jsonschema`, `reqwest`, any model SDK, `spookery-round`, the `spook` crate |
| `spookery-round` | `spookery-kernel` | `tokio`, `rmcp`, `reqwest`, `serde_json` as a direct dependency (it uses `kernel::codec`), MCP, any model SDK, the `spook` crate |
| `terminal` | `protocol`, `spookery-kernel` | `ai`, `reqwest`, `rmcp`, `tokio` |
| `gm_connection` | `rmcp` client, `tokio`, the HTTP transport | `ai`, `terminal`, `spookery-kernel` |
| `ai` | `protocol`, `spookery-kernel`, an HTTP client, a runtime | `spookery-round`, `terminal`, `rmcp`, any model SDK |
| `audit` | `spookery-kernel`, `spookery-round`, the player perceive renderer | `tokio`, `rmcp`, `reqwest` |
| bin `spook` | `spookery-kernel`, `spookery-round`, `tokio`, `rmcp`, `socket2`, `serde_json`, and its `protocol`, `terminal`, `ai`, `gm_connection`, and `audit` modules | `reqwest` outside `crates/spook/src/ai/` and `crates/spook/src/gm_connection.rs`. Host code that calls `terminal` or `ai`. Business rules that belong in the kernel. A second tick implementation. |

`serde_json` in the kernel is allowed. The import scan does not forbid it. The scan stays a source scan, not a `Cargo.toml` parser. It reads every runtime file under `crates/kernel`, `crates/round`, and `crates/spook/src`. `crates/kernel` and `crates/round` must not contain the byte strings `tokio`, `rmcp`, `sqlx`, `postgres`, `redis`, `reqwest`, `langchain`, `openai`, `anthropic`, or `ollama`. `reqwest` may appear only under `crates/spook/src/ai/` and `crates/spook/src/gm_connection.rs`. `protocol` must not contain `tokio`, `rmcp`, `reqwest`, `spookery_kernel`, `crate::terminal`, or `crate::ai`. `terminal` must not contain `tokio`, `rmcp`, `reqwest`, or `crate::ai`. `ai` must not contain `rmcp`, `crate::terminal`, or `spookery_round`. `host` must not contain `crate::terminal` or `crate::ai`. `gm_connection` may contain `tokio`, `rmcp`, and `reqwest`. It must not contain `crate::ai` or `crate::terminal`. `audit` must not contain `tokio`, `rmcp`, or `reqwest`. A comment in a scanned file that names a banned crate is still a failure. This spec is where that ban is written. There is no cargo feature named `llm`. An optional cargo feature may be named `ai`.

Tokio in `spook` is the accept loop, keepalive, scheduler sleep, and the GM client's runtime. The tick function runs on the shard thread.

## Effect envelope

`Effect` is one struct. The class is the payload. Rows emit the classes below.

```rust
pub struct Effect {
    pub class: EffectClass,
    pub as_viewer: ViewerId,
    pub intent_id: String,
}

pub enum EffectClass {
    Clock { clock_id: String, delta: i64 },
    Meter { actor_id: String, key: String, write: MeterWrite },
    Item { actor_id: Option<String>, place: Option<String>, id: String, delta: i64, max: Option<i64> },
    Wield { actor_id: String, id: String },
    Unwield { actor_id: String },
    Location { actor_id: String, from: String, to: String, arrived_at: Option<u64> },
    Memory { actor_id: String, fact_ids: Vec<String>, tags: Vec<String>, text: Option<String> },
    Flag { actor_id: String, key: String, value: bool },
    Message { message_id: String, to: Vec<String>, place: Option<String>, fact_ids: Vec<String>, text: String },
    Narrate { to: Vec<String>, fact_ids: Vec<String>, text: String },
}
```

`ViewerId` is `Player` or `Npc`. There is no `Gm` variant. The JSON token is `player:{id}` or `npc:{id}`.

JSONL and MCP write one flat object per effect. The object is internally tagged and flattened. `class` is a string: `clock`, `meter`, `item`, `wield`, `unwield`, `location`, `memory`, `flag`, `message`, or `narrate`. `MeterWrite` is `Base(i64)`, `Offset(i64)`, or `Snap`. The wire writes `write` as `base`, `offset`, or `snap`, and writes `delta` only for `base` and `offset`. Payload fields are siblings of `as` and `intent_id`. They are not nested under `class`. An externally tagged object is not the codec. Key order is fixed: `class`, then `as`, then `intent_id`, then payload fields in the order declared on that `EffectClass` variant. A custom serializer writes that order. serde's default enum encoding does not. `save.yaml` byte-stability does not depend on this object. It is the tick JSONL shape only. Tests assert this order.

## Apply

`apply_effect` is the only mutator of world numbers. Arithmetic stays `checked_add`. A meter write clamps a sum that fits in `i64`. Removing an item from a count of 0 is `MISSING_ITEM`. A removal that would pass below 0 from a positive count is `OUT_OF_RANGE`. Memory text longer than 240 characters is `SCHEMA`. Memory facts the viewer may not know are `ILLEGAL_KNOWLEDGE`. The flag key `busy` is `SCHEMA` at load and is not a template. An actor is busy when `act.busy_until` is still after `world_time`.

The effect's viewer must be the active intent's actor. Failure is `VIEWER_MISMATCH`.

Clocks do not clamp. An unknown clock is `UNKNOWN_KEY`. A paused clock, or a clock that requires presence while `humans == 0`, is `PAUSED_CLOCK`. `value + delta` outside `[0, max]` is `OUT_OF_RANGE`. If the intent actor is not in `clock.owners`, the result is `UNKNOWN_KEY`. That actor has no such clock key.

Location: `from` must equal the live place, or the effect is `OUT_OF_RANGE`. `to` must name a place in the pack, or the effect is `NO_EXIT`. An `actor_id` other than the intent actor is `ACTOR_MISMATCH`. A PC location while `humans == 0` is `TABLE_EMPTY`. A timed arrival sets `arrived_at` to the world minute the traveller finishes. An instant move omits `arrived_at`. Arrival adds that place to the PC's `visited` list.

An `item` effect adds or removes a count on an actor or a place. `who: place:<id>` and `who: here` name a place. Removing from a count of 0 is `MISSING_ITEM`. A removal that would pass below 0 from a positive count is `OUT_OF_RANGE` and the intent rolls back. `max` caps a positive add. A count that lands on 0 is omitted. Dropping or spending the wielded item clears that actor's wielded slot in the same tick.

Meter: `write` is `base`, `offset`, or `snap`. `base` adds the delta to `base` and clamps that base into `[min, base_cap]`, and keeps `offset`. `offset` adds the delta to `offset` and keeps `base`. `snap` sets the next pair to `(base, 0)`. The live pair is assigned only when `base` is inside `[min, base_cap]` and `base + offset` fits in `i64`. A sum that fits and passes `max` is clamped when `current` is read. `current` is `clamp(base + offset, min, max)`. It is not stored. An add that does not fit in `i64`, or a next pair that fails that check, is `OUT_OF_RANGE` and the record is unchanged. An unknown meter id is `UNKNOWN_KEY`. A PC and an NPC take the same write.

## Rows and habits

A rule lives in `rules/verbs.yaml` or in the file of its owner: a place, an item, a cast member, or a story. The tag on the shared file is `verbs`. `rules/moves.yaml` does not load. Check says it was replaced by `rules/verbs.yaml`.

One rule has `id`, `when`, an optional `if`, an optional `cost`, an optional `roll`, and the lists `do`, `success`, `partial`, and `failure`. `when` is a verb, `habit`, or a change. A verb may name a target kind: `none`, `actor`, `place`, `exit`, or `item`.

The full id is the owner plus the local id: `verb:travel`, `place:docks_shop/bell`, `item:tonic/use`, `cast:mira/restock_lamp_oil`, `story:smuggler/bell`. For `attack`, the wielded weapon's rule wins when that item has one. Otherwise one verb intent uses the target item's rule for that verb, then the target actor's, then the rule on the actor's current place, then the rule in `rules/verbs.yaml`. A full id is not a verb word and does not select a rule. A habit is selected by its local id, and only for that NPC.

`if` is a list. Each dash is one condition: `has_item`, `wielding`, `meter`, `flag`, `at`, `knows`, `clock`, `hour`, `humans`, `note_exists`, `all`, `any`, or `not`. Two conditions under one dash are `SCHEMA`. A failed `has_item` is `MISSING_ITEM` and that line is not queued. `has_item` and `wielding` read `id: target` as the intent's target token. `wielding` is true when that actor's wielded slot holds that item.

`roll` names a `check` and a `target`. The target is a number, or a meter plus a number. `partial_at` is strictly below that target and needs a `partial` list. A `partial` list without `partial_at`, a `partial_at` without that list, a `partial_at` at or above the target, or an outcome list without a roll, is `SCHEMA`. An opposing total replaces the target and that roll stays binary.

`do` runs on every band. `success`, `partial`, and `failure` run on that band. `failure` is the miss. A yield is `do: []` with no outcome lists. `wait` is a yield. A yield with a roll reports the band and writes no world key. `say` is speech, so that id is not a rule.

`cost` may name `time`, meters, and items. A cost the actor cannot pay is `CANT_AFFORD`. The refusal names the meter or the item, the journal word is `CANT_AFFORD` plus that name, and the world is unchanged. The player sentence is `You cannot afford <name>. [CANT_AFFORD]`. Items in `cost` are paid when the intent commits, before its effects, and a spent wielded item clears the slot. `time` is parsed to minutes and is applied. See Exits and travel. `resist` on an effect gives its subject a keyed roll. A success shrugs that effect off and runs `resisted` instead. `resist: same` reuses the previous resist roll in that list. `do` counts for the outcome lists that follow it. `spook check` reports `resist: same` when no earlier effect in that run has rolled a resist. A resist is an outcome. The tick does not record it as a refusal. Both the action roll and the resist roll use the per-roll seed, so `spook verify` reproduces them.

A change rule fires in `run_triggers`, after `apply`, from the changes made in that tick. It is edge-triggered: the watch was false before the change and true after. A watch is arrive, leave, a meter cross, a flag becoming a value, a clock filling, an item gained or lost, or a fact becoming known. `here` on a place rule is that place. Matching rules run in owner order, then file path, then position in the file. `if` is read against the staged world. Those rules repeat on the changes they just wrote until a round writes nothing a watch matches. A rule runs at most once for one subject in a round. A tick runs at most `max_trigger_depth` rounds. The default is 4, set in `tick.yaml`. When another round would pass that cap, the tick trace records `trigger_depth` and stops. A change rule may roll. That roll uses the per-roll seed, so `spook verify` reproduces it. A change rule with a cost does not load.

A player line queues a verb. Typing a habit name is `NOT_IN_SPEC` and is not queued. A full id is `NOT_IN_SPEC` for a PC. It does not select another actor's habit or a rule on another place. The GM intent tool queues a verb for a player or an NPC, and that NPC's habit. A habit for a PC is `NOT_IN_SPEC`. `source` on that tool stays `player`. A player-sourced habit skips `if`. A kernel habit still requires eligibility and `if`.

Load rejects an empty id, a duplicate id, two verb rules for the same verb in one file, the flag key `busy`, a rule id `say`, an unknown effect key, and a retired selector (`token`, `first_rate`, `rate`, `first_drift`, `toward_target`, `first_known`, `first_listener`, `keep`, `first_due_note`, `first_owned`). `travel` targets an exit. The kernel admits that exit before the tick. The docks grit check is the `grit` verb. The player wire object is `{"kind":"check","outcome":"success"|"partial"|"miss"}`.

An effect is `meter`, `item`, `wield`, `unwield`, `flag`, `location`, `note`, `narrate`, `clock`, `memory`, or `reveal`. `note` is stored with the save key `messages`. `narrate` is one-off text. It is written on the tick for the people it names and is not stored in the save. `reveal` loads and does not apply on this pass. `clock` `start: true` clears that clock's `paused`. A memory effect adds its fact ids to that actor's known facts. `location` `via` names an exit from the actor's current place. `location` `to` with `teleport: true` names a place and does not need an exit. `location` `to` without `teleport: true` is `NOT_IN_SPEC`. A meter `add` offsets the current pair. A meter `set` offsets by the distance from current to that number. `wield` puts a held weapon in the actor's one wielded slot. An item that is not tagged `weapon` is `NOT_A_WEAPON` and the slot stays as it was. Wielding an item the actor does not hold is `MISSING_ITEM`. `unwield` empties the slot. Item definitions live in `world/items/<id>.md` with `id`, `name`, `tags`, prose, and rules. An actor's inventory and a place's `items` are `{id: count}`. The save's `wielded` is an item id or absent. `spook check` reports an unknown item id in a save or a rule.

Bindings and the outcome list are different steps.

1. Habit selection picks one cast rule before the apply loop. Expand fills that rule's `do` list against the staging `Now`. A rule whose `do` list does not expand is not the kernel intent. Player travel binds the typed exit.
2. In `resolve`, `rules::roll` runs when the rule has a `roll`. The viewer is that intent's actor. A PC or an NPC may roll for themselves. A viewer who is not the actor is `UNKNOWN_KEY`. An unknown id is `UNKNOWN_KEY`. Each roll has its own key, so another intent does not move its dice.
3. After that roll, the selected templates are `do`, plus `success` on success, `partial` on partial, and `failure` on miss. No roll means `do` only. The result is a `Vec` of `EffectClass`.
4. Expand wraps that vec with `as_viewer` and `intent_id`. That post-roll vec is what `apply_effect` sees and what the tick JSONL stores. `play_recorded` runs the same stages, which expand those intents again and call `rules::roll` again from each roll's key. Replay does not install the log as `Now`. A presence record restores `presence` and `state_rev`. A migrate record restores `pack_rev` and `state_rev`. Replay applies those commits. It does not install them as effects. The names `need` and `resource` are not variants. An effect that is refused rolls the whole intent back. A list longer than `max_effects_per_intent` is refused as `budget` before any of it applies.

The post-roll value is the effect vec. Habits may have `success`, `partial`, and `failure`.

The player queue on `World` stores `Intent` only. The post-roll effect vec is tick-local until the JSONL write. Replay keeps the recorded intents and expands them again. It also applies presence and migrate records as their commits.

Priority, computed before the roll, is player before NPC before system, then PC-owned clocks, claimed targets, then `shuffle_key`. `shuffle_key` is Blake2s of shard, tick, and actor. "PC-owned clock" uses the clock id on the expanded effect. "Claimed" means the target actor is in `actors_present`, or the target place contains a present PC, or a message's `to` hits `actors_present`.

Library paths return `Result`. `run`, `tick`, `status`, and `migrate` print `Reason::token()` and exit 1. `spook check` prints one diagnostic line per problem, or one JSON array when `--json` is set. `spook check --rules` prints each full rule id in owner order when the pack has no problems. `spook replay`, `spook verify`, and `spook audit` print their own lines.

## Exits and travel

A place file lists `exits`. Each exit has `to` (a place id) and `name`, and may have `cost`, `if`, `locked_text`, and `hidden` with `revealed_by`. Two exits on one place with the same name are `SCHEMA`. `cost.time` is stored as minutes (`30s`, `10m`, `2h`, `1d`). `spook check` reports an exit whose `to` is not a place as `SCHEMA`. A place no path reaches from the start, counting hidden and locked exits, is `WARNING`. `teleport: true` on a rule in `rules/verbs.yaml` is `WARNING`. Author rules may teleport.

`travel` targets an exit. `location via` names that exit from the actor's current place. A missing exit, or a hidden exit the actor does not know, is `NO_EXIT` and changes nothing. The player sentence is `There is no exit that way. [NO_EXIT]`. A locked exit (`if` fails) shows `locked_text`, changes nothing, and uses the same token: `The stair door is locked. [NO_EXIT]`. An unpaid exit cost is `CANT_AFFORD`.

The traveller stays at the origin until the journey finishes. `busy_until` is `world_time` at the start of the tick plus the rule's minutes plus the exit's minutes. `busy_for` stores the rule, the verb, the target, the destination, the exit name, and `player`, `kernel`, or `system`. The save field is `act`. A verb while `busy_until` is still after `world_time` is `BUSY` and is not queued. The player sentence is `You are busy. [BUSY]`. Speech is not `BUSY`. Meters and items are paid when the journey starts. The location effect is applied when the journey finishes.

The stage order stays `advance`, `complete`, `gather`, `resolve`, `apply`, `run_triggers`, `observe`, `record`. Arrivals are applied at the end of `apply`, in the first tick whose end is at or after `busy_until`, ordered by `(busy_until, actor_id)`. The tick line records each arrival with `arrived_at` equal to `busy_until`. A short journey can finish in the tick that started it. The actor's next verb is the following tick. A timed rule that is not travel pays its cost at the start and applies its roll and outcome effects at the finish.

Fast-forward starts when every seated PC is busy: a `BUSY` refusal with no GM seated, or a GM `commit_round` whose queue is empty. A GM commit that just ticked continues only when every seated PC is busy, and that tick counts toward the cap. The burst keeps ticking while any seated PC is still busy and the queue is empty, up to `max_fastforward_ticks` (default 12, set in `tick.yaml`). It advances world time without waiting on the wall clock. The idle wake does not fast-forward. The next `BUSY` refusal continues from where the cap stopped.

Each PC's `visited` list starts with the current place. Arrival adds the destination. `help` and `look` list the exits the actor can see from here. `map` lists visited places and those exits. A hidden exit stays off both until the actor knows `revealed_by`. Those three lines are a `lines` player frame and do not tick. `spook map <pack>` prints the whole exit graph, including hidden exits, as text and a mermaid block. It does not take the lock or bind a port.

## Habits

A cast file lists that NPC's habits as rules with `when: habit`, in file order. The idle runner produces at most one kernel intent per eligible NPC. It takes the first habit whose `if` holds and whose `do` list expands. Later habits wait. A habit that is always ready starves the ones under it.

The hour is `(world_time / 60) % 24`, read after the tick has advanced world time. `hour.from` and `hour.to` are integers in `0..24` and must differ. `from < to` means `from <= hour < to`. `from > to` wraps midnight. `humans` compares the seated count. A habit with no `humans` condition can run while the table is empty. `note_exists` is off when that id is already in the save. A `clock` condition compares that clock's value.

An empty id, a duplicate id, or an hour outside `0..24` is `SCHEMA`.

Eligibility runs first for a kernel habit. The NPC must be an NPC, the profile must say eligible, the NPC must not be busy, and no seated PC may share that place. The NPC must not have already acted this tick, and the idle-day or live-hour cap must be unspent. An ineligible NPC produces no habit intent. A player-sourced habit does not check `if` or eligibility. Apply still checks the exit, item counts, the firewall, paused clocks, and the other effect rules.

NPC perceive includes `habits`, the local ids that are on for that NPC, in file order. Player perceive sends an empty list. Queueing an intent does not consult that list.

`tick.yaml` pause is `named_plot_clocks`. `now.pause` stores that one view. Plot clocks with `requires_presence` are paused while `humans == 0` and the policy is `pause_when_empty`.

`tick.yaml` may set `idle` to `on` or `off`. Omitted means `on`. Any other value is `SCHEMA`. `on` wakes an empty table. `off` leaves an empty table still: the runtime does not arm the wake timer, a wake does not tick, and a later wake or a reopen does not catch up. `last_tick_at` and `world_time` stay unchanged while the table is empty. Seated play does not read `idle`. With no GM, a player line commits its own tick. With a GM, `commit_round` commits the queue. `spook tick` is a manual tick and does not read `idle`. Turning `idle` back to `on` after a long `off` period applies the same catch-up cap as any other long absence.

The empty-table knobs sit beside `idle`. `tick_every_real_seconds` is the real seconds between wakes. `idle_minutes_per_tick` is the world minutes one empty tick advances. `idle_catchup_max_ticks` caps how many ticks a catch-up writes. `idle_max_world_hours_per_real_hour` caps world hours gained per real hour of absence. `idle_acts_per_world_day` caps habit acts on an empty day. `tick_every_real_seconds: 0` and `idle_minutes_per_tick: 0` are `SCHEMA`. A seated catch-up is one tick.

Kernel habits do not emit speech. A player world line while `humans == 0` is refused at queue time with `TABLE_EMPTY`. A five-name denylist is not part of habit selection.

## Failure tokens

`Reason` is one `thiserror` enum in the kernel. It stays fieldless and `Copy`. `Display` and `token()` are the wire words. There is no parallel code enum. JSONL lowers only `BUDGET`, the log word `empty_perceive`, and a cost gap written as `CANT_AFFORD` plus the meter or item id. `run`, `tick`, `status`, and `migrate` print the token and exit 1 for an engine refusal. `spook check` prints the diagnostic line instead of the bare token. A usage error prints clap's message and exits 2. The player socket sends `{"kind":"refused","code":...}`. A cost gap also sends `missing` with the meter or item id. `code` stays the token.

`#[from]` is not used on `Reason`. I/O in the kernel collapses to `COMMIT_CONFLICT` or `SCHEMA`, so `Reason` does not wrap `std::io::Error`. The bin has its own error with `Reason` and `Io` variants. `#[from]` is used there, and only there. Tick functions return `kernel::Reason`. A tick-cause value is not an error.

Library and tick paths do not panic. Exhaustive matches replace `unreachable`. Tests may unwrap. A broken internal invariant returns `SCHEMA` rather than panicking, so a pack bug stays a token.

| Old emission | Token | One meaning |
|---|---|---|
| `NOT_OWNER` when the viewer is not the intent actor | `VIEWER_MISMATCH` | Effect viewer is not the intent actor |
| `NOT_OWNER` when a kernel source acts for a player | `KERNEL_ON_PLAYER` | A kernel habit tried to act for a PC |
| `NOT_OWNER` when a kernel source acts for an ineligible NPC | `INELIGIBLE` | NPC failed eligibility |
| `NOT_OWNER` when location or NPC counters need a tick profile and there is none | `NO_PROFILE` | The effect needed a tick profile and there is none |
| `NOT_OWNER` when location, memory, or a system memory names an actor other than the intent actor | `ACTOR_MISMATCH` | The effect names an actor other than the intent actor |
| `NOT_OWNER` when an item count would go below 0 | `OUT_OF_RANGE` | A shortfall from a positive count. A count of 0 is `MISSING_ITEM` |
| `NOT_OWNER` when a PC changes location while `humans == 0` | `TABLE_EMPTY` | This change requires a seated human |
| `LIVE_ONLY` on the five-name denylist | retired | Those names are not a denylist |
| `LIVE_ONLY` on a player world line while `humans == 0` | `TABLE_EMPTY` | A world line needs a seated human |
| The `empty_perceive` log word | `EMPTY_PERCEIVE` | JSONL spelling stays `empty_perceive`. It is not a player-socket token unless the seat surfaces it as `refused`. |

`NOT_OWNER` and `LIVE_ONLY` are not variants. Nothing emits them.

Tokens that already have one meaning and stay: `ILLEGAL_CLASS`, `ILLEGAL_KNOWLEDGE`, `OUT_OF_RANGE`, `BUDGET`, `PAUSED_CLOCK`, `UNKNOWN_KEY`, `PACK_REV_MISMATCH`, `UNKNOWN_MOVE`, `UNSCOPED_PERCEIVE`, `COMMIT_CONFLICT`, `HAND_EDIT`, `SCHEMA`, `NOT_IN_SPEC`, `SESSION_FULL`, `SEAT_TAKEN`, `MISSING_ITEM`, `NOT_A_WEAPON`, `LINE_TOO_LONG`, `NO_EXIT`, `BUSY`.

`LINE_TOO_LONG` is a hello or a player line over its cap. An HTTP body over 1 MiB is refused with 413 and does not use this token.

`SEAT_TAKEN` is a hello for a PC another player seat already holds. `SESSION_FULL` is a fourth player hello, or a second GM. The fourth-player check runs before the held-PC check.

`NOT_IN_SPEC` stays for a non-loopback bind, an unknown verb, `include_tick: false` with a pending queue, an unknown tool, and a `gm` hello on the game port. It is not the expand result for `give`. `MISSING_ITEM` is a failed `has_item`, wielding an item the actor does not hold, or removing an item that is not there. `NOT_A_WEAPON` is wielding an item that is not tagged `weapon`. `ILLEGAL_CLASS` is an unknown class at `parse_effect`, including `need` and `resource`. A field-shape error on a known class is `SCHEMA`. A moves-file unknown class is `SCHEMA`.

## Commit

`write_commit` is the only rename protocol for world bytes:

1. Copy each live target that already exists into `state/.txn/prev`, and fsync the copies and the `prev` directory.
2. Create `state/.txn/COMPLETE`, and fsync the file and the `.txn` directory.
3. Write each new body to a temp file, fsync, and fsync the parent.
4. Rename side files first (`journal/tick-NNNN.jsonl`, `journal/presence-NNNN.json`, `journal/migrate-NNNN.json`, `journal/base.yaml`, `memories/*.jsonl`).
5. Rename `save.yaml` last.
6. Fsync `state/`. Remove `.txn`. `state_rev` was incremented in memory before the bytes were emitted, and it hits disk only in that last rename.

Every commit that writes `save.yaml` also writes one journal record in that same plan. A tick writes `journal/tick-NNNN.jsonl`. A presence change writes `journal/presence-NNNN.json` with the new presence and the `state_rev` after the commit. A migrate writes `journal/migrate-NNNN.json` with `pack_rev` before and after and the `state_rev` after the commit. `NNNN` on a tick file is the tick index. `NNNN` on a presence or migrate file is the `state_rev` after the commit. Each record carries `now_hash`, the Blake2s of the `save.yaml` bytes that commit wrote. `save.yaml` is still renamed last, so a crash cannot leave it ahead of its record.

`rules_rev` is the engine rules version. The constant `RULES_REV` lives in the store and is 6. `save.yaml` and every tick record carry it. `spook run`, `spook verify`, and `spook replay` refuse a save or a tick whose `rules_rev` is missing or different. The refusal is `RULES_REV: start a fresh save`. There is no migration.

The first commit on a pack with no `journal/base.yaml` writes that file in the same plan. Its bytes are the pre-commit `save.yaml`. `migrate` writes `journal/base.yaml` again, in the same plan, as the post-migrate `save.yaml`, and the migrate record marks that point. Older records stay on disk. Replay from the base applies only the records after it, because those older ticks do not replay against the new canon. `state/journal/base.yaml` is replay input. Nothing reads it as the live save. `emit_save` is still the only path that produces `save.yaml`.

`recover` keeps three shapes. No `COMPLETE` means delete `.txn` and leave the live tree. `COMPLETE` and `live.state_rev == prev.state_rev` means restore `prev`. `COMPLETE` and `live.state_rev == prev + 1` means delete `.txn` and keep live. Then drop journal records whose `state_rev_after` is above live, drop `base.yaml` when its `state_rev` is above the live save, and drop memory lines whose `tick_index` is not in a live log.

`commit_presence` writes `save.yaml` through the same `write_commit`, with the presence journal record in that plan. Presence is `humans` and `actors_present` only. When those two fields are unchanged after canonicalize, it writes nothing and does not increment `state_rev`. When either field changes, it increments `state_rev` by 1 and then emits. It does not open a `Draft`, does not write a tick log, and does not advance clocks. A session id is not an input.

`migrate_pack` is the only writer that changes `pack_rev`. It goes through `write_commit`, writes the migrate journal record and the new base in that plan, and then appends one studio line. Replay does not read the studio line. A hand edit of `save.yaml` that is not `spook migrate` remains `HAND_EDIT` on the next tick. Studio tools do not write `state/`.

`write_commit` has no fault parameter. The crash hook is not an environment variable and not `process::abort`. No production path reads `SPOOK_CRASH_AFTER`. Dropping the `Draft` after `apply_effect` without calling `write_commit` leaves no `state/.txn`, and leaves `state_rev` and place items at the bytes from before `begin`. Recovery on the three directory shapes above is the other half of the same proof: an incomplete `.txn` leaves the previous `save.yaml`, `COMPLETE` with an unmoved `state_rev` restores `prev`, and `COMPLETE` with the new `state_rev` keeps the new `save.yaml` and deletes a journal record and a memory line that do not belong to it.

Replay byte-compares `save.yaml` produced by this kernel, for every journaled commit since the base. It also compares the effects logged on a tick with the effects the re-run produced, and names the tick and the first effect that differs. It does not pin a frozen docks hash. A pack that has never committed has no base and no journal. `spook verify` then compares `save.yaml` to itself and reports zero records.

## Speech

`say` is speech. It is not a row. The tape is not an input to expand. Text is not parsed into `known_facts`. There is no whisper that grants a fact.

A player line is `say` plus the words. The first word is the address when it is `place`, or an actor id, and more words follow. `place` is the speaker's live place. An actor id is that actor. Any other line is address `table`, and every word is the text. A line of one word is table text, including when that word is `place` or an actor id. An empty `say` is `SCHEMA`. Say text is at most 4096 characters; longer text is `SCHEMA`.

Speech appends when `humans` is 0. The append takes `state/.session.lock`, writes one object, and fsyncs. That path uses no `Draft` and does not bump `state_rev`. The object keeps `seq`, `from`, `address`, `place`, `actor`, `recipients`, `text`, `tick_index`, and `world_time`. `from` is `player:{id}` or `gm`. A typed `from` on the GM tool is `SCHEMA` and appends nothing. An NPC `from` is not representable. The speech parser has no NPC speaker.

Recipients are seated player actors. `table` includes every seated player. `place` includes the seated players in that place. `actor` includes that actor when they are a seated player. The speaking player is included. An NPC stored in `actor` is on the tape and is not a recipient.

The GM tool is the same act. `address` is `table`, `place`, or `actor`. `place` is required when the address is `place`, and `actor` is required when the address is `actor`. `actor` must name an actor. The other field is absent.

`read_session` returns the tape lines after a sequence number, up to a limit. The limit defaults to 64 and is clamped to 256. The file hard cap is 1 MiB. `append_session` reads the tail for the last `seq` and returns `OUT_OF_RANGE` when the next append would pass 1 MiB. A line that does not parse is `SCHEMA` for the whole read, not a skipped line. `session_status` with `since` uses this window.

## Rounds, seats, and profiles

With no GM, the runtime queues and commits one live tick of `live_minutes_per_tick` (default 5). With a GM seated, the line waits. `commit_round` requires `include_tick: true`. An empty queue writes no tick file, unless every seated PC is busy, in which case the table fast-forwards. `include_tick: false` with a non-empty queue is `NOT_IN_SPEC`. A tick is `live_minutes_per_tick` while `humans > 0` and `idle_minutes_per_tick` otherwise. A journey does not shorten that length.

One wake: when `idle` is `on` (the default), the runtime sleeps `tick_every_real_seconds`, reset when a tick actually advanced, and posts a wake. The shard thread wakes the live shard. A second open of the pack on that wake is not the scheduler. When `idle` is `off`, that timer is not armed, an empty table does not tick, and it does not catch up on a later wake or a reopen. `last_tick_at` and `world_time` stay unchanged while the table is empty. Turning `idle` back to `on` after a long `off` period applies the existing catch-up cap. A seated table still commits its own ticks, and `spook tick` still ticks.

The seat's `tools/list` is the menu. Player input is the move menu plus `say`, `help`, `look`, and `map`. `help`, `look`, and `map` do not tick. `say` does not tick. There is no AI parsing in v1 (see #54).

Sitting perceive allows `player:` and `npc:`. A missing `as`, or `as=gm`, is `UNSCOPED_PERCEIVE`. Perceive calls the perceive budget after that check. `firewall::allows` is the shared visibility test.

An unknown sitting tool is `NOT_IN_SPEC` and writes nothing. `apply_effect` and `patch_entity` are not sitting handlers. Effects come from row expansion during the tick.

## Human GM

`spook gm <pack>` sits the GM seat over streamable HTTP. `--addr` is the game address, which defaults to `127.0.0.1:9737`, and the client dials `http://127.0.0.1:<that port + 1>/mcp`. It starts neither `spook run` nor `spook play`. When `spook run` was started on port 0, that next port is not the agent door: pass the reported door with `--mcp-http`. `spook gm` and `spook ai-gm` are two clients of the same single GM seat. A second GM is `SESSION_FULL`.

`spook gm` with no pack, or with any extra argument, is a usage error before a dial. A refused seat prints a sentence and the token on stderr and exits 1. `SESSION_FULL` prints `The table is full. [SESSION_FULL]`. A shard that is not listening exits 1 and leaves the pack bytes as they were.

stdin is one command per line, and each command calls one seat tool. An empty line sends nothing. An unknown command prints the command list and sends nothing.

`status` calls `session_status`. It prints `Seated: <names>` or `Seated: nobody`, then `Queue:` and one `<name>: <move>` line per queued row, or `Queue: empty`. `look <actor>` calls `perceive` and prints that view the way `spook play` prints a perceive. The actor is `player:<id>`, `npc:<id>`, or a pack id. A pack PC is `player`. A pack NPC is `npc`. An id listed in `actors_present` is `player`. An actor the client cannot place prints `That is not in the world. [UNKNOWN_KEY]` and sends nothing. `say <text>` speaks to the table. `say @<place> <text>` speaks to that place. `say #<actor> <text>` speaks to that actor. The say tool sends no `from`, so the GM speaks as the GM. `queue <actor> <move> [target]` calls `intent` with `source` `player` and that actor's kind. When the move's token is `actor`, the target object is an actor. Any other typed target is a place. `commit` calls `commit_round` with `include_tick` true. It prints `Changed:` and one line per diff summary, or `Nothing changed.`, then a roll line for each check on the result. `help` prints the commands. `quit` leaves the seat.

Events print as they arrive, as text, with no protocol JSON. A say is `<name>: <text>`. A queued line is `<name> queued: <move>`. A check is `Roll: <faces>, +<stat> = <total> vs <target>, <outcome>`, where `<outcome>` is `success`, `partial`, or `miss`. Faces are joined with ` + `. A negative stat keeps its minus sign. The line uses the faces, stat, total, target, and outcome carried on the check event. A check already printed from `commit` is not printed again. Names drop a leading `player:` or `npc:` prefix, turn `_` into a space, and capitalize the first letter, so `player:pc_rowan` prints `Pc rowan`. The speaker `gm` prints `GM`.

A tool refusal prints a sentence and the token on stderr, using the same sentences as `spook play`, and the client stays up.

`spook gm --raw` prints each socket frame as one JSON line on stdout. A refusal prints that line and its token on stderr.

`spook new <dir>` writes a minimal pack into that directory and prints the next commands on stdout: `spook check`, `spook run`, and `spook play`. It creates `<dir>` when that path is missing and the parent directory exists. A missing parent exits 1, prints `<parent> does not exist` on stderr, and creates nothing. An existing non-empty directory, or a path that is not a directory, exits 1, prints a sentence on stderr, and writes nothing. A write that fails removes what it created: a directory that was already empty stays empty, and a directory this command created is removed. The written save has `state_rev` 0, `rules_rev` 6, and a `pack_rev` computed from `world/`, `cast/`, and `rules/`. The command writes only under `<dir>`. It does not take the lock or bind a port.

`spook check <pack>` reads the pack and prints one line per problem. It loads `pack.yaml`, `world/`, `cast/`, `rules/*`, and `state/save.yaml` when that file is present, compares `pack_rev` with the canon hash, and scans Markdown bodies and message template text for hidden-fact text. It does not take the lock, bind a port, recover a transaction, or write under the pack. A pack with no diagnostics prints `no problems` on stdout and exits 0. A diagnostic whose code is `WARNING` is printed on stderr and does not by itself fail the command. Any other code exits 1. JSON prints the array either way. A pack whose diagnostics are all warnings exits 0. Problems go to stderr as `file:line: message (CODE)` when a line is known, or `file: path: message (CODE)` when the location is a key path such as `rules[0].when`. A missing file uses the key path `schema`. The message is plain English and says how to fix the problem. The code in parentheses is one of the codes in the table below. `run`, `tick`, `status`, and `migrate` still print only the token, except a rules mismatch, which prints `RULES_REV: start a fresh save`.

`spook check --json <pack>` runs the same checks. It prints one JSON array on stdout and writes nothing else there. Each element is a diagnostic with `file`, `line`, `column`, `path`, `code`, `message`, and `hint`. `line` and `column` are 1-based when present. An unknown `line`, `column`, `path`, or `hint` is null. The array is ordered by `file`, then `line`, then `path`. A null sorts before a value. A pack with no diagnostics prints `[]` and exits 0. A pack whose diagnostics are all `WARNING` prints that array and exits 0. Any other code exits 1.

The codes `spook check` can emit are stable. The message and the hint may change.

| Code | Meaning |
|---|---|
| `SCHEMA` | The loader rejected the file's shape. |
| `NOT_IN_SPEC` | A file the loader requires is missing. |
| `PACK_REV_MISMATCH` | `pack_rev` is not the hash of `world/`, `cast/`, and `rules/`. |
| `RULES_REV` | `rules_rev` is missing or is not the engine's rules version. Start a fresh save. |
| `HAND_EDIT` | `state/save.yaml` is not the canonical save text. |
| `HIDDEN_FACT_IN_BODY` | A hidden fact's text appears in a Markdown body. |
| `HIDDEN_FACT_IN_TEXT` | A hidden fact's text appears in a note. A rule file is not a body, so this is not `HIDDEN_FACT_IN_BODY`. |
| `WARNING` | The pack loads. An unreachable place, or `teleport: true` in `rules/verbs.yaml`. Printed, and the command still exits 0 when every diagnostic is a warning. |

`spook replay <pack>` replays the journal from `state/journal/base.yaml` into a temp directory, deletes that directory, and prints `state_rev <r>, <n> records`. `spook verify <pack>` reads `save.yaml` once, replays the journal up to that `state_rev`, byte-compares, and compares logged effects with the re-run. Later records are ignored. A match prints `verified <n> records, state_rev <r>` and exits 0. A mismatch exits 1 and prints the first divergent record. When the save bytes differ, it also prints the first differing `save.yaml` line. When logged effects differ, it names the tick and the first differing effect. `--from <dir>` uses that directory's `state/save.yaml` as the base instead. Both commands write nothing under the pack, take no lock, and bind no port. A pack that has never committed has no journal and no base. Verify treats `save.yaml` as its own base, so the check passes with zero records. A path with no `pack.yaml` or no `state/save.yaml` is not a pack, and both commands print `NOT_IN_SPEC` and exit 1.

## Loopback

A fourth player hello is `SESSION_FULL`. A hello for a PC another seat holds is `SEAT_TAKEN`. A second GM is `SESSION_FULL`. A refused hello gets one refusal line. The seat then closes its write side and reads and discards whatever the client already sent, until the client hangs up or 2 seconds pass, so a client that sent more than its hello still gets the refusal.

A hello line and a player line are at most 8 KiB, not counting the newline. A longer line is `LINE_TOO_LONG` and the socket closes. The server does not keep more than that cap. Invalid UTF-8 on a hello or a player line is `SCHEMA`, not a replacement character. A player connection stays up after that player-line refusal. A hello that is `SCHEMA` or `LINE_TOO_LONG` is refused and closed. A socket that has not sent a complete hello within 10 seconds is closed, and nothing is committed.

A player connection may send 5 lines per second, and it may burst to 10. When that allowance is used, the connection waits until the next line is allowed before it reads again. The line is kept. The wait is on that connection alone, so a flood does not delay another seat. The GM seat is not rate-limited. One GM is the trusted seat, and its commits already run one at a time on the shard thread.

An HTTP POST body on the agent door is at most 1 MiB. A longer body is refused with 413 and does not take the seat.

Each player socket and the HTTP GM session has an outbound queue of at most 256 frames and at most 64 KiB. An empty queue still takes one frame of any size, so one welcome is delivered. A frame's bytes count until it is written to the client. The shard thread does not wait on a send. A full queue means that client is not reading. That connection is dropped and its seat is released. A dropped GM stops reading, so a later tool call does not run. Other seats keep their frames. This queue does not start a turn.

The bind does not grow past loopback. `--addr 127.0.0.1` and `--addr 127.0.0.1:<port>` are the only legal values. Port `0` is legal and asks the operating system to choose a port. `--addr` is one global flag, and every command checks it. The default address is `127.0.0.1:9737`. Any other host, including `0.0.0.0`, a LAN address, or a hostname, is `NOT_IN_SPEC` before the pack opens, for every command. There is no account store.

One GM seat has one agent door. `spook run` serves streamable HTTP at `/mcp` on `127.0.0.1:<game port + 1>` when the game port is fixed. The default game port `9737` makes that `http://127.0.0.1:9738/mcp`. `--addr 127.0.0.1:0` asks the operating system for the game port, and the agent door is an operating-system port of its own, not the game port plus one. `--mcp-http 127.0.0.1:<port>` overrides the door, including `--mcp-http 127.0.0.1:0`. The override uses the same loopback parser as `--addr`, so any other host is `NOT_IN_SPEC` before the pack opens. After both sockets are bound, and before it serves, `spook run` prints one line on stdout: `{"kind":"listening","game":"127.0.0.1:<game>","door":"127.0.0.1:<door>"}`. `spook gm` and `spook ai-gm` dial `http://127.0.0.1:<game port + 1>/mcp` unless `--mcp-http` names the door. A port-0 table's reported door is that name. The game port accepts only player hellos. A line `gm` on that port is `NOT_IN_SPEC` and writes nothing.

Initialize on the agent door claims the GM seat. A second GM is `SESSION_FULL`. On HTTP that refusal is a JSON-RPC error whose message contains the token. A refused seat prints a sentence and the token on stderr, writes nothing to stdout, and exits 1. `SESSION_FULL` prints `The table is full. [SESSION_FULL]`. HTTP DELETE, or the session ending, releases the seat. A request whose `Host` is not `127.0.0.1` is refused with 403. That includes `localhost`, so a client must address the endpoint as `http://127.0.0.1:<port>/mcp`. A request whose `Origin` header is present and is not `http://127.0.0.1` is refused with 403. A missing `Origin` is accepted. There is still no auth. A POST body larger than 1 MiB is refused with 413. An `MCP-Protocol-Version` header that names an unknown version is refused with 400. A missing header is accepted. `spook/event` notifications may be ignored by a generic client. `session_status` with `since` is the pull path.

## Host monitor

A host monitor is not shard behavior.

`idle_digest` takes a host viewer or a player viewer. The host sees effect rows and clocks. A player sees only their messages and owned clocks. `spook status` prints the host digest. Player resume is the player digest. The host digest is not a player resume, and it does not write the shard.

## Player client

`spook play <pack>` sits a player seat and dials `--addr`. `--as player:<id>` picks the actor. Without it, the client sits the pack's start PC. Each client holds one distinct PC. The player socket wire is unchanged: the client only changes how it prints frames.

By default each frame is decoded and printed as readable text, one rendering per `PlayerMessage`, with no protocol JSON. A say is the speaker and the text: `GM: evening`, `You: hello` for the seated player, `Mira: the lamp is lit` for `npc:mira`. `queued` is `Your action is queued.` and does not print the queue id. `check` is `Your roll succeeded.`, `Your roll partly succeeded.`, or `Your roll missed.`. `perceive` prints `You are in <place>.`, `Here: <actors>`, and the `You know:`, `Notes:`, `Clocks:` (`Assassination debt: 2/6, paused`), `Items:`, and `Exits:` sections, skipping empty ones. An exit line is `name → place`, plus `N min` and `locked` when those apply. It does not print fact bodies, unknown mentions, habits, or ids. `resume` is `While you were away:` followed by one line per player digest row. A message row is its text, without the message id, and an owned clock row reads like a perceive clock (`Assassination debt: 2/6, paused`). A row the client cannot read is shown as it came. A refusal is a sentence and the token, such as `The table is full. [SESSION_FULL]`, `That character is already taken. [SEAT_TAKEN]`, `That line is too long. [LINE_TOO_LONG]`, `You cannot afford grit. [CANT_AFFORD]`, `There is no exit that way. [NO_EXIT]`, `You are busy. [BUSY]`, on stderr. A locked exit prints its `locked_text` and `[NO_EXIT]`. A token without a sentence is `The table refused that. [<TOKEN>]`. Ids are shown as names: a leading `npc:` or `player:` is dropped, `_` becomes a space, and the first letter is capitalized. A line the client cannot decode is written to stderr unchanged.

`spook play --raw` prints each protocol frame as one JSON line on stdout, as the socket sent it. A `refused` frame prints only its token on stderr.

A refusal before the first `perceive` exits 1. After the first `perceive`, a refusal is printed and play continues. stdin lines go to the socket unchanged.

## Host

`spook ai-gm <pack>` sits the single GM seat, the same HTTP seat as `spook gm`. `--addr` is the game address, which defaults to `127.0.0.1:9737`, and the client dials `http://127.0.0.1:<that port + 1>/mcp` unless `--mcp-http` names the door. A port-0 table reports its door on the listening line, and that address is the `--mcp-http` value. It starts neither `spook run` nor `spook play`. A second GM gets `SESSION_FULL` and the host exits. `spook run` is the only listener. `--mcp-http` on `spook run` and on the GM clients names that same door. It is not a second server.

`spook ai-gm` with no pack, or with any extra argument, is a usage error before a dial. A refused seat exits 1. A shard that is not listening exits 1 and leaves the pack bytes as they were.

On start the host reads `SPOOK_GM_KEY`, `SPOOK_GM_URL`, and `SPOOK_GM_MODEL`, in that order. The first one missing: it writes `<NAME> is unset` to stderr and exits 1 before it dials. There is no default URL. The client posts to `{SPOOK_GM_URL}/chat/completions` with the key as a bearer token. The key is not an argument and not a pack file.

The host calls `pack_rev` and `load_canon` on the pack path. It does not call `World::open`, `apply_effect`, or `write_commit`. After initialize it calls `session_status` with no arguments and compares `pack_rev`. A mismatch disconnects and exits 1. A match writes `listening` to stderr. stdout stays empty.

A non-empty queue is committed once after the model's turn. Talk alone does not tick. If the model call fails, the host skips the model's says and intents and still commits a queue the player already filled. The seat stays up. Each model call waits at most 60 seconds and at most 10 seconds to connect, and a timeout, a connect error, HTTP 429, or a 5xx is tried once more after 2 seconds. A commit that rolled dice gets one say-only follow-up. That follow-up does not queue and does not commit. `commit_round`'s result includes `checks` (`faces`, `stat`, `total`, `target`, `outcome`). The same roll notices are still sent as `check` events on the GM session. The player wire is `{"kind":"check","outcome":"success"|"partial"|"miss"}`. Faces stay off the player socket.

An addressed actor who is not in the room gets one table say, `They are not here.`, and no model call. On an actor-addressed wake, a say is kept only when its address is `actor` and its actor id is the one who was addressed. On a table or place wake, a say is kept only when it is `table`, or `place` naming the speaker's place. A queued wake, or a sit wake for a non-empty queue, has no addressed actor. The host censuses the PC in `presence.actors_present` the way a table wake censuses the speaker, and keeps a say only when it is `table`, or `place` naming that PC's place. If that list has no PC, the host skips the model, submits no say, and still runs the commit step. One model reply may submit at most two says and one intent, after that rule. A say the rule rejects does not consume the cap. An intent whose move is not in the menu is dropped before the cap.

Speech already on the tape at sit is not answered. High water starts at the current last seq. A queue that is already waiting still commits.

The POST body is the bounded prompt: the last 16 says this process has handled, the new player lines, the move menu, the stripped views, and roll faces on a follow-up. A restart remembers nothing. A player view that is only a room census drops `known_facts`, `messages`, and `unknown_mentioned`. An NPC view keeps each fact's `id` and `text`. Every fact `body` is dropped. The prompt does not receive a canon or a cast body. The tape file stays on disk under the pack. The POST does not carry the tape file, `save.yaml`, or a cast file.

## Firewall, lint, and digest

`allows` means the fact is known, or it is not hidden and it is on the viewer's place, or it is not hidden and it is on a co-located owner's cast or place. Hidden facts never leak through place or cast prose.

`perceive` drops a message whose `text` contains the text of a fact that viewer is not allowed to know, the same way it drops a message with an unallowed `fact_ids` entry. `apply` does not refuse that message. Who can see a place note changes as actors move, so the check belongs at perceive time. `spook check` reports that template with `HIDDEN_FACT_IN_TEXT`, the file, and the line.

`spook audit <pack>` replays the journal once from its base into one temp directory and deletes that directory. At every committed state, including the base, it renders the player perceive line for every PC. It fails when that line contains the text of a hidden fact that PC does not list in `known_facts`, or that fact's id as a JSON string, so a shorter id does not match a longer one. That set is read from the canon and the save. Audit does not call `allows`. A player check notice is an outcome or a refusal token and carries no fact id or text. The journal does not store the notice, so the engine line audit scans is the perceive line. One `pass <pc>` or `FAIL <pc>` line is printed per PC, then each offending perceive line as `state_rev <r> <pc> <line>`.

Speech is the table's choice. For each tape line and each recipient, a hidden fact's text the recipient did not know when the line was written is a note: `note seq <n> speaker <from> recipient <id>`. Notes do not change the exit code. `--strict` makes them fail. Tape lines are matched to the earliest journal state with the same tick index, and the output says `notes follow the journal state at the tape line's tick`. When a line cannot be matched, notes use the final state, and the output says `notes use the final state`. Exit 0 when no perceive line leaks and either there are no notes or `--strict` is off. Exit 1 on a perceive leak, or on a note when `--strict` is set. A path with no `pack.yaml` or no `state/save.yaml` prints `NOT_IN_SPEC` and exits 1. The command writes nothing under the pack, takes no lock, and binds no port.

`perceive` is budgeted. The budget is 800 tokens unless the caller passes a budget. `lint` scans Markdown bodies for hidden fact text, and `spook check` also scans message template text. `strip_body` deletes the body, leaves frontmatter, and the pack still ticks. Those functions stay in firewall, perceive, digest, and lint.

Load fails with `SCHEMA` if `places.actors` is edited apart from `actors.place`. Emit rebuilds occupancy from `actors.*.place`.

Faces of a roll go to the GM socket only. The player socket gets `{"kind":"check","outcome":"success"|"partial"|"miss"}` or `{"kind":"refused","code":"..."}` for that player's own roll. An NPC's roll sends no player frame. No faces. The tick JSONL lists `resolve_check` and `apply_effect` without faces. A refused tick records `rejected[].reason` as the token: `BUDGET` lowercased, `empty_perceive` as that word, a cost gap as `CANT_AFFORD` plus the meter or item id, and every other token uppercase.

## Non-goals

Still not built, and not implied by a missing sentence above:

- A cargo feature named `llm`, or an NPC `from` on the tape.
- Parsing speech into `known_facts`, or a whisper that grants a fact.
- A non-loopback bind, accounts, or a second GM.
- An ack cursor, or a per-client outbound queue whose purpose is to make a host event start a host turn. The seat's send queue only drops a slow reader.
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
| Client patches YAML | The play door queues an intent or appends speech. The CLI has no path that opens `save.yaml` for a client write. |
| GM calls `apply_effect` | The tool is not a sitting handler. Effects come from expand inside a `Draft`. Only `commit_round`, `commit_presence`, and `migrate_pack` rename files. |
| A summarizer writes memories | No LLM crate. Memory lines are written inside the commit, from a `memory` effect, after `allows`. |
| Speech copied into `known_facts` | The tape is not an input to expand. No whisper. Inbound model text is not a field on `Intent`. |
| Host digest reused as the player resume | `idle_digest` takes `DigestViewer::Host` or `DigestViewer::Player`. Different rows. |
| Hand edit of `save.yaml` | `HAND_EDIT` until `spook migrate`. |
| Replay applies JSONL effects as the new state | Replay does not install the log as `Now`. It re-expands the recorded intents, calls `rules::roll` again from each roll's key, and then calls `apply_effect`. A presence record restores presence. A migrate record restores `pack_rev`. Neither is an effect list. |
| Two ticks in flight | One lock around the tick run and `commit_round`. The second caller gets `COMMIT_CONFLICT`. Tick code is synchronous on the shard thread. |
| `patch_entity` writes canon and the live shard | Not a sitting tool. A canon byte change mismatches `pack_rev` until migrate. |
| New effect class in a handler | `ILLEGAL_CLASS` at the parse boundary. `EffectClass` has seven variants. |
| A meter whose stored pair is outside the definition | `SCHEMA` at load. Apply clamps when the sum fits in `i64`. `OUT_OF_RANGE` leaves the record unchanged when the sum does not. |
| `perceive` with no `as`, or `as=gm` | `ViewerId` has no GM variant. Sitting perceive rejects anything but `player:` and `npc:`. |
| `places.actors` edited apart from `actors.place` | Load fails `SCHEMA`. Emit rebuilds occupancy from `actors.*.place`. |
| Feature `llm` | No such feature. |
| Tokio task per shard inside the kernel | Kernel and tick cannot name the runtime. The scan stays. |
| `rand::thread_rng` or a hand-rolled die | `rules::roll` is Blake2s. One copy, in `rules/roll.rs`. |
| Markdown body used as an item count or a hidden fact | `strip_body` and `lint`. |
| A second snapshot beside `save.yaml` | `state/journal/base.yaml` is replay input written in the commit. It is never read as the live save. One emit path still writes `save.yaml`. |
| Non-loopback bind, accounts, second GM, fourth player, held PC | `NOT_IN_SPEC` for a non-loopback bind. No account store. `SESSION_FULL` for a second GM or a fourth player. `SEAT_TAKEN` for a held PC. |
| Hidden fact in a perceive body | `allows` plus lint. |
| A client sends an endless line, floods lines, or stops reading | Hello and player lines cap at 8 KiB (`LINE_TOO_LONG`). An HTTP body over 1 MiB is refused with 413. Invalid UTF-8 on a hello or player line is `SCHEMA`. A hello that does not finish in 10 seconds is closed and commits nothing. A player connection allows 5 lines per second with a burst of 10, and it waits for the next allowance instead of dropping the line. The GM seat is not rate-limited. An outbound queue of at most 256 frames and 64 KiB drops a client that stops reading, and the shard thread does not block on that send. |

Privacy: the pack is local. There is no account, no connection secret, and no telemetry. The tape holds table speech on disk under the pack. That is the product log. The tape file stays on disk. `spook ai-gm` sends table text off the machine only as the body of its chat-completions POST: the last 16 says this process has handled, the new player lines, the move menu, the stripped views, and roll faces on a follow-up.
