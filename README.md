# Spookery

Spookery is a generic RPG referee engine. An author writes a pack, a folder of YAML rules plus Markdown for places and people, and hosts it with `spook run` the way you would host a private server. One GM, a person or a model, and up to three players walk around and take actions. The engine is the only writer of world state. Dice are seeded, and a replay repeats the recorded rolls. The crate version is `0.0.0`, there are no releases, and this tree is pre-v0.1.

## Build from source

Install Rust 1.89 or newer with [rustup](https://rustup.rs). The workspace `rust-version` is 1.89. CI pins 1.89.0.

From the repository root:

```sh
cargo build --release
```

The binary is `target/release/spook`. On Windows it is `target/release/spook.exe`.

Linux needs `pkg-config` and `libssl-dev`, or your distribution's equivalents. The model client uses reqwest with native TLS. macOS needs the Xcode command line tools. Windows needs the MSVC C++ build tools. On Windows, check the repository out with LF line endings (`core.autocrlf` false). A CRLF checkout rewrites the pack, and `spook run` then exits 1 with `PACK_REV_MISMATCH`. CI sets `core.autocrlf` false before checkout.

CI builds this workspace on `ubuntu-latest`, `macos-latest`, and `windows-latest` with Rust 1.89.0. That workflow does not install extra packages. The GitHub Ubuntu image already includes `pkg-config` and `libssl-dev`. A minimal Linux install does not.

## Quick start

The docks pack is `packs/oneshot-docks`. Copy it first. `spook run` writes the save into the pack you pass (`state/`: the snapshot, the tape, and lock files). A copy leaves the fixture in the repo alone.

The fight pack is `packs/sample-d20`. Copy it before you host it, the same way as the docks pack.
`spook play` sits `pc_lark`. Type `strike npc_briar` to attack, `drink` while you carry a tonic, `travel mill_lane` to leave the yard, and `wait` to pass the turn.

`spook new` writes a smaller pack. The walkthrough is [docs/first-pack.md](docs/first-pack.md).

```sh
cp -r packs/oneshot-docks /tmp/docks
spook new /tmp/mypack
export PATH="$PWD/target/release:$PATH"
```

Run that `export` in each terminal. `spook --help` lists the commands and the flags. `spook check /tmp/docks` prints any pack problem in plain English and writes nothing. `spook check /tmp/mypack` prints `no problems`. `spook play /tmp/mypack` sits `pc_ada` and prints `You are in Yard.` `travel lane` prints `You are in Lane.`

Terminal 1, from the repository root:

```sh
spook run /tmp/docks
```

The process stays in the foreground and listens on `127.0.0.1:9737`. It prints nothing while it waits. When the player sits, and again when a tick commits, it prints the host digest. Type `stop` to shut it down.

Terminal 2:

```sh
spook play /tmp/docks
```

You sit the pack's start character, `pc_rowan`. `--as player:<id>` sits that character instead. The docks pack has three: `pc_rowan` in the shop, and `pc_harbormouse` and `pc_wren` on the street.

```sh
spook play /tmp/docks --as player:pc_harbormouse
spook play /tmp/docks --as player:pc_wren
```

A second client for a character who is already sitting exits 1 and prints `That character is already taken. [SEAT_TAKEN]`. A fourth player exits 1 and prints `The table is full. [SESSION_FULL]`.

The client prints readable text. This is the opening of a session, then one line of speech:

```text
While you were away:
Assassination debt: 2/6, paused
You are in Docks shop.

Here: Aldric, You

You know:
Oil is on the shelf.

Clocks:
Assassination debt: 2/6, paused

Items:
Lamp oil: 4
You: The lamp is out.
```

Lines to type:

```text
say The lamp is out.
travel docks_street
wait
grit
```

The first word is the verb. `say` plus the words after it is speech at the table, and it does not tick. `travel` takes one exit name. `travel docks_street` leaves the shop for the street, and the client prints `You are in Docks street.` `wait` and `grit` take no further words. `grit` is the pack's check, 2d6 plus the grit meter. The client prints `Your roll succeeded.`, `Your roll partly succeeded.`, or `Your roll missed.`

With no GM seated, `travel`, `wait`, and `grit` each commit one tick by themselves. Nothing narrates, and the line is not held for someone else to commit. Each seated player does that on their own line. With a GM seated, the lines wait, and one commit plays them in the order they were queued.

`--addr` sets the loopback address for every command. The default is `127.0.0.1:9737`. Passing `127.0.0.1` alone means that same port. Any other host is refused before the pack opens.

`status` prints the host digest and does not write the pack. `tick` and `migrate` write `state/`. Stop `spook run` before those two, because the server keeps the save in memory.

```sh
spook status /tmp/docks
spook tick /tmp/docks
spook migrate /tmp/docks
```

`tick` advances the pack one tick and prints nothing when it succeeds. `migrate` stores the current canon hash in the save and bumps the save counter. It also prints nothing when it succeeds.

## Prove the log

Each commit that changes the save appends one journal record under `state/journal`, and the first commit stores a replay base there. `spook replay` replays that journal and prints the final save counter. `spook verify` replays it and checks the save byte for byte. Neither command writes the pack, takes the lock, or binds a port. A later commit from a running server is ignored, because verify reads the save once and stops at that counter.

```sh
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

## Game master

### No GM

Leave the GM seat empty. Speech is stored on the tape and does not advance time. Each player move is committed as its own tick, and world numbers change only inside that tick. There is no narration and no model call.

With both seats empty, the server still wakes on the pack's real-time interval. In the docks pack that interval is `tick_every_real_seconds: 300`. While a player is sitting, that wake does not tick. A pack sets `idle: off` in `rules/tick.yaml` to leave that empty table still.

### Model GM

`spook ai-gm` sits the GM seat and calls a chat-completions endpoint. It does not start the server or the player. Start `spook run` first.

Three environment variables, and no others. All three are required. There is no default URL.

| Variable | Required | Meaning |
| --- | --- | --- |
| `SPOOK_GM_KEY` | yes | Bearer token. Read from the environment, never from the pack. |
| `SPOOK_GM_URL` | yes | Origin of the endpoint. Any OpenAI-compatible chat completions URL works. The client posts to `{url}/chat/completions`. |
| `SPOOK_GM_MODEL` | yes | Model name sent in the request. |

A missing variable is printed on stderr as `<NAME> is unset` and the process exits 1 before it dials. The variables are checked in the order `SPOOK_GM_KEY`, `SPOOK_GM_URL`, `SPOOK_GM_MODEL`. Then:

```sh
spook ai-gm /tmp/docks
```

When the provider refuses a call, stderr shows `model call returned <status>: <text>`. The text is the provider's error body on one line, cut to 200 characters, with the key replaced by `[key]`.

`crates/spook/tests/live_round.rs` plays one real round against whatever provider those three variables name: it hosts a copy of the docks pack, sits `spook ai-gm`, queues one player move, and passes when every model call returns 2xx and the move commits a tick. It is ignored by default. With [Ollama](https://ollama.com) running locally:

```sh
ollama pull qwen2.5:0.5b
SPOOK_GM_KEY=ollama SPOOK_GM_URL=http://127.0.0.1:11434/v1 SPOOK_GM_MODEL=qwen2.5:0.5b \
  cargo test -p spook --test live_round --locked -- --ignored --nocapture
```

The `Live AI` workflow runs it against Ollama on pull requests that touch the model host. It also runs it against xAI weekly and on demand, never on pull requests, with the `XAI_API_KEY` repo secret passed in as `SPOOK_GM_KEY`. Without that secret the xAI job skips with a notice. The xAI model is `grok-4.20-0309-non-reasoning` unless the `SPOOK_XAI_MODEL` repo variable names another.

### Human GM

`spook gm` sits the same GM seat as `spook ai-gm`. It does not start the server. Start `spook run` first. Both dial the agent door on the next port, `http://127.0.0.1:9738/mcp` when the game is on `9737`. `--mcp-http 127.0.0.1:<port>` dials that door instead. `spook run --addr 127.0.0.1:0` lets the operating system choose the game port and a separate door port, then prints `{"kind":"listening","game":"127.0.0.1:<game>","door":"127.0.0.1:<door>"}`. Pass that door to `--mcp-http`. A fixed game port still uses the next port.

```sh
spook gm /tmp/docks
```

Type `help` for the seat commands. `spook gm --raw` prints each protocol frame as one JSON line.

### Coding agent

`spook run` serves one agent door at `/mcp` on the game port plus one. The default is `http://127.0.0.1:9738/mcp`. `--mcp-http 127.0.0.1:<port>` overrides it, and `--mcp-http 127.0.0.1:0` lets the operating system choose. `--addr 127.0.0.1:0` does the same for the game port and chooses the door on its own. The process prints the bound addresses once, as one JSON line on stdout, before it serves. Point the agent at `127.0.0.1`, not `localhost`. The endpoint checks the `Host` header, and `http://localhost:9738/mcp` gets a 403.

```sh
spook run packs/oneshot-docks
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

## Privacy

`spook ai-gm` sends table text to the host named by `SPOOK_GM_URL`. There is no default host. The API key is the bearer token from `SPOOK_GM_KEY` in the environment. The pack does not hold the key.

The request is one chat-completions POST. The JSON body carries one user message, the model name, `"stream": false`, and one function tool named `act`. The message content is a bounded JSON prompt: the last 16 says this process has handled, the new player lines, the move menu, the queued intents, and stripped views. A follow-up after a roll adds the roll faces. A player view that is only a room census drops known facts, messages, and unknown mentions. An NPC view keeps each fact's id and text. Every fact body is dropped. The prompt includes the Markdown body of the current place and of each cast member in the views. Each of those bodies is cut at 800 characters, and a body that repeats a hidden fact is left out. It does not include `save.yaml` or the tape file. Those stay on disk under the pack. Starting `spook ai-gm` again remembers nothing from the previous process.

## Docs

- [docs/spec.md](docs/spec.md) is the normative spec. A rule it does not state is unspecified.
- [docs/README.md](docs/README.md) has the principles, glossary and repo layout.
- [LICENSE](LICENSE)

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

## License

The engine is open source (AGPL-3.0). Runtime code, the tests of that code, and repository tooling and scripts are AGPL-3.0-or-later. Documentation, schemas, and packs are CC BY 4.0.

The terms are in [LICENSE](LICENSE) and [LICENSE-CC-BY-4.0](LICENSE-CC-BY-4.0).
