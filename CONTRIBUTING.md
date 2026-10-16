# Contributing

Contributions are issues only for now. Open an issue, and pick the form that fits.

A pull request from outside this repository is closed with thanks.

Title letters, sizes, and labels are below, under Issues and PRs. Build and play steps are in [README.md](README.md).

A contributor license agreement (CLA) and a developer certificate of origin (DCO) are deferred.

Feature scenarios live under `crates/spook/tests/features`. Two programs run them:

```sh
cargo test -p spook --test features_host --test features_world
```

## Issues and PRs

Issue and PR titles start with a letter for the area and end with a size.

- `R:` rounds and rules: the round stages and triggers
- `W:` world model: places, exits, travel, items, pack rule schema
- `K:` kernel and core plumbing: journal, host, transport, CI
- `N:` naming: renames that make a name say what it does
- `D:` docs
- `T:` tests and flaky-test fixes

Sizes: `[S]` about one sitting, `[M]` a focused PR, `[L]` likely split into sub-issues.

Every issue gets a priority label (`P0` highest to `P3`) and a type label (`enhancement`, `bug`, `documentation`). Phases are milestones.

## Prove the log

Each commit that changes the save appends one journal record under `state/journal`, and the first commit stores a replay base there. `spook replay` replays that journal and prints the final save counter. `spook verify` replays it and checks the save byte for byte. Neither command writes the pack, takes the lock, or binds a port. A later commit from a running server is ignored, because verify reads the save once and stops at that counter.

```sh
cp -r packs/oneshot-docks /tmp/docks
spook tick /tmp/docks
spook replay /tmp/docks
spook verify /tmp/docks
```

`replay` prints `state_rev <r>, <n> records`. `verify` prints `verified <n> records, state_rev <r>` when the save matches. A fresh copy has never committed, so verify prints `verified 0 records, state_rev 0`.

`spook audit` replays that same journal and checks each player character's view for a hidden fact. A clean table prints `pass` and the character id, one line each, and exits 0. Speech that repeats a hidden fact is a note and does not fail the command. `--strict` makes those notes fail.

```sh
spook audit /tmp/docks
spook audit --strict /tmp/docks
```

## Live AI CI

The `Live AI` workflow runs it against Ollama on pull requests that touch the model host. It also runs it against xAI weekly and on demand, never on pull requests, with the `XAI_API_KEY` repo secret passed in as `SPOOK_GM_KEY`. Without that secret the xAI job skips with a notice. The xAI model is `grok-4.20-0309-non-reasoning` unless the `SPOOK_XAI_MODEL` repo variable names another.

## Coding agent clients

`spook run` serves one agent door at `/mcp` on the game port plus one. The default is `http://127.0.0.1:9738/mcp`. `--mcp-http 127.0.0.1:<port>` overrides it, and `--mcp-http 127.0.0.1:0` lets the operating system choose. `--addr 127.0.0.1:0` does the same for the game port and chooses the door on its own. The process prints the bound addresses once, as one JSON line on stdout, before it serves. Point the agent at `127.0.0.1`, not `localhost`. The endpoint checks the `Host` header, and `http://localhost:9738/mcp` gets a 403.

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

The tools are `session_status`, `perceive`, `intent`, `say`, and `commit_round`. A client may ignore `spook/event` notifications. `session_status` with `since` returns the speech it missed.
