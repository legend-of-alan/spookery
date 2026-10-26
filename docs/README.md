# Docs

[Engine guarantees](engine-guarantees.md) states what the engine promises. The feature files test them. Where the doc and the feature files disagree, the feature files win.

[Write your first pack](first-pack.md) walks through `spook new` and the first edits. [Pack reference](pack-reference.md) is the field reference for authors and agents. [Expressions](spec.md) is the number language those fields share. [Seat protocol](seat-protocol.md) is the wire for a player client or a GM integration.

[At the table](#at-the-table) covers the host flags. [Game master](#game-master) covers the empty seat, the model, and a human GM. [Privacy](#privacy) is what `spook ai-gm` sends. The [license](#license) split is at the bottom. Build and play are in the [README](../README.md#start).

Do not restore `docs/legacy_architecture/` or `docs/adr/`.

## Principles

1. **Text is the source of truth.** If people and models can't diff it, it doesn't belong in the product.
2. **Numbers live in code.** Dice and outcomes come from the engine. Models narrate after the tools return.
3. **Idle worlds are boring on purpose.** Shops keep living while the table is away, but the plot doesn't finish itself.
4. **Clients are swappable.** A terminal, Discord, voice or graphics client is a shell, not a second product.
5. **The engine proves the format.** A feature that can't be written as a pack field isn't a feature.

## How this differs

The model does not keep the world. It can talk, and it can ask the engine to commit a round. Dice, items, and places change only inside that commit. The journal records the round. Replay repeats the recorded rolls. `spook verify` checks the journal. A player sees a hidden fact only when that character knows it.

## Glossary

| Term | Meaning |
|---|---|
| **Spookery** | The project name. The library is `spookery`; the package and binary are `spook`. |
| **Pack** | A folder of text that is the world: canon, rules and current state. |
| **Seat** | One place at the table. Each player character is a seat, and the GM is one more. |
| **Canon** | Files under `world/`, `cast/` and `rules/` in a pack. They don't change while the world runs. |
| **`spook`** | The program that loads a pack. It is the only thing that writes the pack's state. |
| **Kernel** | The code that decides what is legal, rolls checks and tracks who knows what. |
| **Tick** | A unit of game time on the world clock. Talking does not take a tick; moving and acting do. |
| **Round** | One batch in which queued intents are settled together and the results are logged. |
| **Intent** | A proposed move from a player, an NPC or the engine's scheduler. |
| **Effect** | A checked change to the world, produced from an intent. |
| **Expression** | A number in a rule. An integer, dice such as `2d6` or `2d20kh1`, a meter read such as `self.might`, and `+`, `-`, `*`, `/`, `min`, and `max`. A condition's value does not roll dice. |
| **GM** | The storyteller seat. Usually an agent that talks to the agent door; `spook ai-gm` is the built-in model GM. |
| **Play door** | The human client, `spook play`, over loopback. |
| **Agent door** | The MCP endpoint `spook run` serves, `http://127.0.0.1:9738/mcp` by default. |

## Repo layout

```text
README.md              how to build and play
docs/                  guarantees, first-pack guide, pack reference, expressions, seat protocol, this file
packs/                 sample-d20 is the starter. oneshot-docks is the multi-PC fixture
.grok/skills/          the author-pack skill
crates/
  protocol/            spookery-protocol: player and GM wire codecs
  engine/              spookery-engine: world model, rules, rounds, and journal
  agent/               spookery-agent: model GM turn state machine
  cli/                 spookery-cli: spook play, spook gm, and spook ai-gm
  server/              spookery-server: world loop, seats, idle clock, and MCP door
  spook/               package and binary spook, library spookery
    tests/features/    BDD scenarios
```

## At the table

`--addr` sets the loopback address. The default is `127.0.0.1:9737`. Passing `127.0.0.1` alone means that same port. Any other host is refused before the pack opens. The agent door is the next port, `127.0.0.1:9738`.

`--as <id>` sits that character. `pc_rowan` and `player:pc_rowan` are the same seat. A second client for a character who is already sitting exits 1 and prints `That character is already taken. [SEAT_TAKEN]`. A fourth player exits 1 and prints `The table is full. [SESSION_FULL]`.

`spook status` prints the host digest and does not write the pack. `spook tick` and `spook migrate` write `state/`. Stop `spook run` before those two, because the server keeps the save in memory.

## Game master

### No GM

Leave the GM seat empty. Speech is stored on the tape and does not advance time. Each player move is committed as its own tick, and a move that leaves every seated player busy runs on to the arrival. There is no narration and no model call.

### Model GM

`spook ai-gm` sits the GM seat and calls a chat-completions endpoint. It does not start the server or the player. Start `spook run` first. The client posts to `{SPOOK_GM_URL}/chat/completions` with one tool, `act`. The model answers with that tool. The engine then commits the queued lines.

Three environment variables, and no others. All three are required. There is no default URL.

| Variable | Required | Meaning |
| --- | --- | --- |
| `SPOOK_GM_KEY` | yes | Bearer token. Read from the environment, never from the pack. |
| `SPOOK_GM_URL` | yes | Origin of the endpoint, including the `/v1` prefix the provider documents. |
| `SPOOK_GM_MODEL` | yes | Model name sent in the request. |

A missing variable is printed on stderr as `<NAME> is unset` and the process exits 1 before it dials. The variables are checked in the order `SPOOK_GM_KEY`, `SPOOK_GM_URL`, `SPOOK_GM_MODEL`.

xAI:

```sh
SPOOK_GM_KEY=your-key SPOOK_GM_URL=https://api.x.ai/v1 SPOOK_GM_MODEL=grok-4.20-0309-non-reasoning spook ai-gm
```

Claude, through the chat-completions endpoint:

```sh
SPOOK_GM_KEY=your-key SPOOK_GM_URL=https://api.anthropic.com/v1 SPOOK_GM_MODEL=claude-sonnet-4-5 spook ai-gm
```

Set `SPOOK_GM_MODEL` to the Claude model you want. [Ollama](https://ollama.com), running locally:

```sh
ollama pull qwen2.5:0.5b
SPOOK_GM_KEY=ollama SPOOK_GM_URL=http://127.0.0.1:11434/v1 SPOOK_GM_MODEL=qwen2.5:0.5b spook ai-gm
```

When the provider refuses a call, stderr shows `model call returned <status>: <text>`. The text is the provider's error body on one line, cut to 200 characters, with the key replaced by `[key]`.

`crates/spook/tests/integration/live_round.rs` plays one real round against whatever provider those three variables name. It is ignored by default. CI runs it too; see [CONTRIBUTING.md](../CONTRIBUTING.md#live-ai-ci).

### Human GM

`spook gm` sits the same GM seat as `spook ai-gm`. Start `spook run` first. Both dial the agent door. `--mcp-http 127.0.0.1:<port>` dials that door instead. `spook run --addr 127.0.0.1:0` lets the operating system choose the ports, then prints the listening line. Pass that door to `--mcp-http`.

```sh
spook gm
```

Type `help` for the seat commands. `spook gm --raw` prints each protocol frame as one JSON line.

## Privacy

`spook ai-gm` sends table text to the host named by `SPOOK_GM_URL`. There is no default host. The API key is the bearer token from `SPOOK_GM_KEY` in the environment. The pack does not hold the key.

The request is one chat-completions POST. The JSON body carries one user message, the model name, `"stream": false`, and one function tool named `act`. The message content is a bounded JSON prompt: the last 16 says this process has handled, the new player lines, the move menu, the queued intents, and stripped views. A follow-up after a roll adds the roll faces. A player view that is only a room census drops known facts, messages, and unknown mentions. An NPC view keeps each fact's id and text. Every fact body is dropped. The prompt includes the Markdown body of the current place and of each cast member in the views. Each of those bodies is cut at 800 characters. It does not include `save.yaml` or the tape file. Those stay on disk under the pack. Starting `spook ai-gm` again remembers nothing from the previous process.

## License

The engine is open source (AGPL-3.0). Runtime code, the tests of that code, and repository tooling and scripts are AGPL-3.0-or-later. The exception is the `spookery-protocol` crate, which is MIT, so a game client can reuse the wire codec freely. Documentation, schemas, and packs are CC BY 4.0.

The terms are in [LICENSE](../LICENSE), [LICENSE-MIT](../LICENSE-MIT) and [LICENSE-CC-BY-4.0](../LICENSE-CC-BY-4.0).
