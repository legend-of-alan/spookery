# ADR 0005: Closed effect enum

- Status: Accepted
- Date: 2026-10-02
- Source: docs/architecture_v0.md lines 558–620 and 626–635

## Context

`docs/architecture_v0.md` states "`Effect` is a Rust enum. The YAML/JSON boundary parses `class` plus `payload` into that enum. Unknown `class` or an extra payload key returns `ILLEGAL_CLASS` and never becomes an `Effect`." It states "There is no `inventory` class, no `xp` class, and no `season_advance` class." Idle names and live-only names are separate lists. It states "`MoveName` is a newtype validated against `moves.yaml`. The idle expansion is a `match` on the eleven idle names." It states "There is no move expression language."

## Decision

Envelope, every variant: `as_viewer: ViewerId`, `intent_id: IntentId`. JSONL and MCP write `as_viewer` under the key `as`.

The `class` column is these twelve classes:

| `class` | Payload |
|---|---|
| `clock` | `clock_id`, `delta: i64` |
| `need` | `actor_id`, `key`, `delta` |
| `stock` | `place`, `sku`, `delta` |
| `location` | `actor_id`, `from`, `to` |
| `relationship` | `a`, `b`, `delta` in `-1, 0, +1` |
| `reputation` | `actor_id`, `faction`, `delta` in `-1, 0, +1` |
| `memory` | `actor_id`, `fact_ids`, `tags`, `text?` |
| `rumor` | `rumor_id`, `fact_ids`, `from`, `to`, `heat` 1–3 |
| `resource` | `actor_id`, `key`, `delta` |
| `flag` | `actor_id`, `key`, `value: bool` |
| `proposal` | `proposal_id`, `kind`, `target`, `summary` |
| `message` | `message_id`, `to`, `place?`, `fact_ids`, `text` |

"There is no `inventory` class, no `xp` class, and no `season_advance` class. Catch-up overflow files one `proposal` with `kind: other` and summary `season_advance` (§7)."

Intent fields:

| Field | Values |
|---|---|
| `id` | Caller-supplied for a player intent and for a library test. Kernel-minted for derivation and the season proposal (pin P37). Unique inside the txn |
| `tick_index` | The tick that will consider it |
| `actor_id` | |
| `actor_kind` | `npc`, `player`, `system` |
| `source` | `kernel`, `llm`, `player`, `system`. v0 derivation emits only `kernel`. `llm` exists so a recorded line can be replayed. v0 never creates an `llm` intent |
| `move` | Loaded allowlist. Unknown → `UNKNOWN_MOVE` |
| `target` | At most one of `place`, `actor`, `clock`, `rumor`. Two set → `SCHEMA` |
| `args` | Move-specific. Extra keys → `UNKNOWN_KEY` |
| `rationale_code` | String, audit only, not mechanical |
| `llm_say` | Persisted as null. Inbound text dropped |

"Idle moves: `wait`, `travel`, `work`, `restock`, `rest`, `pay_upkeep`, `drift_relationship`, `spread_rumor`, `leave_note`, `file_proposal`, `advance_clock`."

"Live-only names, rejected with `LIVE_ONLY` when `presence.humans == 0`: `talk`, `attempt`, `attack`, `give`, `take`."

"Pin P37. A kernel-minted id is the lowercase hex of `BLAKE2s-256("{shard_id}|{tick_index}|{actor_id}|{move}")`, 64 characters. It is minted when the intent object is built, before `filter_blocklist` and before `prioritize`, so a later rejection keeps that id. `kernel_intent_id("cap", 1, "n2", "work")` is `daa186f67613ae8ea1275b3fd03c38d52eea99e976235e499b5c2b0853c0d819`. The CLI mints a UUID v4 for a play line. Library tests and trace B set the id themselves. Replay copies the stored id and does not mint again. A duplicate id inside the txn is `SCHEMA`."

"`expand` returns `NOT_IN_SPEC` for those five even during a session. They may be declared in `moves.yaml` so the fixture matches the move list. They do not mutate inventory. Do not add an effect class to make `give` work."

Move → effect:

| Move | Effects | Args the kernel reads |
|---|---|---|
| `wait` | none. Does not count against `max_npcs_acting` | none |
| `travel` | one `location` | `target.place` is `to`. `from` is the live place |
| `work` | `need` and/or `resource` only as a canon `work` block declares. Missing block → zero effects. The intent still applied | the block |
| `restock` | one `stock` | `sku`, `delta` |
| `rest` | one `need` on key `rest` | `delta` |
| `pay_upkeep` | `resource` and/or `need` | `key`, `delta` |
| `drift_relationship` | one `relationship` | `target.actor` is `b`. `delta` from the drift formula below |
| `spread_rumor` | one `rumor` | `target.rumor` or args `rumor_id`, `to` |
| `leave_note` | one `message` | message fields |
| `file_proposal` | one `proposal` | proposal fields |
| `advance_clock` | one `clock` | `target.clock`, `delta` default 1 |

"Truncate to `max_effects_per_intent` (default 4). Do not invent a further effect from `rationale_code`, `llm_say`, or a Markdown body."

"Illegal combinations are enums plus private fields, not booleans the caller can mix."

- "`Effect` is an enum. A string class does not cross the parse boundary."
- "`ViewerId` is `Player(ActorId)` or `Npc(ActorId)`. It has no `Gm` variant."
- "`Txn` is not `Clone`, not `Send`, and has no public "write files" method."
- "`Canon` is owned by `Shard` behind `&`. Tick code cannot get `&mut Canon`."
- "`MoveName` is a newtype validated against `moves.yaml`. The idle expansion is a `match` on the eleven idle names. A new mechanical move is a new match arm plus a conformance test. There is no move expression language. Eligibility is the §4.3 predicate, not a string DSL."
- "Adding a name to `moves.yaml` without a match arm fails at expand: `UNKNOWN_MOVE` if the name is absent from the allowlist, `NOT_IN_SPEC` if it is listed and has no arm. The fixture lists exactly the sixteen names in §5 (eleven idle plus five live-only)."

## Consequences

- Unknown `class` or an extra payload key returns `ILLEGAL_CLASS` and never becomes an `Effect`. A string class does not cross the parse boundary.
- "There is no `inventory` class, no `xp` class, and no `season_advance` class."
- Live-only names `talk`, `attempt`, `attack`, `give`, and `take` are rejected with `LIVE_ONLY` when `presence.humans == 0`.
- The source states "`expand` returns `NOT_IN_SPEC` for those five even during a session." "They do not mutate inventory. Do not add an effect class to make `give` work."
- "There is no move expression language." `MoveName` is a `match` on the eleven idle names.
- "Truncate to `max_effects_per_intent` (default 4). Do not invent a further effect from `rationale_code`, `llm_say`, or a Markdown body."
- `ViewerId` has no `Gm` variant. `Txn` has no public "write files" method. Tick code cannot get `&mut Canon`.

## No existing proof

Per-class failure rows that no feature card names:

- `clock`: "`PAUSED_CLOCK` if `paused` or (`requires_presence` and `presence.humans == 0`). `UNKNOWN_KEY` if the clock is absent. `OUT_OF_RANGE` if `value + delta` is outside `[0, max]`. Needs clamp; clocks do not."
- `need`: "`UNKNOWN_KEY` if `key` is not in the profile needs list. Clamp to the profile range when a range is present. No range in canon → leave the value, still require the key to be listed."
- `stock`: "`NOT_OWNER` unless `place` is in `range`, appears on a schedule row, and is a key of `stock_rate`. `OUT_OF_RANGE` if `abs(delta)` exceeds `stock_rate[place][sku]` or the result would be `< 0`."
- `location`: "`from` must equal live place. `to` must be in profile `range` and on the schedule graph. Otherwise `OUT_OF_RANGE`. Idle `location` whose `actor_id` is a PC → `NOT_OWNER`."
- `relationship`: "At most one apply per pair per world-day. If either stored-busy or `last_interaction_world_time` is within 1 world-day → `SKIPPED` (no-op, remainder of this intent continues)."
- `reputation`: "At most one per faction per world-day, tracked by `reputation.<faction>.last_world_time`. Second apply → `SKIPPED`."
- `memory`: "`text` ≤ 240 chars. `actor_id` must be the causing actor (`NOT_OWNER` otherwise). Fact ids and `text` may only mention facts in `known_facts` after earlier effects in this txn. Else `ILLEGAL_KNOWLEDGE`."
- `rumor`: "Recipient must already be allowed each fact. Else `ILLEGAL_KNOWLEDGE`. Counts against `max_rumor_hops`."
- `resource`: "`UNKNOWN_KEY` if `key` is not on the profile (NPC) or the live sheet (PC). Result `< 0` → `OUT_OF_RANGE`."
- `flag`: "Key must be listed on the profile. `key: busy` writes `ActorLive.busy`. Any other allowed key writes `flags`."
- `proposal`: "`kind` is `delete_place`, `patch_entity`, or `other`. Writes a proposal file inside the txn. Does not edit the target."
- `message`: "Queued onto `now.messages`. Duplicate id → `UNKNOWN_KEY`. Not narration."

## Superseded

The class list stays. `docs/architecture_v0.1.md` §2 states "The closed effect enum stays the v0 §5 list. No new class." `docs/architecture_v0.2.md` section "Still unbuilt for the minor" states "No new effect class. `give`, `take`, and an item changing hands are v0.3."

`docs/architecture_v0.2.md` section pay_upkeep replaces the idle-list name `pay_upkeep`, the `pay_upkeep` move-effect row, and the fixture sentence that counts that name among the sixteen: "`pay_upkeep` is retired. It is absent from `packs/oneshot-docks/rules/moves.yaml` and from `rulesets/moves.yaml`. `expand` has no arm for that name. Derivation does not match it. The v0 idle list, the move-effect row, derivation step 1, and the typed `pay_upkeep <key> <delta>` line no longer hold. Need floors stay out of the canon schema."

`docs/architecture_v0.1.md` §1 replaces "`expand` returns `NOT_IN_SPEC` for those five even during a session" for `talk` and for `attempt`. It states "`talk` is a player `say`, not an intent. Zero effects. It is refused `LIVE_ONLY` when `presence.humans` is 0. It stops returning `NOT_IN_SPEC` once a player is seated." It states "`attempt` stays an intent." It states "`attack`, `give`, and `take` stay `NOT_IN_SPEC`." §5 states "`attempt` stays `LIVE_ONLY` when `humans == 0`, same as the other four live-only names. `attack`, `give`, and `take` still expand to `NOT_IN_SPEC` and do not move inventory." It states "`talk` is not an expand arm and is not stored as an intent." It states "A queued intent whose move is `talk` is a listed name with no arm, so `expand` returns `NOT_IN_SPEC` (v0 §6). That path is not speech."
