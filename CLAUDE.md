# CLAUDE.md

Guidance for coding agents. [CONTRIBUTING.md](CONTRIBUTING.md) is the authority for names, tests, logs, titles and design principles, so this file points there and does not repeat it.

Spookery is a Rust workspace that hosts a "pack" (a folder of YAML rules and Markdown pages) as a living world for up to three players and one GM. The GM may be a human or a model, but only the engine changes the world: dice are seeded, every commit is journaled, and `spook verify` replays the journal to prove the save.

## Before you write

- Delete, reuse or derive before you add. Do not add a type, helper or flag that one caller needs.
- Name things by the Naming rules in `CONTRIBUTING.md`. Reuse a word the code already has, and take the canonical word from `docs/concept-map.md`. Do not coin a word, and do not use a metaphor.
- Say it once. When docs and code disagree, fix the one that is wrong in the same PR.

## Commands

The Rust version is pinned in `rust-toolchain.toml`. CI runs exactly these, and all must be clean:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build --workspace --locked
cargo test --workspace --locked
```

Narrower runs:

```sh
cargo test -p spookery-engine --lib
cargo test -p spook --test integration engine_guarantees
cargo test --workspace --test features_host --test features_world
cargo test -p spook --test features_world -- --name "Locked exit"
```

`--name` matches scenario titles, not file names, and each program prints a `0 scenarios` summary for every folder with no match.

`spook run <pack>` writes `state/` into the pack it is given, so run it on a copy and never on `packs/*` or `tests/packs/*`. Pack authoring is in `.grok/skills/author-pack/SKILL.md`.

## Crates

`.github/workflows/ci.yml` lists every allowed crate edge and feature, and fails any other. Beyond that list:

- `protocol`, `agent` and `client` are MIT licensed, so their dependencies must be permissive.
- `engine` is synchronous and depends only on `protocol`. It logs through `tracing`.
- `agent` is async but runtime-free: it never builds, enters or blocks on a runtime.
- `client` talks to the host only over the wire.
- `spook` is the clap binary, and its `host` and `client` features each build alone.
- Crates other than `spook` keep sources at the crate root (`lib.rs`, not `src/lib.rs`).
- The host's ports, wire format and MCP setup are in `docs/seat-protocol.md`.
- Engine modules are `pub(crate)` except `entity`, `ecs` and `round`. The rest of the public surface is the re-export list in `crates/engine/lib.rs`, and the `unreachable_pub` lint enforces it.

## Engine

- **One writer.** State changes only inside a round, which `round/play.rs` runs as the stages in `round/stage/`: advance, pick, apply, finish, triggers, deliver, record. Stages write into the round's working copy of the save, and nothing is published until `record` commits. The one exception is `spook migrate`.
- **Live and replay share the path.** `round/replay.rs` runs the same stages on a lockless session, and `round/verify.rs` compares the result byte for byte. Keep a new mechanic inside this path. Dice come from `round/dice.rs`, never from ambient randomness or the model.
- **Versions.** `store::RULES_REV` is stamped on every save and round, and a mismatch is refused, never migrated. Bump it only when the save or round format changes. After canon edits, `spook check` reports `PACK_REV_MISMATCH` until `spook migrate`.
- **Visibility.** `rules/perceive.rs` and `rules/visibility.rs` decide what each viewer sees. Treat any change there as a privacy change.

## Tests and docs wired together

- Scenarios live in `crates/engine/tests/features/<folder>` and `crates/spook/tests/features/<folder>`. A new spook folder runs without being listed, and a new step module is registered with `#[path]` in `features_host.rs` or `features_world.rs`.
- Every `##` section of `docs/engine-guarantees.md` cites feature files that exist (`engine_guarantees` test).
- Every YAML fence in `docs/pack-reference.md` forms a pack that `spook check` accepts (`pack_reference` test).
- Every backticked file path, Rust path and CamelCase name in this file and `docs/*.md` exists under `crates/` (`doc_code_names` test), so a rename updates the docs in the same PR.
- `tests/packs/docks` is the one test pack. Tests copy it with `testing::copy_pack` and never write into `packs/` or `tests/packs/`. A canon edit there moves its `pack_rev`, so `crates/protocol/sessions/v8.jsonl` and `docs/seat-protocol.md` move with it.

## PRs

- Use `Closes #N` only as the closing keyword. Elsewhere write "issue #N" or "PR #N", never a bare `#N`.
- Never merge a PR. Never force-push or rebase a pushed branch. Merge `main` into it instead.
