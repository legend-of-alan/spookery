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

```sh
cp -r packs/oneshot-docks /tmp/docks
export PATH="$PWD/target/release:$PATH"
```

Run that `export` in each terminal. `spook --help` lists the commands and the flags. `spook check /tmp/docks` prints any pack problem in plain English and writes nothing.

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

Stock:
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

The first word is the verb. `say` plus the words after it is speech at the table, and it does not tick. `travel` takes one place id. `travel docks_street` leaves the shop for the street, and the client prints `You are in Docks street.` `wait` and `grit` take no further words. `grit` is the pack's check, 2d6 plus the grit meter. The client prints `Your roll succeeded.`, `Your roll partly succeeded.`, or `Your roll missed.`

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

Each commit that changes the save appends one journal record under `state/log`, and the first commit stores a replay base there. `spook replay` replays that journal and prints the final save counter. `spook verify` replays it and checks the save byte for byte. Neither command writes the pack, takes the lock, or binds a port. A later commit from a running server is ignored, because verify reads the save once and stops at that counter.

```sh
spook tick /tmp/docks
spook replay /tmp/docks
spook verify /tmp/docks
```

`replay` prints `state_rev <r>, <n> records`. `verify` prints `verified <n> records, state_rev <r>` when the save matches. A fresh copy has never committed, so verify prints `verified 0 records, state_rev 0`.

## Game master

### No GM

Leave the GM seat empty. Speech is stored on the tape and does not advance time. Each player move is committed as its own tick, and world numbers change only inside that tick. There is no narration and no model call.

With both seats empty, the server still wakes on the pack's real-time interval. In the docks pack that interval is `tick_every_real_seconds: 300`. While a player is sitting, that wake does not tick. A pack sets `idle: off` in `rules/tick.yaml` to leave that empty table still.

### Model GM

`spook ai-gm` sits the GM seat and calls a chat-completions endpoint. It does not start the server or the player. Start `spook run` first.

Three environment variables, and no others:

| Variable | Required | Meaning |
| --- | --- | --- |
| `XAI_API_KEY` | yes | Bearer token. Read from the environment, never from the pack. |
| `SPOOK_GM_MODEL` | yes | Model name sent in the request. |
| `SPOOK_GM_URL` | no | Origin of the endpoint. The default is `https://api.x.ai/v1`. Any OpenAI-compatible chat completions URL works. The client posts to `{url}/chat/completions`. |

A missing `XAI_API_KEY` or `SPOOK_GM_MODEL` is printed on stderr and the process exits 1 before it dials. Then:

```sh
spook ai-gm /tmp/docks
```

### Human GM

`spook gm` sits the same GM seat as `spook ai-gm`. It does not start the server. Start `spook run` first.

```sh
spook gm /tmp/docks
```

Type `help` for the seat commands. `spook gm --raw` prints each socket frame as one JSON line.

### Coding agent

`spook mcp` is the stdio door. It does not start the server and it does not take a pack. Start `spook run` first. Grok Build, Claude Code, and Cursor each register that command as an MCP server:

```sh
spook mcp --addr 127.0.0.1:9737
```

The same seat is also at `http://127.0.0.1:<port>/mcp` when the server is started with `--mcp-http 127.0.0.1:<port>`. That port is not the game port. Both stay on loopback. Point your agent at `127.0.0.1`, not `localhost`. The endpoint checks the `Host` header, and `http://localhost:<port>/mcp` gets a 403.

The tools are `session_status`, `perceive`, `intent`, `say`, and `commit_turn`. A client may ignore `spook/event` notifications. `session_status` with `since` returns the speech it missed.

## Privacy

`spook ai-gm` sends table text to the host in `SPOOK_GM_URL`. When that variable is unset, the host is `https://api.x.ai/v1`. The API key is the bearer token from `XAI_API_KEY` in the environment. The pack does not hold the key.

The request is one chat-completions POST. The JSON body carries one user message, the model name, `"stream": false`, and one function tool named `act`. The message content is a bounded JSON prompt: the last 16 says this process has handled, the new player lines, the move menu, the queued intents, and stripped views. A follow-up after a roll adds the roll faces. A player view that is only a room census drops known facts, messages, and unknown mentions. An NPC view keeps each fact's id and text. Every fact body is dropped. The prompt does not include a cast body, `now.yaml`, or the tape file. Those stay on disk under the pack. Starting `spook ai-gm` again remembers nothing from the previous process.

## Docs

- [docs/spec.md](docs/spec.md) is the normative spec. A rule it does not state is unspecified.
- [docs/product_spec.md](docs/product_spec.md) is product notes. Where it disagrees with the spec, the spec wins.
- [LICENSE](LICENSE)

## License

The engine is open source (AGPL-3.0). Runtime code, the tests of that code, and repository tooling and scripts are AGPL-3.0-or-later. Documentation, schemas, and packs are CC BY 4.0.

The terms are in [LICENSE](LICENSE) and [LICENSE-CC-BY-4.0](LICENSE-CC-BY-4.0).
