# ADR 0010: v0.2 action row

- Status: Accepted
- Date: 2026-10-02
- Source: `docs/architecture_v0.2.md` Demo and Patches (lines 12–36) and Still unbuilt for the minor (lines 174–185)

## Context

Demo states "When v0.2.6 is green, a pack can define a player action as one row" and "No new effect class appears in this version." Patches states "v0.2.0, v0.2.1, v0.2.2, v0.2.3, v0.2.4, v0.2.5, and v0.2.6 are proved. The later rows are names." It states "Two patches proceed together only when they do not both edit the player read loop, the shard thread, or `tools/call`."

`docs/architecture_v0.1.md` states "A typed line is mapped to `tools/call` as follows." Still unbuilt for the minor states "The typed `spook gm` client stays the v0.1 client. It accepts `close`."

## Decision

"When v0.2.6 is green, a pack can define a player action as one row. The row names a verb. It says the token after the verb is absent, a place id, or an actor id, and which field of an existing effect that token fills. The effect lands on the speaker or on the named actor. The row may name a check, with one effect list for success and one for failure, using classes that already apply. Another actor's action, queued before `commit_turn`, may oppose that check. The row may name an item id the sheet must already hold, and a missing item refuses the action before any effect. No new effect class appears in this version."

"The pack does not choose the place or the actor. The player types that token. Friends and foes are not a field in this version."

"Two patches proceed together only when they do not both edit the player read loop, the shard thread, or `tools/call`."

"`attack` is a pack row once v0.2.4 exists: an actor token, a check, and a `resource` delta on a key the sheet already lists. `defend` is that actor's queued row at v0.2.5. This minor does not require `oneshot-docks` to contain either row. `give`, `take`, and an item changing hands are v0.3. Idle moves such as `work` keep the match arms they have."

"These stay out for every patch of v0.2:"

- "No new effect class. `give`, `take`, and an item changing hands are v0.3."
- "Friends and foes are not a field. The row does not list who may be named."
- "No ack cursor. No outbound queue."
- "No model inside `npc_act`. No cargo feature named `llm`."
- "No change whose purpose is to make `spook/event` start a Grok turn."
- "The typed `spook gm` client stays the v0.1 client. It accepts `close`."

"`docs/architecture_v0.1.md` A7 still holds where this file does not name a change."

## Consequences

The F-v02-* cards cover the behavioral law for v0.2.0 through v0.2.6.

What stays out of this minor:

- "No new effect class."
- "`give`, `take`, and an item changing hands are v0.3."
- "Friends and foes are not a field. The row does not list who may be named."
- "No ack cursor. No outbound queue."
- "No model inside `npc_act`. No cargo feature named `llm`."
- "No change whose purpose is to make `spook/event` start a Grok turn."
- "This minor does not require `oneshot-docks` to contain either row."

"Two patches proceed together only when they do not both edit the player read loop, the shard thread, or `tools/call`."

The typed `spook gm` client accepts `close`.

## No existing proof

"Showing that scene does not cut the bare tag."

"This minor does not require `oneshot-docks` to contain either row. `give`, `take`, and an item changing hands are v0.3. Idle moves such as `work` keep the match arms they have."

"No ack cursor. No outbound queue."

"No model inside `npc_act`. No cargo feature named `llm`."

"No change whose purpose is to make `spook/event` start a Grok turn."

"Friends and foes are not a field. The row does not list who may be named."

"The typed `spook gm` client stays the v0.1 client. It accepts `close`."

"`docs/architecture_v0.1.md` A7 still holds where this file does not name a change."

## Superseded

The v0.1 typed-line map below is text the typed client does not implement. `docs/architecture_v0.2.md` Still unbuilt for the minor replaces it: "The typed `spook gm` client stays the v0.1 client. It accepts `close`."

`docs/architecture_v0.1.md` lines 168–184 state:

"`spook gm` is not a second server. It sends `gm`, then speaks the rmcp 0.8 codec. A typed line is mapped to `tools/call` as follows. The remainder rules are pin P33, plus the `attempt` target token from §5."

| Typed line | Call |
|---|---|
| `say <text...>` | `say` `{ address: table, text }` |
| `say place <place> <text...>` | `say` `{ address: place, place, text }` |
| `say actor <id> <text...>` | `say` `{ address: actor, actor, text }` |
| `perceive <as>` | `perceive` `{ as }` |
| `recall <as>` | `recall` `{ as }` |
| `recall <as> <query...>` | `recall` `{ as, query }` |
| `intent <play-line>` | `intent`. The remainder is one v0 play line. `source` is `player`, as v0 §8 already requires. A leading `npc:<id>` or `player:<id>` selects the actor |
| `commit` | `commit_turn` `{ include_tick: true }`. `session_id` is omitted. The handler accepts that omission. It does not invent a session id |
| `npc_act` | `npc_act`. Result stays `LLM_DISABLED` |
| `save <name>` | `session_save` `{ name }` |
| `apply <json>` | `apply_effect`. The remainder is the v0 effect envelope as one JSON object |
| `check <as> <intent_id> <check_id> <target>` | `resolve_check` |
| `close` | Drop the connection and free the GM seat. No presence write |

`docs/architecture_v0.1.md` line 261 states "The GM server has no say-line grammar. `say` is the tool in §3. `spook gm` maps typed lines to that tool. The handler is the same one a model's `tools/call` reaches."

These sentences in `docs/architecture_v0.2.md` say "until v0.2.2". The proved v0.2.2 row in Patches, and the section "v0.2.2 — the street", replace them. That row states "Proved, below. The docks `travel` row uses a place token and one `location` effect."

"`token: place` is `SCHEMA` at pack load until v0.2.2. A template `class` other than `flag` is `SCHEMA` at pack load until v0.2.2."

"`travel` rejects an `effects` list at pack load until v0.2.2."
