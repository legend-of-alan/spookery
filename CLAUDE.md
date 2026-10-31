# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Spookery is a Rust workspace that hosts a "pack" (a folder of YAML rules and Markdown pages) as a living world for up to three players and one GM seat. The GM may be a human or a model, but only the engine changes the world: dice are seeded, every commit is journaled, and `spook verify` replays the journal to prove the save. Read `docs/README.md` for the glossary (pack, seat, canon, tick, round, intent, effect) before naming things.

## Commands

The Rust version is pinned in `rust-toolchain.toml`. CI runs exactly these, and all must be clean:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build --workspace --locked
cargo test --workspace --locked
```

Narrower test runs:

```sh
cargo test -p spookery-engine --lib                                    # engine unit + property tests (round/prop)
cargo test -p spook --test integration feature_census                  # plain integration tests, filtered by name
cargo test --workspace --test features_host --test features_world      # all BDD scenarios
cargo test -p spook --test features_world -- --name "Locked exit"      # BDD scenarios whose `Scenario:` line matches a regex
```

`--name` matches scenario titles, not feature file names. Each program runs every folder in its list, so the output shows a `0 scenarios` summary for each folder with no match.

The live model round (`crates/spook/tests/integration/live_round.rs`) is `#[ignore]`d; run it with `-- --ignored` and `SPOOK_GM_KEY`, `SPOOK_GM_URL`, `SPOOK_GM_MODEL` set (Ollama works: key `ollama`, URL `http://127.0.0.1:11434/v1`).

Pack tooling (after `cargo build`): `spook check --json <pack>` (stdout is `[]` when clean), `spook status`, `spook round`, `spook migrate`, `spook verify`. `spook run <pack>` writes `state/` into the pack it is given, so always run it on a copy, never on `packs/*` in the repo.

## Crate layering (enforced by CI)

The `dependency-and-feature-checks` job in `.github/workflows/ci.yml` fails the build if these edges appear in the normal dependency tree:

- `protocol` (`spookery-protocol`): player and GM wire types. MIT licensed, so every dependency must be permissively licensed.
- `engine` (`spookery-engine`): world model, rules, rounds, save and journal. Synchronous; depends only on `protocol`.
- `server` (`spookery-server`): the table, seats over loopback TCP, idle timer, the MCP "agent door". Must not depend on `cli`, `agent` or `reqwest`.
- `agent` (`spookery-agent`): the model-GM turn state machine and prompt. Pure logic: no `engine`, `server`, `cli`, `tokio`, `reqwest` or `socket2`.
- `cli` (`spookery-cli`): `spook play` / `spook gm` terminals, the MCP GM client, and the `ai-gm` HTTP loop around `agent`. Must not depend on `engine` or `server`; it only talks to the host over the wire.
- `spook`: the clap binary (`src/main.rs`) and library `spookery`. Feature `server` pulls in engine+server, `client` pulls in cli; both builds must work alone (`--no-default-features --features server|client`).

Crates other than `spook` keep sources at the crate root (`lib.rs`, not `src/lib.rs`). Engine modules are `pub(crate)`; the public surface is the re-export list in `crates/engine/lib.rs`, and test-only hooks live behind its `testing` feature.

## Engine architecture

- **One writer.** All state changes happen inside a round. `round/play.rs::play_round` runs the stages in `round/stage/` in a fixed order: advance → pick → resolve → apply → finish → triggers → deliver → record. Stages write into a `Draft` (the staged save) and nothing is published until `record` commits.
- **Live and replay share the path.** `play_round` runs on the World that holds the pack lock and commits. `replay_round` runs on a lockless World and keeps the round in memory. Both run the same stages through `run_stages`. Replay (`round/replay.rs`) feeds recorded queues and presence back through those stages, and `round/verify.rs` checks the result byte for byte. Keep any new mechanic inside this path so replay stays exact. Dice come from `round/dice.rs`, keyed per round, never from ambient randomness or the model.
- **Store** (`store/`): the journal is an append-only, hash-chained set of gzip bundles under `state/journal`; `save.yaml` is written at checkpoints; speech goes on the session tape (`state/tape.jsonl`) and never touches the journal or clock. `commit.rs` publishes atomically, `recovery.rs` finishes or rolls back a commit a crash left half done, `lock.rs` holds the pack lock.
- **Versions.** `store::RULES_REV` is stamped on every save and round; a mismatch is refused, never migrated, so bump it only when the round format really changes. `pack_rev` hashes `world/`, `cast/` and `rules/` (not `pack.yaml`); after canon edits `spook check` reports `PACK_REV_MISMATCH` until `spook migrate`.
- **Visibility.** `rules/perceive.rs` builds each viewer's `View`; `visibility.rs` and `stage/deliver.rs` decide what each seat hears. A player sees a hidden fact only when that character knows it, and the model GM's prompt gets trimmed views (fact bodies are always dropped). Treat any change here as a privacy change.
- `canon/` loads and schema-checks a pack (unknown keys are rejected); `rules/` holds conditions, costs, gates, rolls, effects and the expression language described in `docs/spec.md`.

## Host and seats

`spook run` binds only loopback: the game port (default `127.0.0.1:9737`) and the MCP agent door on the next port (`/mcp`, streamable HTTP via `rmcp`). The door checks the `Host` header, so clients must use `127.0.0.1`, not `localhost`. GM tools are `table_status`, `perceive`, `queue_intent`, `say`, `commit_round`, `close`. With the GM seat empty, each player line commits its own round. `spook gm` and `spook ai-gm` are both clients of the agent door. The wire format is specified in `docs/seat-protocol.md`.

## Tests and docs that are wired together

- BDD scenarios live in `crates/engine/tests/features/<folder>` and `crates/spook/tests/features/<folder>`, run by cucumber in two programs per crate (`features_host`, `features_world`). A new feature folder must be added to exactly one list: `crates/spook/tests/feature_dirs.rs` or `testing::HOST`/`WORLD` in `crates/engine/lib.rs`. New step modules are registered with `#[path]` in `features_host.rs` / `features_world.rs`. The `feature_census` test fails on unlisted, doubly listed, or loose `.feature` files.
- `docs/engine-guarantees.md` states what the engine promises; every `##` section must cite feature files that exist (`engine_guarantees` test). The feature files win if the two disagree.
- Every YAML fence in `docs/pack-reference.md` must form a pack that `spook check` accepts (`pack_reference` test), so pack schema changes need doc updates in the same PR.
- `packs/sample-d20` is played to its ending by tests; `packs/oneshot-docks` is the multi-PC fixture the engine and seat tests rely on, so leave it unchanged. `spook new` writes the template in `crates/spook/templates/starter/`.
- Tests copy packs into temp dirs (`testing::copy_pack`, `testing::fresh_dir`); never let a test write into `packs/`.

## Conventions

- Follow the design principles and the issue and PR title format in `CONTRIBUTING.md`.
- In PR bodies, use `Closes #N` only for the closing keyword; elsewhere write "issue #N" / "PR #N", never a bare `#N`.
- Never merge a PR. Never force-push or rebase a pushed branch; merge `main` into it instead.
- PR CI runs Linux only; `main` also runs macOS and Windows, and Windows runs clippy for the `cfg(windows)` code in `engine/store/durability.rs`.
- Do not restore `docs/legacy_architecture/` or `docs/adr/`.
- Pack authoring workflow (edit a copy, loop on `spook check --json`, `spook migrate` after canon edits) is in `.grok/skills/author-pack/SKILL.md`.
