# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Spookery is a Rust workspace that hosts a "pack" (a folder of YAML rules and Markdown pages) as a living world for up to three players and one GM seat. The GM may be a human or a model, but only the engine changes the world: dice are seeded, every commit is journaled, and `spook verify` replays the journal to prove the save. Read `docs/README.md` for the glossary (pack, seat, canon, tick, round, intent, effect) before naming things, and follow the Naming rules in `CONTRIBUTING.md`.

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
cargo test -p spook --test integration engine_guarantees               # plain integration tests, filtered by name
cargo test --workspace --test features_host --test features_world      # all BDD scenarios
cargo test -p spook --test features_world -- --name "Locked exit"      # BDD scenarios whose `Scenario:` line matches a regex
```

`--name` matches scenario titles, not feature file names. Each program runs every folder in its list, so the output shows a `0 scenarios` summary for each folder with no match.

The live model round (`crates/spook/tests/integration/live_round.rs`) is `#[ignore]`d; run it with `-- --ignored` and `SPOOK_GM_KEY`, `SPOOK_GM_URL`, `SPOOK_GM_MODEL` set (Ollama works: key `ollama`, URL `http://127.0.0.1:11434/v1`).

Pack tooling (after `cargo build`): `spook check --json <pack>` (stdout is `[]` when clean), `spook status`, `spook round`, `spook migrate`, `spook verify`. `spook run <pack>` writes `state/` into the pack it is given, so always run it on a copy, never on `packs/*` or `tests/packs/*` in the repo.

## Crate layering (enforced by CI)

The `dependency-and-feature-checks` job in `.github/workflows/ci.yml` lists every allowed workspace edge, dev edges included, and every feature that turns one on; any other edge or feature fails the build. `testing` rides only dev edges. It also bans the outside crates named below from each crate's normal tree:

- `protocol` (`spookery-protocol`): player and GM wire types. MIT licensed, like `agent` and `client`, so every dependency must be permissively licensed.
- `engine` (`spookery-engine`): world model, rules, rounds, save and journal. Synchronous; of the workspace crates it depends only on `protocol`; its tests add only itself with `testing`. It logs through the `tracing` facade.
- `server` (`spookery-server`): the table, the game port's WebSocket endpoint, idle timer, the MCP endpoint. Must not depend on `client`, `agent` or `reqwest`.
- `agent` (`spookery-agent`): the GM brain any client can reuse. One module per concern: `connection` and `model` are the `Connection`/`Model` adapters it awaits, `chat_completions` the model request and reply codec, `context` the client state and observation, `tool` the tool list, dispatch and results, `turn` the GM turn graph (graph-flow) and `task/` one file per graph task. MIT licensed. Async but runtime-free: tokio reaches it only through graph-flow (`rt`, `time`); it never builds, enters or blocks on a runtime and does no I/O itself. No `engine`, `server`, `client`, `reqwest`, `rmcp`, `socket2` or `mio`.
- `client` (`spookery-client`): MIT licensed. One module per job. `player` and `gm` run `spook play` / `spook gm`, `terminal` shows `spook play` as plain lines or in its game window (`terminal/window`), `text` is their readable text, `websocket` is the WebSocket seat connection, `ai_gm` runs `spook ai-gm`: it implements the agent's `Connection` over the WebSocket connection and runs `agent::run`, and `http_model` calls the model over HTTP. Must not depend on `engine`, `server` or `rmcp`; it only talks to the host over the wire.
- `spook`: the clap binary (`src/main.rs`) and library `spookery`. Feature `server` pulls in engine+server, `client` pulls in client; both builds must work alone (`--no-default-features --features server|client`).

Crates other than `spook` keep sources at the crate root (`lib.rs`, not `src/lib.rs`). Engine modules are `pub(crate)`, except `entity`, which other crates name as `spookery_engine::entity`, `ecs`, whose component store other crates name as `spookery_engine::ecs::World`, and `round`, whose round result other crates name as `spookery_engine::round::Output`; the rest of the public surface is the re-export list in `crates/engine/lib.rs`, and test-only hooks live behind its `testing` feature. `pub` means reachable from outside the crate; the workspace `unreachable_pub` lint enforces it.

## Engine architecture

- **One writer.** All state changes happen inside a round. The one exception is `spook migrate`: `store/save.rs::migrate` writes through the same `Writer`s and `commit::seal_migrate` seals its own journal record. Only `Session::replace_world` swaps a session's world. `round/play.rs` runs the stages in `round/stage/` in a fixed order: advance → pick → apply → finish → triggers → deliver → record. Each stage reads a rule against the canon, the save, who is seated and the intent, and `round/stage/apply.rs` writes a rule's effects for an instant move, a timed action's finish and a change rule alike. Stages write into the round's working copy of the save, an `ecs::World`, and nothing is published until `record` commits. `ecs/undo.rs` gives each component store its own undo, so a savepoint is a change number, and `ecs/transaction.rs` owns the round's budget, changes, memory lines and change number.
- **Live and replay share the path.** A live round runs on the session that holds the pack lock and commits; replay (`round/replay.rs`) runs the same stages on a lockless session, feeding recorded queues and presence back through them, and `round/verify.rs` checks the result byte for byte. Keep any new mechanic inside this path so replay stays exact. Dice come from `round/dice.rs`, keyed per round, never from ambient randomness or the model.
- **Store** (`store/`): the journal is an append-only, hash-chained set of gzip bundles under `state/journal`; `save.yaml` is read, written and migrated by `save.rs` and published at checkpoints; speech goes on the session tape (`state/tape.jsonl`) and never touches the journal or clock. `commit.rs` publishes atomically, `recovery.rs` cuts a torn append and finishes a checkpoint a crash left half done, `lock.rs` holds the pack lock.
- **Versions.** `store::RULES_REV` is stamped on every save and round; a mismatch is refused, never migrated, so bump it only when the save or round format really changes. `pack_rev` hashes `world/`, `cast/` and `rules/` (not `pack.yaml`); after canon edits `spook check` reports `PACK_REV_MISMATCH` until `spook migrate`.
- **Visibility.** `rules/perceive.rs` builds each viewer's `View`; `rules/visibility.rs` and `stage/deliver.rs` decide what each seat hears. A note reaches only its readers and whoever stands in its place, a flag never reaches a player, and the model GM's prompt gets trimmed views (a player's drops notes). Treat any change here as a privacy change.
- `canon/` loads and schema-checks a pack (unknown keys are rejected); `rules/` holds conditions, costs, gates, travel, rolls, effects, visibility, the player's text and the expression language described in `docs/spec.md`.

## Host and seats

`spook run` binds only loopback: the game port (default `127.0.0.1:9737`), a WebSocket endpoint with `/player` and `/gm`, and the MCP endpoint on the next port (`/mcp`, streamable HTTP via `rmcp`; `--mcp-addr` moves it). Both check the `Host` header, so clients must use `127.0.0.1`, not `localhost`. Every seat sends `ClientMessage` (`{id, body: {kind, args}}`) and receives `ServerMessage` (`{seq, reply_to, body: {kind, ...}}`); every server message but the hello takes `seq` from the table's one counter, and a reply follows the messages its command caused. GM commands are `status`, `perceive`, `queue`, `say`, `commit`, `release`. With the GM seat empty, each player move commits its own round. `spook play`, `spook gm` and `spook ai-gm` use `/player` and `/gm` on the game port. MCP clients such as coding agents use the MCP endpoint. The wire format is in `docs/seat-protocol.md`; client config is in the Coding agent clients section there.

## Tests and docs that are wired together

- BDD scenarios live in `crates/engine/tests/features/<folder>` and `crates/spook/tests/features/<folder>`, run by cucumber. The engine runs all of its folders in one program, `features_world`. Spook runs the folders named in `HOST` (`crates/spook/tests/feature_main.rs`) in `features_host` and everything else under `tests/features` in `features_world`, so a new folder runs without being listed. New step modules are registered with `#[path]` in `features_host.rs` / `features_world.rs`.
- `docs/engine-guarantees.md` states what the engine promises; every `##` section must cite feature files that exist (`engine_guarantees` test). The feature files win if the two disagree.
- Every YAML fence in `docs/pack-reference.md` must form a pack that `spook check` accepts (`pack_reference` test), so pack schema changes need doc updates in the same PR.
- Every backticked file path, Rust path and CamelCase name in `CLAUDE.md` and `docs/*.md` must exist under `crates/` (`doc_code_names` test), so a rename updates the docs in the same PR.
- `tests/packs/docks` is the one test pack, the fixture every engine and seat test copies with `testing::copy_pack("docks", label)`: three PCs at the docks, and a yard with a d20 strike beside the 2d6 `exert` check, a fight and a lane ending. A canon edit there moves its `pack_rev`, so `crates/protocol/sessions/v3.jsonl` and `docs/seat-protocol.md` move with it. `packs/demo-d20` is the shipped demo: no test plays it, and CI only runs `spook check` on it. `spook new` writes the template in `crates/spook/templates/starter/`.
- Tests copy packs into temp dirs (`testing::copy_pack`, `testing::fresh_dir`); never let a test write into `packs/` or `tests/packs/`.

## Conventions

- Follow the design principles and the issue and PR title format in `CONTRIBUTING.md`.
- Logs and output follow `CONTRIBUTING.md`'s Logs and output: `tracing` events only, never a print outside `main.rs` and the `spook play`/`spook gm` terminals. `spook run`, `spook ai-gm` and the `spook play` window write `logs/` in the directory they start in.
- In PR bodies, use `Closes #N` only for the closing keyword; elsewhere write "issue #N" / "PR #N", never a bare `#N`.
- Never merge a PR. Never force-push or rebase a pushed branch; merge `main` into it instead.
- PR CI runs Linux only; `main` also runs macOS and Windows, and Windows runs clippy for the `cfg(windows)` code in `engine/store/durability.rs`.
- Do not restore `docs/legacy_architecture/` or `docs/adr/`.
- Pack authoring workflow (edit a copy, loop on `spook check --json`, `spook migrate` after canon edits) is in `.grok/skills/author-pack/SKILL.md`.
