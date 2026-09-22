# Spookery stack — v0

**Status:** Frozen for v0 implementation. Idle UX locked: host digest ≠ player resume.  
**Companions:** `BRANDING.md`, `PRODUCT_SPEC_v0.5.md`, `PACK_ENCODING_v0.3.md`, `SHARD_TICK_SPEC_v0.3.md`, `NOW_AND_EFFECTS.md`, `BUSINESS_CASE.md`.

If a crate, service, or format is not in this file, it is not in v0.

## Decision record

| Topic | Decision |
|---|---|
| Who builds v0 | Coding agents, reviewed by a professional programmer |
| Runtime language | Rust (`spook` single binary; `spookery` is the same file) |
| Crate / package name | `spookery` (do not publish a crate named `spook`; that name is taken) |
| Store | The pack folder. No database |
| `now.yaml` mid-shard | Runtime-only. Hand edits are a migration, not a live patch |
| Player characters | Starting template in `cast/`. Mutable sheet in `state/` |
| Hidden-string lint | Fail if any Markdown body contains another entity’s `hidden` string |
| Official fixture rules | Tiny original moves list. Not an SRD/ORC subset |
| Idle UX | Host `status` is a full idle digest. Player resume is viewer-scoped. Off-screen stock is not a login banner. See product spec §11 |

Go is the pace fallback only if week 3 is still fighting the type system instead of conformance test 1.

## Stack

| Layer | Choice | Notes |
|---|---|---|
| Language | Rust | Kernel invariants. Illegal writes should be hard to express. |
| Async | Tokio at the edges only | Scheduler and later `--serve`. Tick/kernel are synchronous. No concurrent ticks. |
| MCP | `rmcp` + `rmcp-macros`, stdio | One process, three profiles: `pack-table`, `pack-tick`, `pack-studio`. |
| Pack prose | Markdown bodies on `.md` entities | Flavor only. Never kernel-true. |
| Pack data / rules / `now` | YAML | One pinned YAML crate, chosen week 1, golden-file locked. |
| Logs | JSONL | `state/log/tick-NNNN.jsonl`, `state/memories/<id>.jsonl`. Evidence, not a second `Now`. |
| Contract | JSON Schema in `/schema` | Validates the data model. File extension is an encoding. |
| Canonical `Now` | Typed Rust struct → pinned YAML emitter | Typed equality for debug. Byte-equal `now.yaml` is the ship gate. |
| Commit | Temp file → `fsync` → atomic rename + shard dir lock | Crash before `tick_commit` keeps previous `state_rev`. |
| Hash / RNG | BLAKE2s | `pack_rev` over sorted canon bytes. Tick seed `shard_id\|tick_index`. |
| `recall` (v0) | Keyword + fact-id index | No vector DB. |
| LLM | None in this repo | BYOK live GM speaks MCP. Idle default `llm_npc_intents_per_tick: 0`. |
| CLI | `spook run \| lint \| tick \| replay \| conformance \| status` | Alias `spookery` → same binary. First 45 days: CLI + stdio only. |
| Tests | Rust tests + goldens under `/tests/conformance` | Same change must add or update a test. |
| Later play door | HTTP + WebSocket on the same binary | `--serve` only after the week-idle fixture survives. |
| Later shells | Separate clients | Skins. They submit intents. They do not resolve hits. |
| Hosting | One process per shard, folder on a volume | Measure idle shard-day cost before treating $9 as real. See `BUSINESS_CASE.md`. |
| Now / effects | `NOW_AND_EFFECTS.md` | Typed `Now` + closed effect payloads. `/schema/*.json` lands in commit 1. |
| License | AGPL-3.0-or-later on `/src`; CC BY 4.0 on spec + schema + official fixture | Already decided. |

## Explicitly out of v0

Web framework, ORM, embedder, game engine, official model SDK, marketplace, Discord bot, Postgres, SQLite.

## Crate pins (intent)

Pick exact versions in the first implementation commit. Do not tour crates.

- `rmcp`, `rmcp-macros`
- `serde` + **one** YAML crate (freeze week 1; current candidates: `noyalib` or `serde-saphyr`)
- `blake2` / equivalent BLAKE2s
- `jsonschema`
- `tokio` (scheduler / later serve only)
- no LLM client crate

## First implementation task

Conformance test 1 (hidden fact never appears in another actor’s `perceive`, memories, or rumors), then the `perceive` tool that makes it pass.
