# Spookery v0.2 architecture

Status: Draft. The demo below is true. The bare tag `v0.2` is cut on patch `v0.2.6`. Patch `v0.2.0` is tag `v0.2.0` at `913eeb6`. Patch `v0.2.1` is tag `v0.2.1`. Patch `v0.2.2` is proved. Patch `v0.2.3` is proved. Patch `v0.2.4` is proved. Patch `v0.2.5` is proved. Patch `v0.2.6` is proved. A later patch is tagged `v0.2.N` when that patch's proofs are green. Where this file names a change, it wins over `docs/architecture_v0.1.md`, then `docs/architecture_v0.md`. Where this file does not name a change, v0.1 holds, then v0. A rule written in none of them is out of scope.
License: CC BY 4.0, same as the v0 spec. Runtime code under /src stays AGPL-3.0-or-later.

This file is the draft spec for the v0.2 minor. `docs/architecture_v0.1.md` and `docs/architecture_v0.md` stay frozen. This file does not copy them. Do not edit those files. v0.2.2 is the first edit of `packs/oneshot-docks`.

A session reads this file and implements the open patch. A pin this file quotes is the pin the session uses.

---

## Demo

When v0.2.6 is green, a pack can define a player action as one row. The row names a verb. It says the token after the verb is absent, a place id, or an actor id, and which field of an existing effect that token fills. The effect lands on the speaker or on the named actor. The row may name a check, with one effect list for success and one for failure, using classes that already apply. Another actor's action, queued before `commit_turn`, may oppose that check. The row may name an item id the sheet must already hold, and a missing item refuses the action before any effect. No new effect class appears in this version.

v0.2.2 is a scene a person can show before that sentence is true. Rowan starts in `docks_shop`, types `travel docks_street`, and after the tick the player can see Rowan on the street. Showing that scene does not cut the bare tag.

The pack does not choose the place or the actor. The player types that token. Friends and foes are not a field in this version.

---

## Patches

v0.2.0, v0.2.1, v0.2.2, v0.2.3, v0.2.4, v0.2.5, and v0.2.6 are proved. The later rows are names. Their full text is written when a session is about to implement that patch. A later patch may be named before the previous one is green. Two patches proceed together only when they do not both edit the player read loop, the shard thread, or `tools/call`.

| Patch | What becomes true |
|---|---|
| v0.2.0 | Proved, below. Tool keys, `travel <place>` as a legal line, one wire codec. A successful arrival in `oneshot-docks` is not playable. |
| v0.2.1 | Proved, below. A pack row emits one `flag`. |
| v0.2.2 | Proved, below. The docks `travel` row uses a place token and one `location` effect. Rowan starts in `docks_shop`, types `travel docks_street`, and after the tick the player can see Rowan on the street. Speech and `wait` still work. |
| v0.2.3 | Proved, below. The row may name a check. The acting player sees success or failure, and the effect templates run either way. |
| v0.2.4 | Proved, below. A success list and a failure list. A `flag` on the success list is written only when the roll succeeds. |
| v0.2.5 | Proved, below. An action the named actor queued before `commit_turn` supplies the opposing check. |
| v0.2.6 | Proved, below. The row may name an item id. A sheet that lacks it refuses the action, queues nothing, and leaves the item where it is. This patch meets the demo. |

`attack` is a pack row once v0.2.4 exists: an actor token, a check, and a `resource` delta on a key the sheet already lists. `defend` is that actor's queued row at v0.2.5. This minor does not require `oneshot-docks` to contain either row. `give`, `take`, and an item changing hands are v0.3. Idle moves such as `work` keep the match arms they have.

---

## v0.2.0 — baseline

Tag `v0.2.0` is `913eeb6`. The behavior below is law. The gate letters A–C belong to this patch and are proved.

GM `tools/list` gives each name its own description. The sitting allowlist stays the v0.1 order. The five live tools name the keys their handlers already require.

| Tool | Required | Also accepted |
|---|---|---|
| `say` | `address`, `text`. `place` when `address` is `place`. `actor` when `address` is `actor`. | A supplied `from` stays `SCHEMA`. `table` still takes neither `place` nor `actor`. |
| `perceive` | `as` | `focus`, `budget_tokens`. A missing `as` stays `UNSCOPED_PERCEIVE`. |
| `commit_turn` | `include_tick` | `session_id`, a string when present. |
| `session_status` | none | `since`. `{}` stays legal. Any other key stays `SCHEMA`. |
| `intent` | `actor_id`, `actor_kind`, `source`, `move` | `id`, `tick_index`, `target`, `args`, `rationale_code`, `llm_say`. `source` stays `player`. |

A missing required key stays `SCHEMA` or `UNSCOPED_PERCEIVE`, as today. `npc_act` stays `LLM_DISABLED`. `session_open`, `patch_entity`, `resolve_check`, `apply_effect`, `recall`, and `session_save` stay `NOT_IN_SPEC`. Each name on the list has a visible handler or a visible refusal. A name off the list stays `NOT_IN_SPEC`.

`spook run <pack> --mcp pack-table` does not gain `say` and does not gain a `tools/call` body. The stdio server stays list-only. `v0_c01_load_mcp` stays green.

`travel <place>` is one place id, on the same queue path as `wait`. With a GM seated, the line returns `queued`, does not advance `world_time`, and does not move the actor until `commit_turn`. With no GM, it commits one live tick. A missing place is `SCHEMA` and queues nothing. Any other shape of a `travel` line is `SCHEMA` and queues nothing. The applying tick uses the location effect `expand` already returns. A place the pack does not have stays that tick's existing kernel refusal. No new effect class. NPC derivation is unchanged.

`attack`, `give`, and `take` stay `NOT_IN_SPEC` and queue nothing. Player lines on this patch are `say`, `talk`, `wait`, `attempt <check> <target>`, and `travel <place>`.

The same PC drops and connects again. That connect is a new `session_open` for the same actor, as v0.1. Scrollback returns. `humans` is 1. Intents already queued are still queued, once each. Reconnect does not apply them. No new session-id rule. No durable queue.

The GM socket does not receive tape bytes before MCP `initialize`. `initialize` does not append the tape. GM scrollback is `session_status` with `since`. Player scrollback on hello stays the v0.1 lines. A `say` tool result is still followed by its `spook/event`, not preceded by it. A player `say` still reaches the seated GM as a `spook/event`. `spook gm --stdio`, when nothing is listening, still exits 1. Stderr is the operating-system error. Stdout is empty.

Proofs `v02_p01` through `v02_p05` are `tests/conformance/v02/gates.rs`, compiled through `tests/sitting.rs`. v0 proofs compile through `tests/v0.rs` from `tests/conformance/v0/`. v0.1 proofs compile through `tests/sitting.rs` from `tests/conformance/v01/`. Shared helpers are `tests/support.rs`. Those functions stay as they are. `store.rs` stays one file.

---

## v0.2.1 — action row

Proved. The behavior below is law. Proofs `v02_p06`, `v02_p07`, and `v02_p08` are `tests/conformance/v02/gates.rs`, compiled through `tests/sitting.rs`. They do not rewrite `v02_p01` through `v02_p05`. The fixture pack is one the proof builds. `packs/oneshot-docks` is unchanged.

A move in `rules/moves.yaml` that carries `effects` is a player action row. `schema/moves.schema.json` accepts the keys below and still rejects an unknown key. A move without `effects` keeps today's arm, including `travel`. Idle moves such as `work` stay that match.

```yaml
- name: mark
  token: actor
  on: named
  effects:
    - class: flag
      key: marked
      value: true
```

`token` is `none` or `actor`. `on` is `speaker` or `named`. The load accepts `token: none` with `on: speaker`, and `token: actor` with `on: named`. Any other pair is `SCHEMA` at pack load. `token: place` is `SCHEMA` at pack load until v0.2.2. A template `class` other than `flag` is `SCHEMA` at pack load until v0.2.2. `effects` is a non-empty list. Each item is `class`, `key`, and `value`. `value` is a boolean. `key` is not `busy`. An empty list, a missing key, or an extra key is `SCHEMA` at pack load.

The names `say`, `talk`, `wait`, `attempt`, `attack`, `give`, and `take` reject an `effects` list at pack load. `travel` rejects an `effects` list at pack load until v0.2.2. The optional `check` key on a row is v0.2.3. A move without `effects` may still carry `check`, and that key still does nothing.

`say` and `talk` stay speech. A player verb that names a row is on the same queue path as `wait`. The line is the verb plus one actor token when `token` is `actor`, and the verb alone when `token` is `none`. A missing token, or an extra token, is `SCHEMA` and queues nothing. With a GM seated, the line returns `queued`, does not advance `world_time`, and does not write the flag until `commit_turn`. With no GM, it commits one live tick. A verb that is not `say`, `talk`, `wait`, `attempt`, `travel`, and not a row, stays `NOT_IN_SPEC` at the classifier. `attack`, `give`, and `take` stay `NOT_IN_SPEC` and queue nothing.

Expand of a row builds one `flag` per template. `on: speaker` writes the acting actor. `on: named` writes the typed actor id. The typed word is not looked up at parse time. An actor the pack does not have stays that tick's existing refusal.

A `flag` whose `key` is listed in that actor's cast `tick.flags` writes `actors.<id>.flags.<key>`. A key that is not listed returns `UNKNOWN_KEY` and writes nothing. `busy` stays the kernel's own flag and is not a row key. No new effect class. NPC derivation does not emit a move that carries `effects`.

---

## v0.2.2 — the street

Proved. The behavior below is law. Proof `v02_p09` is `tests/conformance/v02/gates.rs`, compiled through `tests/sitting.rs`. It uses `packs/oneshot-docks`. No new effect class. Showing the scene does not cut the bare tag.

`token: place` pairs with `on: speaker`. The typed place fills the location field `to`. `from` is the acting actor's current place. The effect's actor is the speaker. A travel row expands to the `Effect::Location` the travel arm already builds.

The docks row is one template, `class: location`, with no key and no value. A location template that carries key, value, or any other key is `SCHEMA` at pack load. A place token with a flag template is `SCHEMA`. A place token with `on: named` is `SCHEMA`. An actor or none token with a location template is `SCHEMA`.

`travel` is the one reserved name that may carry effects, and only in that shape. `say`, `talk`, `wait`, `attempt`, `attack`, `give`, and `take` still reject an effects list. A check on a row is v0.2.3. A travel move with no effects keeps today's arm.

The player line stays `travel <place>` on the same queue path as `wait`. A missing place or an extra token is `SCHEMA` and queues nothing. With a GM seated, the line returns `queued`, does not advance `world_time`, and does not move the actor until `commit_turn`. With no GM, it commits one live tick. `world_time` advances by 5.

`pc_rowan` has a cast document and a tick profile. `range` and `schedule` include `docks_street`. `mira_rooms` is off that range. `eligible` is false, so derivation does not enqueue a row for this profile. `travel mira_rooms` still queues and refuses `OUT_OF_RANGE` without moving Rowan.

The player perceive line is `kind` perceive, `place`, and `visible_actors` for everyone in that place, including `pc_rowan`. After the tick that line shows `docks_street`. Speech and `wait` still work.

---

## v0.2.3 — the check

Proved. The behavior below is law. Proof `v02_p10` is `tests/conformance/v02/gates.rs`, compiled through `tests/sitting.rs`. It does not rewrite `v02_p01`, `v02_p02`, `v02_p03`, `v02_p04`, `v02_p05`, `v02_p07`, `v02_p08`, or `v02_p09`. The fixture pack is one the proof builds. `packs/oneshot-docks` is unchanged. No new effect class.

A row may name `check`. The value is a check id. `schema/moves.schema.json` accepts that key on a row and still rejects an unknown key. A row that does not name `check` does not roll.

On the tick that applies the row, the named check is resolved with target 0. Success is total at least the target. The acting player sees `{"kind":"check","success":true}` or `{"kind":"check","success":false}`. The effect templates run on both outcomes. A success list and a failure list are v0.2.4. A check id the pack does not have is that tick's existing refusal, and the templates do not run.

The player line stays the verb plus the row's token. A missing token or an extra token is `SCHEMA`. With a GM seated, the line returns `queued` and the roll waits for `commit_turn`. With no GM, it commits one live tick.

---

## v0.2.4 — the two lists

Proved. The behavior below is law. Proof `v02_p11` is `tests/conformance/v02/gates.rs`, compiled through `tests/sitting.rs`. It does not rewrite `v02_p01`, `v02_p02`, `v02_p03`, `v02_p04`, `v02_p05`, `v02_p06`, `v02_p07`, `v02_p08`, or `v02_p09`. `v02_p10` still proves that one `effects` list runs on both outcomes. The fixture pack is one the proof builds. `packs/oneshot-docks` is unchanged. No new effect class.

A row may name `success` and `failure`. Each is a non-empty list of templates. A template on `success` is applied only when the roll succeeds. A template on `failure` is applied only when the roll fails. A flag on `success` is written only when the roll succeeds. The `effects` list still runs on both outcomes. A row that does not name `check` does not roll, and `success` and `failure` do not write.

The check is still resolved with target 0 when the named actor has no queued action. v0.2.5 supplies the opposing check. The acting player sees `{"kind":"check","success":true}` or `{"kind":"check","success":false}`. A check id the pack does not have is that tick's existing refusal, and the lists do not run.

`schema/moves.schema.json` accepts `success`, `failure`, and a `resource` template, and still rejects an unknown key. A flag template is `class`, `key`, and `value`. A `resource` template is `class`, `key`, and `delta`. `resource` is the class the kernel already applies.

`attack` is a pack row: `token: actor`, `on: named`, a check, and a `resource` delta on a key the sheet already lists. The delta is a template on `success` or `failure`. A flag `effects` list on `attack` is still `SCHEMA` at pack load. `attack` with no row stays `NOT_IN_SPEC` and queues nothing. `defend` stays the v0.2.5 row. An opposing queued action is v0.2.5. A required item is v0.2.6. `give`, `take`, and an item changing hands stay v0.3.

The player line stays the verb plus the row's token. A missing token or an extra token is `SCHEMA`. With a GM seated, the line returns `queued` and the roll waits for `commit_turn`. With no GM, it commits one live tick.

---

## v0.2.5 — the opposing check

Proved. The behavior below is law. Proof `v02_p12` is `tests/conformance/v02/gates.rs`, compiled through `tests/sitting.rs`. It does not rewrite `v02_p01`, `v02_p02`, `v02_p03`, `v02_p04`, `v02_p05`, `v02_p06`, `v02_p07`, `v02_p08`, `v02_p09`, `v02_p10`, or `v02_p11`. `v02_p11` still proves the two lists at target 0 with no opposing action. The fixture pack is one the proof builds. `packs/oneshot-docks` is unchanged. No new effect class. No new moves key.

`defend` is the named actor's queued row. It is a pack row in the shape v0.2.4 already accepts. On the tick that applies a row that names a check, an action the named actor queued before `commit_turn` supplies the opposing check when that action names a check. That check is resolved for the named actor with target 0. Its total is the target of the row being applied. The same resolution is the one that action uses when that action itself has no opposing action. A row with no such queued action is still resolved with target 0. The acting player sees `{"kind":"check","success":true}` or `{"kind":"check","success":false}`. The success list and the failure list stay as v0.2.4 wrote them. A row that does not name a check does not roll. A refusal of the opposing check is that tick's existing refusal, and the lists do not run.

The player line stays the verb plus the row's token. A missing token or an extra token is `SCHEMA`. With a GM seated, the line returns `queued` and the roll waits for `commit_turn`. With no GM, it commits one live tick. A required item refuses before queue, as v0.2.6. `give`, `take`, and an item changing hands stay v0.3.

---

## v0.2.6 — the item

Proved. The behavior below is law. Proof `v02_p13` is `tests/conformance/v02/gates.rs`, compiled through `tests/sitting.rs`. It does not rewrite `v02_p01`, `v02_p02`, `v02_p03`, `v02_p04`, `v02_p05`, `v02_p06`, `v02_p07`, `v02_p08`, `v02_p09`, `v02_p10`, `v02_p11`, or `v02_p12`. `v02_p12` still proves the opposing check with no required item. The fixture pack is one the proof builds. `packs/oneshot-docks` is unchanged. No new effect class.

A row may name `item`. The value is an item id. `schema/moves.schema.json` accepts that key and still rejects an unknown key. An empty id is `SCHEMA` at pack load. A row that does not name `item` is unchanged.

The acting sheet is the player who types the line. That sheet's inventory lists the id, or the line is `MISSING_ITEM`. The player socket sends `{"kind":"refused","code":"MISSING_ITEM"}` before queue, with a GM seated and with no GM. Nothing is queued. `world_time` does not advance. No tick is written. The item stays on the sheet. Hello does not send this code. The tick log does not record it.

A sheet that lists the id still queues, and the row still runs as v0.2.5 wrote it: the check, the two lists, and an opposing action queued before `commit_turn`. `give`, `take`, and an item changing hands stay v0.3.

The player line stays the verb plus the row's token. A missing token or an extra token is `SCHEMA`. With a GM seated, a held item returns `queued` and the roll waits for `commit_turn`. With no GM, it commits one live tick.

---

## Still unbuilt for the minor

These stay out for every patch of v0.2:

- No new effect class. `give`, `take`, and an item changing hands are v0.3.
- Friends and foes are not a field. The row does not list who may be named.
- No ack cursor. No outbound queue.
- No model inside `npc_act`. No cargo feature named `llm`.
- No change whose purpose is to make `spook/event` start a Grok turn.
- The typed `spook gm` client stays the v0.1 client. It accepts `close`.

`docs/architecture_v0.1.md` §7 still holds where this file does not name a change.

---

## Host

A Grok skill may tell an already-running turn to call `say` and `session_status`. A host monitor on the tape may print new `player:` lines to wake that turn. Neither is shard behavior. None of these patches writes the skill.
