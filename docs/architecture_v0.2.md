# Spookery v0.2 architecture

Status: Draft. Not law. Three gates, none started. Tag `v0.1` is `b1d4fd1`. Where this file names a change, that change is the license for that gate. Until the gates are proved and this status says frozen, `docs/architecture_v0.1.md` holds, then `docs/architecture_v0.md`. Where this file does not name a change, v0.1 holds, then v0. A rule written in none of them is out of scope.
License: CC BY 4.0, same as the v0 spec. Runtime code under /src stays AGPL-3.0-or-later.

This file licenses three gates of one version. It names what becomes true, the proofs, and what stays unbuilt. It does not prescribe modules or a file-by-file plan. Do not edit `packs/oneshot-docks`, `docs/architecture_v0.md`, or the frozen body of `docs/architecture_v0.1.md`.

The v0.1 demo stays the demo. Its mechanical line is `wait`. Rowan stays in `docks_shop`. Gate B adds `travel <place>` because `expand` already turns a place target into a location effect.

---

## 1. What becomes true

When all three gates are green, the following hold. A gate implements only the rules §3 assigns it.

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

`attack`, `give`, and `take` stay `NOT_IN_SPEC` and queue nothing. The only new player verb is `travel`. Player lines are `say`, `talk`, `wait`, `attempt <check> <target>`, and `travel <place>`.

The same PC drops and connects again. That connect is a new `session_open` for the same actor, as v0.1. Scrollback returns. `humans` is 1. Intents already queued are still queued, once each. Reconnect does not apply them. No new session-id rule. No durable queue.

The GM socket does not receive tape bytes before MCP `initialize`. `initialize` does not append the tape. GM scrollback is `session_status` with `since`. Player scrollback on hello stays the v0.1 lines. A `say` tool result is still followed by its `spook/event`, not preceded by it. A player `say` still reaches the seated GM as a `spook/event`. `spook gm --stdio`, when nothing is listening, still exits 1. Stderr is the operating-system error. Stdout is empty. The pack is unchanged.

Three outcomes, one per gate. Not a file list.

- Gate A: a dispatch table so each GM tool name has a visible handler or a visible refusal. Handlers stay on the shard thread.
- Gate B: a player-line classifier, so an unknown verb is not discovered by catching `NOT_IN_SPEC`.
- Gate C: one wire codec for player lines, tape objects, `spook/event` frames, and tool results. Key order stays the order v0.1 pins. The codec stays outside the kernel. The kernel still does not link `tokio`, `rmcp`, or a model SDK.

---

## 2. Proofs

Each item is the assertion. The new proofs live in one new conformance file, created in Gate A and extended by Gates B and C. `tests/conformance/v01.rs` stays unrewritten. `tests/conformance/v01_p09.rs` and `tests/conformance/v01_p12_p21.rs` stay unrewritten. The v0 tests `docs/architecture_v0.1.md` §8 lists as unrewritten stay unrewritten. No shared test harness. No unit-test backfill. Do not split `store.rs`. These proofs do not edit `packs/oneshot-docks`.

1. GM `tools/list` gives distinct descriptions and names the keys §1 lists for `say`, `perceive`, `commit_turn`, `session_status`, and `intent`. A missing required key stays `SCHEMA`, or `UNSCOPED_PERCEIVE` when `perceive` has no `as`. `npc_act` stays `LLM_DISABLED`. `session_open`, `patch_entity`, `resolve_check`, `apply_effect`, `recall`, and `session_save` stay `NOT_IN_SPEC`.
2. `spook run --mcp pack-table` does not gain `say` or a `tools/call` body. `v0_c01_load_mcp` stays green.
3. With a GM seated, `travel <place>` returns `queued`, does not advance `world_time`, and does not move the actor until `commit_turn`. With no GM, it commits one live tick. A missing place is `SCHEMA` and queues nothing. `attack` stays `NOT_IN_SPEC` and queues nothing. No new effect class. NPC derivation is unchanged.
4. The same PC drops and connects again. Scrollback returns, `humans` is 1, and the in-memory queue is not flushed or applied twice. No new session-id rule. No durable queue.
5. The GM socket does not receive tape bytes before MCP `initialize`. Scrollback is `session_status` with `since`. A `say` tool result is still followed by its `spook/event`, not preceded by it. The stdio bridge still exits when nothing is listening.

---

## 3. Gates

Three gates, none started. The letters belong to this version. v0.1's Gates A–E stay closed. This file does not open a gate in `docs/architecture_v0.1.md`.

A gate does not prescribe modules or file order. It names what becomes true, and what stays unbuilt so the session does not invent the next gate. The proofs stay in §2. This section does not restate them.

The gates are linear. Gates A and B both leave the existing JSON builders in place. Gate C is the one that replaces them. Do not implement two gates in parallel. Do not start a gate whose predecessor is not already green. A session reads that gate, the proofs it names, and the §1 rules those proofs cite.

Every gate keeps the §2 file rules. The v0 tests, `v01.rs`, `v01_p09.rs`, and `v01_p12_p21.rs` stay unrewritten in all three.

### Gate A — schemas

Proofs 1 and 2.

GM `tools/list` gives each name its own description and names the §1 keys for the five live tools. A missing required key stays `SCHEMA` or `UNSCOPED_PERCEIVE`. The dispatch table gives each listed name a visible handler or a visible refusal. `spook run --mcp pack-table` does not gain `say` or a `tools/call` body.

Still absent. `travel` stays `NOT_IN_SPEC` and queues nothing. No classifier. No codec. Accepting `gm` still writes the stored tape before MCP `initialize`. Player lines stay `say`, `talk`, `wait`, and `attempt <check> <target>`.

### Gate B — travel

Proofs 3 and 4. Gate A is green.

`travel <place>` uses the same queue path as `wait`. The classifier refuses an unknown verb without catching `NOT_IN_SPEC` from speech. The same PC drops and connects again: scrollback returns, `humans` is 1, and the in-memory queue is not flushed or applied twice.

Still absent. No codec. Accepting `gm` still writes the stored tape before `initialize`. No new effect class. No durable queue. No new session-id rule. `attack`, `give`, and `take` stay `NOT_IN_SPEC`.

### Gate C — codec

Proof 5. Gate B is green.

One wire codec for player lines, tape objects, `spook/event` frames, and tool results. Key order stays the order v0.1 pins. The codec stays outside the kernel. The GM socket does not receive tape bytes before MCP `initialize`. GM scrollback is `session_status` with `since`. A `say` tool result is still followed by its `spook/event`. The stdio bridge still exits when nothing is listening.

Still absent. No new player verb. Queue lifetime stays the Gate B rule: in memory, not flushed on reconnect, not durable, not applied twice. No ack cursor. No outbound queue. Player scrollback on hello stays the v0.1 lines. `initialize` does not append the tape.

---

## 4. Still unbuilt

`docs/architecture_v0.1.md` §7 still holds. None of the three gates opens the list below.

Will not appear in this version:

- No new effect class. A roll still writes nothing to `Now`. A rule that reads a roll and then changes the world is a later version.
- No ack cursor. No outbound queue.
- No model inside `npc_act`. No cargo feature named `llm`.
- No change whose purpose is to make `spook/event` start a Grok turn.
- The only new player verb is `travel`.
- The typed `spook gm` client stays the v0.1 client. It accepts `close`.

---

## 5. Host

A Grok skill may tell an already-running turn to call `say` and `session_status`. A host monitor on the tape may print new `player:` lines to wake that turn. Neither is shard behavior. None of these gates writes the skill.
