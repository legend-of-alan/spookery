# Contributing

Contributions are issues only for now. Open an issue, and pick the form that fits.

A pull request from outside this repository is closed with thanks.

Labels and PR titles are below, under Issues and PRs. Build and play steps are in [README.md](README.md).

A contributor license agreement (CLA) and a developer certificate of origin (DCO) are deferred.

Feature scenarios live under `crates/engine/tests/features` and `crates/spook/tests/features`. The engine runs them in one program and spook in two:

```sh
cargo test --workspace --test features_host --test features_world
```

`rust-toolchain.toml` pins the Rust version, and rustup installs it for you. Pull requests only run CI on Linux. Pushes to main also run it on macOS and Windows.

Dependencies use normal caret versions, like `"1.4"`. An exact `=` pin needs a written reason in a comment next to it.

## Issues and PRs

Issue titles are plain sentences. Every issue gets an area label and a size label.

A PR squashes into one commit titled after the PR, so the title starts with the first letter of its issue's area label and ends with its size label, like `N: Rename Hangup to Close [S]`. CI checks it.

Every issue gets a priority label from `P0` (most urgent) to `P3` (least urgent), except an `idea`, which carries none. It also gets a type label, which is `enhancement`, `bug` or `documentation`. Phases are milestones.

## Design principles

- **Keep it simple.** The best fix is often deleting code, not adding it. Before you write something new, check whether you can remove, reuse or derive what you need from what's already there. Build only as much as the job needs. CI shows a `Net: +N production, +M tests` notice on its check run for the lines it adds or removes. If it grows the code, say why in the PR body. Simple means no unnecessary abstractions, not cramming distinct jobs into one file: split a growing or mixed-job file into focused modules.
- **Say it once.** Each message, rule, check or format should live in one place. If you find the same thing in two places, merge them instead of adding a flag to keep them in sync.
- **No side doors.** Tests should use the same code path as the real program. If a test needs a shortcut, fix the test or the design.
- **Use the words people already know.** Good code reads the way people expect, so it needs no tutorial, like WASD in a game. Names follow [Naming](#naming) below.
- **Fix the cause.** When something fails, find out why. Don't paper over it with sleeps, retries or longer timeouts.
- **No plain comments.** Rust source has no `//` or `/* */` comments, except the `// repeats module: <reason>` marker from Naming rule 8. Doc comments (`///` and `//!`) say what an item does and what it guarantees its callers, never why a workaround exists or why odd behavior is fine. If code needs explaining, fix the code.
- **Scenario first.** Write the BDD scenario that describes what a player, GM or pack author sees, then the code. Scenarios test behavior, not how the code does it, so a refactor shouldn't break them.
- **Test it once.** Each behavior gets one test, at the layer where a player, GM or pack author sees it. [Testing](#testing) lists what that rules out.
- **Libraries are for others too.** `protocol` and `agent` exist so other clients and GMs can be written. That a crate has one consumer in this repository today is not a reason to fold it into that consumer.

## Testing

A test earns its place by failing when something a player, GM or pack author would notice changes. Check a new test against this list before you add it.

- **Extend, don't duplicate.** If a scenario already shows the behavior, extend or rename it instead of adding another. Don't add a wire-level scenario that repeats an end-to-end one, or the reverse. One scenario can cover several acceptance criteria, so you don't need one per item.
- **Don't guard the past.** Don't test that a removed flag, field or behavior stays gone, and don't re-run a fixed bug's old steps to prove it stays fixed. The scenario for the new behavior already fails if the old code comes back.
- **Leave other scenarios alone.** Don't add setup or checks to a scenario about something else just to re-test your change. Extend the scenario whose subject is your change.
- **Assert what people see.** Check output, refusals, the save or `spook verify`. Don't check journal keys, private fields, call counts or the order of internal steps when a visible outcome already shows the result. A refactor that keeps behavior should break no test.
- **Wait for events, not time.** A step waits for the reply, status or line of output it needs, never for a fixed time. Let time pass only when a timeout is what the scenario checks, and then write the check so it fails one way only. A timer never fires early, so check that a timeout happened by waiting well past it, and check that it hasn't happened yet with a timeout far longer than the scenario's waits. A scenario that checks two timers fired waits past the later one.
- **Unit tests only where no scenario can reach.** A parser edge case is a good one. A parser test for an argument that doesn't exist is not.
- **Name it for what the steps prove.** A scenario title says what its `Then` lines check. If they disagree, fix one of them.

## Logs and output

**Output** is what a command or seat is for: what `spook` shows a player, a GM, a pack author or the operator of `spook run` so they can play, run the game or fix a pack. Game text, refusals, the error a command exits with, the `spook check`, `status` and `verify` reports, the AI GM's `listening` and warning lines, and the host console are output. Output has fixed wording that tests check, and no log setting changes it.

A **log** is everything else: a `tracing` event that records what the program did, read only to find out why it behaved as it did. Turning logs off never costs a user information.

- **The test.** If `RUST_LOG=off` would hide something a user needs, it is output. A line a user can do nothing with except report it is a log.
- **Where logs go.** `spook run`, `spook ai-gm` and `spook play` in its window write their logs to `logs/<command>-<time>-<pid>.log` under the directory they were started in, and `spook run` prints that path on stderr once its pack is open. Every other command logs to stderr. Logs never go to stdout and never under a pack's `state/`.
- **Only the terminals print.** `spook`'s `main.rs` and the `spook play` and `spook gm` terminals (`client/terminal.rs`, `client/gm.rs`) write to stdout and stderr. Other crates hand their output to a writer or callback the caller passes in: `spookery_engine_net::Config::out` for the host console, and the AI GM's `report`. Output never goes through `tracing`, and a log never stands in for output.
- **Refusals and errors are output.** Logging one again at `warn` or `error` would report it twice. If it lacks its cause, such as the disk error behind `STORE_IO`, log the cause at `debug`.
- **Logs never copy the game record.** The tape holds what was said and the journal holds what each round did. A log names a round, seat or command by its id. It never carries speech, effects, dice, views or message bodies.
- **Logs never carry secrets or prompts.** No key, claim token, model prompt or model reply text, at any level.
- **No flag sets the log level.** `RUST_LOG` is the only setting, and `spook` reads it once. Spookery has no `-v` or `-q`. A command that someday shows more or less output decides that itself, and never reads or sets the log level.
- **Fields, not formatted text.** Write `tracing::info!` with its crate path and keep the default target, which is the module path. Fields are ids and codes (named by Naming rule 17, so `actor_id`, `conn_id` and `command_id`), counts, and `elapsed_ms`. A field that copies a wire or save name keeps that name, such as `world_time`.

| Level | When | For example |
| --- | --- | --- |
| `error` | A fault stopped part of the program, and no output says so. | The game port stopped serving. |
| `warn` | The program worked around a fault or runs degraded, and no output says so. A healthy run logs none. | A journal bundle left open, a torn journal record cut at startup, a model call retried. |
| `info` | One event per thing on an operator's timeline: a seat claimed or freed, a round committed, a GM turn, a model call. | `round committed round_index=4 state_rev=9 elapsed_ms=3` |
| `debug` | One event per command or reply, for a developer. Never one per frame, tick or loop pass. | A command received, a refusal sent, the cause behind an output code. |
| `trace` | Spookery logs nothing here. Dependencies do. | |

With `RUST_LOG` unset or empty, `spook run` and `spook ai-gm` write Spookery's `info` events and every crate's warnings and errors (`warn,spook=info`) to their file. Every other command writes only warnings and errors (`warn`) to stderr. `RUST_LOG` replaces that default. It takes `EnvFilter` directives, and a target matches by prefix, so `spook` names every Spookery crate and no dependency:

```sh
RUST_LOG=warn,spook=debug spook run pack
RUST_LOG=warn,spookery_client::http_model=info spook ai-gm
RUST_LOG=warn,spookery_engine=debug spook verify pack
RUST_LOG=off spook run pack
```

A `RUST_LOG` that names only targets turns every other target off, so start it with `warn,`. A bare `RUST_LOG=debug` also turns on `rmcp`, which logs every MCP request with its arguments. Tests start `spook` with the environment they run in, so while `RUST_LOG` is set, scenarios that check the stderr of a one-shot command fail. Narrow the run with `--name` first.

## Naming

Default to the [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/checklist.html) (casing, `as_`/`to_`/`into_` conversions, getters, constructors, typed units and the rest), with these exceptions and additions:

1. Choose names that let a human newcomer follow a file top to bottom from its names and types alone, and easily trace the code without already knowing its implementation.
   A short name is useful only when its meaning is clear where it appears. Review names from the caller's perspective, not the author's.
2. Types are nouns and functions are verbs. A getter can be a noun. A pair constructor that mirrors `mpsc::channel()`, like `frame_queue()`, can be a noun when its doc comment says so.
   Ask "a value of this type is a ___". If the answer is a noun, that noun is the name, so not `Evaluated` but `Verdict`. A past participle or adjective is a name only for an enum variant that is a state or past event of its enum's noun (`Cow::Borrowed`, `Bound::Excluded`, `ErrorKind::TimedOut`), for the type a same-named method returns (`Iterator::copied()` returns `Copied`), or under rule 15.
   A past participle or adjective is not a function name (`suffixed`, `parsed`), except an adapter that copies a std name, such as `cloned` or `copied`. So not `suffixed(path)` but `add_suffix(path)`.
3. Borrow words from Rust, std, the crates we use and this codebase before inventing new ones. If std says `try_lock`, say that.
4. No vague words. A name says what the thing shows, sends, reads or refuses.
5. Two names that look alike must do alike jobs.
6. Use game terms like tick and round only in the sense the [glossary](docs/README.md#glossary) gives them. Game time is ticks, and a batch of work is a round. A bare game-time number carries the suffix `_ticks`. Minutes and hours are real time, or the shown clock.
7. A type that carries messages between layers, such as a request, reply, frame, event or queue half, says in its doc comment which two layers it connects. Similar names get that comment, not a rename.
8. Don't repeat the module name inside a name. A name that is the module name, such as `query::Query`, is fine. CI checks the files a PR touches; a name kept on purpose, under rule 15 or as a pair like `ClientMessage`, ends its line with `// repeats module: <reason>`, or on the next line when rustfmt moves it there.
9. A comment shouldn't have to explain a name. If it does, fix the name.
10. Names collide only within one namespace, so `tokio::fs` and `std::fs` are fine. When a name exists in two namespaces anywhere in the project, a `use` keeps the path prefix (`fs::read`, never a bare `read`).
11. Fix a bad name in the code you touch, even if it is older than your change. A rename-only commit changes no logic and updates the docs that mention the name.
12. Prefer a typed unit such as `Duration`, which carries its own unit. A bare number carries the unit as a suffix, such as `_secs` or `_bytes`.
13. Name a module after the protocol or job it handles, never a metaphor. Say `mcp`, not `door`.
14. End a stream with `None` and report a failure with `Err`. Don't add `Eof` or `Unreadable` variants for them.
15. A name that mirrors a tool or wire name uses that exact name, such as `ServerBody::Queued` for `queued`.
16. Use one word per wire concept across docs, code and flags. The player side is the game port everywhere.
17. An identifier is `id` or ends in `_id`, whatever type backs it (a `String` today, a typed or interned id later). `name` is only ever text a player reads. Nothing else is called a name, and an id never stands in for a name or a name for an id. A map, set or lookup is keyed by id, never by name: code that starts from a name a player typed resolves it to an id first. An outside format's keys keep their spelling, such as the `name` that OpenAI and MCP tool calls use for a tool's id.
18. Every lifetime, single or one of several, has a name of at most 3 letters that says what it borrows, for example `'w` for the component store, `'s` for system state, `'c` for the canon, `'r` for the round, `'de` for deserializer input, and `'end` for an end. Generic `'a` and `'b` are for non-specific wrappers such as a collection or a plain borrow. Names that std or well-known crates already use keep their spelling even if they are longer (`'static`, and `'scope` and `'env` from `std::thread::scope`). Use `'se` only when a lifetime actually borrows a serializer; serde's `Serialize` has none. A single type parameter is `T`, or a usual letter (`K`/`V` for a map key and value, `E` for an error or an entity collection, `F` for a closure, `I` for an iterator, `C` for a component, `D`/`S`/`A` for serde, and Bevy's `Query` `D`/`F` for data and filter). Several type parameters, or one whose meaning is not obvious, get a short meaningful name (`Lock` for a pack lock, `Conn` for a connection, `Body`, `Id`, `Seen`).

Items in the guidelines that only matter for a published library, such as stability and future-proofing, don't apply yet.

## Live AI CI

The `Live AI` workflow runs the live round test (`crates/spook/tests/integration/live_round.rs`) against Ollama on pull requests that touch the model host. It also runs it against xAI weekly and on demand, never on pull requests, with the `XAI_API_KEY` repo secret passed in as `SPOOK_GM_KEY`. Without that secret the xAI job skips with a notice. The xAI model is `grok-4.20-0309-non-reasoning` unless the `SPOOK_XAI_MODEL` repo variable names another.
