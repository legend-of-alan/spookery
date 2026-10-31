# Contributing

Contributions are issues only for now. Open an issue, and pick the form that fits.

A pull request from outside this repository is closed with thanks.

Title letters, sizes, and labels are below, under Issues and PRs. Build and play steps are in [README.md](README.md).

A contributor license agreement (CLA) and a developer certificate of origin (DCO) are deferred.

Feature scenarios live under `crates/engine/tests/features` and `crates/spook/tests/features`. Two programs per crate run them:

```sh
cargo test --workspace --test features_host --test features_world
```

`rust-toolchain.toml` pins the Rust version, and rustup installs it for you. Pull requests only run CI on Linux. Pushes to main also run it on macOS and Windows.

## Issues and PRs

Issue and PR titles start with a letter for the area and end with a size.

- `R:` rules and rounds: round stages and triggers
- `W:` world model: places, exits, travel, items, pack rule schema
- `E:` engine plumbing: journal, save, replay, verify
- `H:` host and seats: server, seat protocol, CLI and agent clients
- `N:` naming
- `D:` docs
- `T:` tests, flaky-test fixes and CI

Sizes: `[S]` about one sitting, `[M]` a focused PR, `[L]` likely split into sub-issues.

Every issue gets a priority label from `P0` (most urgent) to `P4` (a parked idea). It also gets a type label, which is `enhancement`, `bug` or `documentation`. Phases are milestones.

## Design principles

- **Keep it simple.** The best fix is often deleting code, not adding it. Before you write something new, check whether you can remove, reuse or derive what you need from what's already there. Build only as much as the job needs. In your PR, say how many lines it adds or removes, and if it grows the code, say why.
- **Say it once.** Each message, rule, check or format should live in one place. If you find the same thing in two places, merge them instead of adding a flag to keep them in sync.
- **No side doors.** Tests should use the same code path as the real program. If a test needs a shortcut, fix the test or the design.
- **Use the words people already know.** Good code reads the way people expect, so it needs no tutorial, like WASD in a game. Names follow [Naming](#naming) below.
- **Fix the cause.** When something fails, find out why. Don't paper over it with sleeps, retries or longer timeouts.
- **Scenario first.** Write the BDD scenario that describes what a player or GM sees, then the code. Scenarios test behavior, not how the code does it, so a refactor shouldn't break them.

## Naming

Default to the [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/checklist.html) (casing, `as_`/`to_`/`into_` conversions, getters, constructors, typed units and the rest), with these exceptions and additions:

1. A newcomer should follow a file top to bottom from its names and types alone.
2. Types are nouns and functions are verbs. A getter can be a noun. A pair constructor that mirrors `mpsc::channel()`, like `frame_queue()`, can be a noun when its doc comment says so.
3. Borrow words from Rust, std, the crates we use and this codebase before inventing new ones. If std says `try_lock`, say that.
4. No vague words. A name says what the thing shows, sends, reads or refuses.
5. Two names that look alike must do alike jobs.
6. Use game terms like tick and round only in the sense the [glossary](docs/README.md#glossary) gives them. A tick is game time, and a batch of work is a round.
7. A type that carries messages between layers, such as a request, reply, frame, event or queue half, says in its doc comment which two layers it connects. Similar names get that comment, not a rename.
8. Don't repeat the module name inside a name (Clippy's `module_name_repetitions`).
9. A comment shouldn't have to explain a name. If it does, fix the name.
10. Names collide only within one namespace, so `tokio::fs` and `std::fs` are fine. When a name exists in two namespaces anywhere in the project, a `use` keeps the path prefix (`fs::read`, never a bare `read`).
11. Fix a bad name in the code you touch, even if it is older than your change. A rename-only commit changes no logic and updates the docs that mention the name.
12. Prefer a typed unit such as `Duration`, which carries its own unit. A bare number carries the unit as a suffix, such as `_secs` or `_bytes`.

Items in the guidelines that only matter for a published library, such as stability and future-proofing, don't apply yet.

## Prove the log

Each commit that changes the save appends one journal record under `state/journal`, and the first commit stores a replay base there. `spook verify` replays that journal and checks the save byte for byte. It does not write the pack, take the lock, or bind a port. A later commit from a running server is ignored, because verify reads the save once and stops at that counter.

```sh
cp -r packs/oneshot-docks /tmp/docks
spook round /tmp/docks
spook verify /tmp/docks
```

`verify` prints `verified <n> records, state_rev <r>` when the save matches. A fresh copy has never committed, so verify prints `verified 0 records, state_rev 0`.

## Live AI CI

The `Live AI` workflow runs the live round test (`crates/spook/tests/integration/live_round.rs`) against Ollama on pull requests that touch the model host. It also runs it against xAI weekly and on demand, never on pull requests, with the `XAI_API_KEY` repo secret passed in as `SPOOK_GM_KEY`. Without that secret the xAI job skips with a notice. The xAI model is `grok-4.20-0309-non-reasoning` unless the `SPOOK_XAI_MODEL` repo variable names another.

## Coding agent clients

`spook run` serves one agent door at `/mcp` on the game port plus one. The default is `http://127.0.0.1:9738/mcp`. `--door 127.0.0.1:<port>` overrides it, and `--door 127.0.0.1:0` lets the operating system choose. `--addr 127.0.0.1:0` does the same for the game port and chooses the door on its own. The process prints the bound addresses once, as one JSON line on stdout, before it serves. Point the agent at `127.0.0.1`, not `localhost`. The endpoint checks the `Host` header, and `http://localhost:9738/mcp` gets a 403.

```sh
cp -r packs/oneshot-docks /tmp/docks
spook run /tmp/docks
```

Register `http://127.0.0.1:9738/mcp`. The key names below are the ones each client documents for streamable HTTP.

Claude Code:

```sh
claude mcp add --transport http spook http://127.0.0.1:9738/mcp
```

Or in `.mcp.json`:

```json
{
  "mcpServers": {
    "spook": {
      "type": "http",
      "url": "http://127.0.0.1:9738/mcp"
    }
  }
}
```

Grok Build, in `~/.grok/config.toml`. The key is `url` under `[mcp_servers]`, not a `mcpServers` JSON file.

```toml
[mcp_servers.spook]
url = "http://127.0.0.1:9738/mcp"
```

Cursor, in `.cursor/mcp.json`:

```json
{
  "mcpServers": {
    "spook": {
      "url": "http://127.0.0.1:9738/mcp"
    }
  }
}
```

Gemini CLI, in `~/.gemini/settings.json`. `httpUrl` is the streamable HTTP key. `url` is SSE.

```json
{
  "mcpServers": {
    "spook": {
      "httpUrl": "http://127.0.0.1:9738/mcp"
    }
  }
}
```

Windsurf, in `~/.codeium/windsurf/mcp_config.json`. The key is `serverUrl`.

```json
{
  "mcpServers": {
    "spook": {
      "serverUrl": "http://127.0.0.1:9738/mcp"
    }
  }
}
```

Claude Desktop's config file only takes stdio. `mcp-remote` reaches the HTTP door:

```json
{
  "mcpServers": {
    "spook": {
      "command": "npx",
      "args": ["mcp-remote", "http://127.0.0.1:9738/mcp"]
    }
  }
}
```

The tools are `table_status`, `perceive`, `queue_intent`, `say`, `commit_round`, and `close`. A client may ignore `spook/event` notifications. `table_status` with `since` returns the speech it missed. `close` answers after the GM seat is free.
