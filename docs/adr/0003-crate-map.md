# ADR 0003: Crate and module map

- Status: Accepted
- Date: 2026-10-02
- Source: docs/architecture_v0.md §3, lines 119–201

## Context

`docs/architecture_v0.md` §3 states "Cargo workspace, package name `spookery`, `publish = false` on every member. Binary names `spook` and `spookery` share `src/cli/main.rs`. Do not publish a crate named `spook`." It states "Dependency arrows, and the only legal ones" and gives the crate table. It states "Tokio exists only inside `spookery-mcp` (stdio runtime) and, from gate C, inside `src/cli` for the scheduler sleep. The tick function and the kernel stay on the calling thread. No `tokio::spawn` of a tick. No cargo feature named `llm`." It states "`store` is the only module that calls `File::create` under `state/` or `saves/`. Other modules take `&mut Txn`." It states "`rulesets/` is the authoring template copied into a pack. The loader reads `pack/rules/` only. v0 loads one ruleset, the one in the pack directory."

## Decision

`edition = "2021"`. `rust-version = "1.89"` because the pinned YAML crate requires it (§16).

The workspace paths are:

```text
Cargo.toml                 # workspace; bin package spookery at the repo root
src/cli/main.rs            # thin binary
src/kernel/Cargo.toml      # package spookery-kernel
src/kernel/lib.rs
src/tick/Cargo.toml        # package spookery-tick
src/tick/lib.rs
src/mcp/Cargo.toml         # package spookery-mcp
src/mcp/lib.rs
schema/                    # JSON Schema, CC BY 4.0
rulesets/                  # template of the tiny original rules; not loaded at runtime
packs/oneshot-docks/       # official fixture, CC BY 4.0
tests/conformance/         # integration tests of the root bin package
```

Dependency arrows, and the only legal ones:

| Crate | May depend on | Must not depend on |
|---|---|---|
| `spookery-kernel` | `serde`, `serde-saphyr` (the one YAML crate), `blake2`, `jsonschema`, Rust std | `tokio`, `rmcp`, `spookery-tick`, `spookery-mcp`, any model SDK, any DB crate |
| `spookery-tick` | `spookery-kernel` | `tokio`, `rmcp`, `spookery-mcp` |
| `spookery-mcp` | `spookery-kernel`, `spookery-tick`, `rmcp`, `rmcp-macros`, `tokio` | A second kernel, a model SDK |
| root bin `spookery` | `spookery-kernel`, `spookery-tick`, `spookery-mcp` | Business logic. Parsing argv and printing are the whole crate |

Tokio exists only inside `spookery-mcp` (stdio runtime) and, from gate C, inside `src/cli` for the scheduler sleep. The tick function and the kernel stay on the calling thread. No `tokio::spawn` of a tick. No cargo feature named `llm`.

`rulesets/` is the authoring template copied into a pack. The loader reads `pack/rules/` only. v0 loads one ruleset, the one in the pack directory.

All paths under `src/kernel/`.

| Module | Owns | Public surface used by tick and MCP |
|---|---|---|
| `model` | `Now`, `Intent`, `Effect`, `PcSheet`, ids, closed enums | Types |
| `error` | `Code` | `Result` |
| `codec` | The only `serde-saphyr` calls for `Now`, sheets, proposals, rules YAML | `emit_now`, `load_now` |
| `canon` | Frontmatter split, `pack_rev` | `load_canon`, `pack_rev` |
| `store` | Shard lock, `Txn`, commit, crash recovery | `tick_commit`, `commit_turn`, `open_shard` |
| `moves` | `rules/moves.yaml` allowlist | `load_moves` |
| `check` | `resolve_check` | `resolve_check` |
| `inventory` | Needs, resources, stock, PC inventory list | Used by `apply` |
| `firewall` | Fact visibility | `allows(viewer, fact_id)` |
| `lint` | Schema, frontmatter, hidden-string, strip-bodies helper | `lint_pack` |
| `perceive` | Scoped view | `perceive` |
| `recall` | Keyword and fact-id index over a view | `recall` |
| `eligibility` | Eligibility predicate | `is_eligible` |
| `derive` | `kernel_intents_for_eligible_npcs` | That function |
| `expand` | Move → effect table | `expand` |
| `apply` | Effect validators, staging into `Txn` | `apply_effect` |
| `pause` | Pause policy applied to staging clocks | `apply_pause_policy` |
| `digest` | Host vs player filter over JSONL | `idle_digest` |
| `limits` | Default budget constants | `Budget` |

`store` is the only module that calls `File::create` under `state/` or `saves/`. Other modules take `&mut Txn`.

Tick modules:

| Module | Function |
|---|---|
| `loop_` | `run_tick` — §7 |
| `advance` | `pick_advance` |
| `budget` | `prioritize`, caps, catch-up count |
| `seed` | `seed_preimage`, `shuffle_key`, `die_face` |
| `replay` | `replay_recorded` |
| `log` | One JSON object, the §12 shape |

MCP modules:

| Module | Function |
|---|---|
| `profile` | Bind `pack-table`, `pack-tick`, or `pack-studio` for the process lifetime. Stable `tools/list` order |
| `table` | Table handlers |
| `tick_tools` | Tick handlers. Same kernel functions the tick loop calls |
| `studio` | Studio handlers |

## Consequences

- The string scan is F-v0-bounds.
- Every member has `publish = false`. Binary names `spook` and `spookery` share `src/cli/main.rs`. The spec states "Do not publish a crate named `spook`."
- Tokio exists only inside `spookery-mcp` (stdio runtime) and, from gate C, inside `src/cli` for the scheduler sleep. The tick function and the kernel stay on the calling thread.
- The spec states "No `tokio::spawn` of a tick. No cargo feature named `llm`."
- The root bin's must-not cell states "Business logic. Parsing argv and printing are the whole crate."
- `store` is the only module that calls `File::create` under `state/` or `saves/`. Other modules take `&mut Txn`.
- The loader reads `pack/rules/` only. v0 loads one ruleset, the one in the pack directory.

## No existing proof

"`publish = false` on every member."

"Binary names `spook` and `spookery` share `src/cli/main.rs`. Do not publish a crate named `spook`."

"`edition = "2021"`. `rust-version = "1.89"` because the pinned YAML crate requires it (§16)."

"Tokio exists only inside `spookery-mcp` (stdio runtime) and, from gate C, inside `src/cli` for the scheduler sleep. The tick function and the kernel stay on the calling thread. No `tokio::spawn` of a tick. No cargo feature named `llm`."

The may-depend column states that `spookery-kernel` may depend on `serde`, `serde-saphyr` (the one YAML crate), `blake2`, `jsonschema`, and Rust std; that `spookery-tick` may depend on `spookery-kernel`; that `spookery-mcp` may depend on `spookery-kernel`, `spookery-tick`, `rmcp`, `rmcp-macros`, and `tokio`; and that the root bin `spookery` may depend on `spookery-kernel`, `spookery-tick`, and `spookery-mcp`.

"`rulesets/` is the authoring template copied into a pack. The loader reads `pack/rules/` only. v0 loads one ruleset, the one in the pack directory."

"`store` is the only module that calls `File::create` under `state/` or `saves/`. Other modules take `&mut Txn`."

"Bind `pack-table`, `pack-tick`, or `pack-studio` for the process lifetime. Stable `tools/list` order."

The workspace path list and the kernel, tick, and MCP module tables name files, owners, and public functions. Those rows stay in this record.

## Superseded

None inside §3.
