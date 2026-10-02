# ADR 0007: Threats and non-goals

- Status: Accepted
- Date: 2026-10-02
- Source: docs/architecture_v0.md §15, lines 1874–1897, and §17, lines 1965–1983

## Context

§15 is titled "Threats to the refusal." Each row names how a second writer appears, and the countermeasure column is the rule that blocks it. §17 states what "Will not appear in this architecture, in the crate graph, or in gates A–C." The closing sentence is "Shells, when they exist later, submit intents. They do not resolve them. They are not designed here."

## Decision

| How a second writer appears | Countermeasure |
|---|---|
| Client resolves a hit, a stock change, or a secret locally and patches YAML | Play door submits `Intent` only. CLI has no path that opens `now.yaml` for write. Clients are not in this repo |
| GM agent calls `apply_effect` and expects durability | `apply_effect` stages a `Txn`. Only `commit_turn` / `tick_commit` rename files |
| `npc_act` or a memory summarizer appends `state/memories/*.jsonl` | No LLM crate. `npc_act` returns `LLM_DISABLED`. Memory lines are written inside the commit, from effect `memory`, after the firewall |
| `llm_say` or `public_say` copied into `Now`, a rumor, or resume | Inbound `llm_say` dropped. v0 does not call `npc_act`, so `public_say` is never stored. Digest prints effect rows only |
| `spook status` output reused as the player login banner | Separate functions. `v0_c09_idle_ux.rs` asserts stock deltas appear in host status and do not appear in player resume |
| Host edits `now.yaml` in an editor and runs `spook tick` | `HAND_EDIT` unless the command is `spook migrate`, which bumps `state_rev` and re-emits |
| Replay applies JSONL `effects` as the new state | `replay_recorded` re-executes moves through `expand` and `apply_effect` on a temp copy |
| Two ticks in flight (MCP request plus scheduler) | One `flock` around `run_tick` and `commit_turn`. Second caller gets `COMMIT_CONFLICT`. Tick code is synchronous |
| `patch_entity` also updates `now.yaml` | `patch_entity` writes canon paths only. `pack_rev` then mismatches and ticks refuse until `migrate` |
| Studio `run_conformance` ticks the live shard | Suite runs on a temp copy |
| A new effect class added in a handler (`xp`, `inventory`, `narration`) | Parse boundary returns `ILLEGAL_CLASS`. Schema `oneOf` forbids extras |
| `perceive()` with no `as`, or `as=gm`, inside the tick | `ViewerId` has no GM variant. Tick profile rejects anything but `npc:<id>`. Test 9 |
| `places.actors` edited independently of `actors.place` | Load fails `SCHEMA` on disagreement. Emit rebuilds the index from `actors.*.place` |
| `state/rumors.yaml` edited and treated as live | Ignored as a source. Next commit rewrites it from `now.rumors` |
| Feature flag `llm` defaulted on | No such feature. `maybe_one_llm_intent` returns `[]` |
| Tokio task per shard inside the kernel | Kernel and tick crates cannot name `tokio`. Scheduler lives in the binary edge |
| Hand-rolled dice or `rand::thread_rng` | `die_face` is BLAKE2s. Replay test 7 fails if a face drifts |
| Markdown body used as an exit, a stock count, or a hidden fact | Strip-bodies test. Lint fails when a body contains a hidden string |
| Second snapshot `now.canonical.json` | Forbidden until `emit_now` is shown unable to pin. One snapshot |
| Publishing crate `spook` | Package name `spookery`, `publish = false` |

Will not appear in this architecture, in the crate graph, or in gates A–C:

- Web framework, PWA, public website
- Discord bot
- `--serve` and any HTTP or WebSocket route (unscheduled, including gate C)
- Marketplace, accounts, billing UI
- Postgres, SQLite, Redis, an ORM
- Embedder, vector database, recall beyond keyword and fact-id
- Game engine, tile editor, visual VTT, battle maps, character controller, spatial netcode
- Official model SDK, LangChain, an LLM crate, idle `npc_act`, `llm_npc_intents_per_tick` above 0
- Multi-ruleset load, D&D branding, SRD/ORC as the official fixture, an official campaign
- Model-edited `cast/` or `world/` during play
- A second writer of `state/`
- Concurrent ticks
- Path B (Spookery as the default consumer AI RPG)

"Shells, when they exist later, submit intents. They do not resolve them. They are not designed here."

## Consequences

- The countermeasure that names `v0_c09_idle_ux.rs` is F-v0-idle.
- The countermeasure that names Replay test 7, and the countermeasure that `replay_recorded` re-executes moves through `expand` and `apply_effect` on a temp copy, are F-v0-replay.
- The countermeasure that `apply_effect` stages a `Txn` and only `commit_turn` / `tick_commit` rename files is F-v0-atomic.
- The countermeasure that kernel and tick crates cannot name `tokio`, the countermeasure that `pack_rev` mismatches and ticks refuse until `migrate`, and the countermeasure that schema `oneOf` forbids extras, are F-v0-bounds.
- The checklist excludes `--serve` and any HTTP or WebSocket route, a database (Postgres, SQLite, Redis, an ORM), an embedder, a second writer of `state/`, concurrent ticks, and an official model SDK.

## No existing proof

The other countermeasures stay in this record:

- "Play door submits `Intent` only. CLI has no path that opens `now.yaml` for write. Clients are not in this repo."
- "No LLM crate. `npc_act` returns `LLM_DISABLED`. Memory lines are written inside the commit, from effect `memory`, after the firewall."
- "Inbound `llm_say` dropped. v0 does not call `npc_act`, so `public_say` is never stored. Digest prints effect rows only."
- "`HAND_EDIT` unless the command is `spook migrate`, which bumps `state_rev` and re-emits."
- "One `flock` around `run_tick` and `commit_turn`. Second caller gets `COMMIT_CONFLICT`. Tick code is synchronous."
- "Suite runs on a temp copy."
- "Parse boundary returns `ILLEGAL_CLASS`."
- "`ViewerId` has no GM variant. Tick profile rejects anything but `npc:<id>`. Test 9."
- "Load fails `SCHEMA` on disagreement. Emit rebuilds the index from `actors.*.place`."
- "Ignored as a source. Next commit rewrites it from `now.rumors`."
- "No such feature. `maybe_one_llm_intent` returns `[]`."
- "Strip-bodies test. Lint fails when a body contains a hidden string."
- "Forbidden until `emit_now` is shown unable to pin. One snapshot."
- "Package name `spookery`, `publish = false`."

The §17 checklist stays in this record: web framework, PWA, public website, Discord bot, `--serve` and any HTTP or WebSocket route, marketplace, accounts, billing UI, Postgres, SQLite, Redis, an ORM, embedder, vector database, recall beyond keyword and fact-id, game engine, tile editor, visual VTT, battle maps, character controller, spatial netcode, official model SDK, LangChain, an LLM crate, idle `npc_act`, `llm_npc_intents_per_tick` above 0, multi-ruleset load, D&D branding, SRD/ORC as the official fixture, an official campaign, model-edited `cast/` or `world/` during play, a second writer of `state/`, concurrent ticks, and Path B. "Shells, when they exist later, submit intents. They do not resolve them. They are not designed here."

## Superseded

"Clients are not in this repo" is replaced by `docs/architecture_v0.1.md` §1 and §3. §1's laptop sitting includes a Player terminal whose command is `spook play packs/oneshot-docks`. §3 states that `spook play <pack>` "Connects to `127.0.0.1:9737` as the player seat" and writes "Nothing under the pack."

The `--serve`, database, model-SDK, and second-writer lines are restated. `docs/architecture_v0.1.md` §2 states "No `--serve`. No accounts. No database. No HTTP route." §3 states "There is no `--serve`." §7 states "No second writer. Clients do not write `state/` or the tape." §7 states "No model SDK in the kernel or the tick." §7 states "No `--serve` on a non-loopback address, no accounts, no connection secret, no website, no database."
